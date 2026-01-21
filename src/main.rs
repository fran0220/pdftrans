mod config;
mod pdf;
mod translate;
mod state;

use axum::{
    Router,
    extract::{Multipart, Path, State},
    response::{Html, IntoResponse, Response, Sse},
    routing::{get, post},
    http::{header, StatusCode},
    body::Body,
};
use std::sync::Arc;
use tower_http::cors::CorsLayer;

use crate::state::AppState;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let config = config::Config::from_env();
    println!("PDF Translator V2 starting...");
    println!("API Base URL: {}", config.base_url);
    println!("OCR Model: {}", config.ocr_model);
    println!("Translate Model: {}", config.translate_model);
    
    let state = Arc::new(AppState::new(config));
    
    let app = Router::new()
        .route("/", get(index))
        .route("/upload", post(upload))
        .route("/progress/{task_id}", get(progress))
        .route("/download/{task_id}", get(download))
        .layer(CorsLayer::very_permissive())
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".to_string());
    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    println!("Server running at http://localhost:{}", port);
    axum::serve(listener, app).await.unwrap();
}

async fn index() -> Html<&'static str> {
    Html(include_str!("index.html"))
}

const MAX_FILE_SIZE: usize = 50 * 1024 * 1024; // 50MB

async fn upload(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (StatusCode::BAD_REQUEST, format!("Multipart error: {}", e))
    })? {
        if field.name() == Some("file") {
            let data = field.bytes().await.map_err(|e| {
                (StatusCode::BAD_REQUEST, format!("Read error: {}", e))
            })?;
            
            if data.len() > MAX_FILE_SIZE {
                return Err((StatusCode::BAD_REQUEST, "文件过大，最大支持 50MB".to_string()));
            }
            
            if data.len() < 4 || &data[..4] != b"%PDF" {
                return Err((StatusCode::BAD_REQUEST, "无效的 PDF 文件".to_string()));
            }
            
            let task_id = uuid::Uuid::new_v4().to_string();
            state.create_task(&task_id);
            
            let state_clone = state.clone();
            let task_id_clone = task_id.clone();
            
            tokio::spawn(async move {
                process_pdf(state_clone, task_id_clone, data.to_vec()).await;
            });
            
            return Ok(serde_json::json!({ "task_id": task_id }).to_string());
        }
    }
    
    Err((StatusCode::BAD_REQUEST, "No file uploaded".to_string()))
}

async fn process_pdf(state: Arc<AppState>, task_id: String, data: Vec<u8>) {
    // Step 1: Render PDF to images
    let pages = match pdf::render_pdf_pages(&data) {
        Ok(p) => p,
        Err(e) => {
            state.set_error(&task_id, format!("PDF 渲染失败: {}", e));
            return;
        }
    };
    
    let total_pages = pages.len();
    state.set_rendering(&task_id, total_pages);
    
    let mut recognized_texts: Vec<String> = Vec::with_capacity(total_pages);
    let mut translated_texts: Vec<String> = Vec::with_capacity(total_pages);
    
    // Step 2: Recognize text from each page
    for page in &pages {
        state.set_recognizing(&task_id, page.page_num, total_pages);
        
        match translate::recognize_text(&state.config, &page.image_base64).await {
            Ok(text) => {
                recognized_texts.push(text);
            }
            Err(e) => {
                state.set_error(&task_id, format!("识别第 {} 页失败: {}", page.page_num, e));
                return;
            }
        }
    }
    
    // Step 3: Translate each page
    for (i, text) in recognized_texts.iter().enumerate() {
        let page_num = i + 1;
        state.set_translating(&task_id, page_num, total_pages);
        
        match translate::translate_text(&state.config, text).await {
            Ok(translated) => {
                translated_texts.push(translated);
            }
            Err(e) => {
                state.set_error(&task_id, format!("翻译第 {} 页失败: {}", page_num, e));
                return;
            }
        }
    }
    
    // Step 4: Generate output PDF
    state.set_generating(&task_id);
    
    match pdf::generate_pdf(&translated_texts) {
        Ok(pdf_data) => {
            state.set_complete(&task_id, pdf_data);
        }
        Err(e) => {
            state.set_error(&task_id, format!("生成 PDF 失败: {}", e));
        }
    }
}

async fn progress(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Sse<impl tokio_stream::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>> {
    let stream = async_stream::stream! {
        loop {
            if let Some(progress) = state.get_progress(&task_id) {
                let is_done = progress.is_done();
                let event = axum::response::sse::Event::default()
                    .data(serde_json::to_string(&progress).unwrap_or_default());
                yield Ok(event);
                
                if is_done {
                    break;
                }
            } else {
                let event = axum::response::sse::Event::default()
                    .data(r#"{"status":"Error","message":"任务不存在"}"#);
                yield Ok(event);
                break;
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        }
    };
    
    Sse::new(stream)
}

async fn download(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    if let Some(pdf_data) = state.get_pdf_data(&task_id) {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/pdf")
            .header(header::CONTENT_DISPOSITION, "attachment; filename=\"translated.pdf\"")
            .body(Body::from((*pdf_data).clone()))
            .unwrap();
    }
    
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("Not found"))
        .unwrap()
}
