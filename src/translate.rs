use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;

use crate::config::Config;

static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn get_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(300))
            .connect_timeout(Duration::from_secs(10))
            .pool_max_idle_per_host(2)
            .build()
            .expect("Failed to create HTTP client")
    })
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Serialize)]
struct Message {
    role: String,
    content: MessageContent,
}

#[derive(Serialize)]
#[serde(untagged)]
enum MessageContent {
    Text(String),
    Multimodal(Vec<ContentPart>),
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum ContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrl },
}

#[derive(Serialize)]
struct ImageUrl {
    url: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ResponseMessage,
}

#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

/// Use vision model to recognize text from image
pub async fn recognize_text(config: &Config, image_base64: &str) -> Result<String, String> {
    let prompt = r#"请仔细识别这张图片中的所有文本内容。

要求：
1. 完整识别所有文字，不要遗漏
2. 保持原文的段落结构和换行
3. 保持原文的列表格式（如 1. 2. 或 - 等）
4. 保持标题和正文的区分
5. 如果有页码、页眉页脚也要识别
6. 只输出识别到的文本，不要添加任何解释

请开始识别："#;

    let request = ChatRequest {
        model: &config.ocr_model,
        messages: vec![Message {
            role: "user".to_string(),
            content: MessageContent::Multimodal(vec![
                ContentPart::Text { text: prompt.to_string() },
                ContentPart::ImageUrl {
                    image_url: ImageUrl {
                        url: format!("data:image/jpeg;base64,{}", image_base64),
                    },
                },
            ]),
        }],
        max_tokens: Some(8192),
    };

    call_api(config, &request).await
}

/// Use translation model to translate text to Chinese
pub async fn translate_text(config: &Config, text: &str) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    
    // If already mostly Chinese, skip translation
    let chinese_ratio = count_chinese_chars(trimmed) as f32 / trimmed.chars().count().max(1) as f32;
    if chinese_ratio > 0.7 {
        return Ok(text.to_string());
    }
    
    let prompt = format!(
r#"你是一个专业的多语言翻译专家。请将以下内容翻译成简体中文。

翻译要求：
1. 翻译准确、流畅、符合中文表达习惯
2. 可以自由调整段落和换行，使译文更易读
3. 专有名词、品牌名、人名可保留原文或音译
4. 技术术语使用常见的中文译法
5. 只输出翻译结果，不要添加任何解释

原文内容：
{}"#, trimmed);

    let request = ChatRequest {
        model: &config.translate_model,
        messages: vec![Message {
            role: "user".to_string(),
            content: MessageContent::Text(prompt),
        }],
        max_tokens: Some(8192),
    };

    call_api(config, &request).await
}

async fn call_api(config: &Config, request: &ChatRequest<'_>) -> Result<String, String> {
    let url = format!("{}/v1/chat/completions", config.base_url.trim_end_matches('/'));
    
    let response = get_client()
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .json(request)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;
    
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    
    if !status.is_success() {
        return Err(format!("API 错误 {}: {}", status, body));
    }
    
    let chat_response: ChatResponse = serde_json::from_str(&body)
        .map_err(|e| format!("解析失败: {} - 响应: {}", e, &body[..body.len().min(500)]))?;
    
    chat_response
        .choices
        .first()
        .map(|c| c.message.content.clone())
        .ok_or_else(|| "空响应".to_string())
}

fn count_chinese_chars(text: &str) -> usize {
    text.chars()
        .filter(|c| {
            let code = *c as u32;
            (0x4E00..=0x9FFF).contains(&code) ||
            (0x3400..=0x4DBF).contains(&code) ||
            (0x20000..=0x2A6DF).contains(&code)
        })
        .count()
}
