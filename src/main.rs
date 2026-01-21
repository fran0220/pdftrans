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
    Json,
};
use std::sync::Arc;
use tower_http::cors::CorsLayer;


use crate::state::{AppState, MAX_CONCURRENT_TASKS};

#[tokio::main]
async fn main() {
    let config = config::Config::from_env();
    println!("PDF Translator V2 (Parallel) starting...");
    println!("API Base URL: {}", config.base_url);
    println!("OCR Model: {}", config.ocr_model);
    println!("Translate Model: {}", config.translate_model);
    println!("Max concurrent tasks: {}", MAX_CONCURRENT_TASKS);
    
    let state = Arc::new(AppState::new(config));
    
    let app = Router::new()
        .route("/", get(index))
        .route("/upload", post(upload))
        .route("/progress/{task_id}", get(progress))
        .route("/cancel/{task_id}", post(cancel))
        .route("/download/{task_id}", get(download))
        .route("/tasks", get(list_tasks))
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

const MAX_FILE_SIZE: usize = 50 * 1024 * 1024;

async fn upload(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Check task limit
    if !state.try_acquire_task_slot() {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!("服务繁忙，当前已有 {} 个任务在处理，请稍后重试", MAX_CONCURRENT_TASKS)
        ));
    }

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        state.release_task_slot();
        (StatusCode::BAD_REQUEST, format!("Multipart error: {}", e))
    })? {
        if field.name() == Some("file") {
            let filename = field.file_name().unwrap_or("unknown.pdf").to_string();
            let data = field.bytes().await.map_err(|e| {
                state.release_task_slot();
                (StatusCode::BAD_REQUEST, format!("Read error: {}", e))
            })?;
            
            if data.len() > MAX_FILE_SIZE {
                state.release_task_slot();
                return Err((StatusCode::BAD_REQUEST, "文件过大，最大支持 50MB".to_string()));
            }
            
            if data.len() < 4 || &data[..4] != b"%PDF" {
                state.release_task_slot();
                return Err((StatusCode::BAD_REQUEST, "无效的 PDF 文件".to_string()));
            }
            
            let task_id = uuid::Uuid::new_v4().to_string();
            state.create_task(&task_id, &filename);
            
            let state_clone = state.clone();
            let task_id_clone = task_id.clone();
            
            tokio::spawn(async move {
                process_pdf_parallel(state_clone, task_id_clone, data.to_vec()).await;
            });
            
            return Ok(Json(serde_json::json!({ "task_id": task_id })));
        }
    }
    
    state.release_task_slot();
    Err((StatusCode::BAD_REQUEST, "No file uploaded".to_string()))
}

async fn process_pdf_parallel(state: Arc<AppState>, task_id: String, data: Vec<u8>) {
    // Ensure we release the slot when done
    let _guard = TaskGuard { state: state.clone() };
    
    // Step 1: Render PDF to images
    let pages = match pdf::process_pdf_pages(&data) {
        Ok(p) => p,
        Err(e) => {
            state.set_error(&task_id, format!("PDF 处理失败: {}", e));
            return;
        }
    };
    
    let total_pages = pages.len();
    if total_pages == 0 {
        state.set_error(&task_id, "PDF 没有页面".to_string());
        return;
    }
    
    state.set_rendering(&task_id, total_pages);
    state.set_processing(&task_id);
    
    // Step 2: Process all pages in parallel (OCR + Translate per page)
    let results = process_pages_parallel(&state, &task_id, pages).await;
    
    // Check if cancelled
    if state.is_cancelled(&task_id) {
        return;
    }
    
    // Collect results in order
    let mut translated_texts: Vec<Option<String>> = vec![None; total_pages];
    let mut has_error = false;
    
    for result in results {
        match result {
            Ok((page_num, text)) => {
                translated_texts[page_num - 1] = Some(text);
            }
            Err(e) => {
                state.set_error(&task_id, e);
                has_error = true;
                break;
            }
        }
    }
    
    if has_error || state.is_cancelled(&task_id) {
        return;
    }
    
    // Convert to Vec<String>
    let texts: Vec<String> = translated_texts.into_iter()
        .map(|t| t.unwrap_or_default())
        .collect();
    
    // Step 3: Generate PDF
    state.set_generating(&task_id);
    
    match pdf::generate_pdf(&texts) {
        Ok(pdf_data) => {
            state.set_complete(&task_id, pdf_data);
        }
        Err(e) => {
            state.set_error(&task_id, format!("生成 PDF 失败: {}", e));
        }
    }
}

async fn process_pages_parallel(
    state: &Arc<AppState>,
    task_id: &str,
    pages: Vec<pdf::PdfPage>,
) -> Vec<Result<(usize, String), String>> {
    use tokio::task::JoinSet;
    
    let mut join_set: JoinSet<Result<(usize, String), String>> = JoinSet::new();
    
    for page in pages {
        let state = state.clone();
        let task_id = task_id.to_string();
        let config = state.config.clone();
        
        join_set.spawn(async move {
            // Check cancelled before starting
            if state.is_cancelled(&task_id) {
                return Err("任务已取消".to_string());
            }
            
            let page_num = page.page_num;
            
            // OCR
            let text = if let Some(ref image_base64) = page.image_base64 {
                match translate::recognize_text(&config, image_base64).await {
                    Ok(t) => {
                        state.increment_ocr_done(&task_id);
                        state.add_log(&task_id, format!("第 {} 页 OCR 完成", page_num));
                        t
                    }
                    Err(e) => {
                        return Err(format!("第 {} 页 OCR 失败: {}", page_num, e));
                    }
                }
            } else if let Some(ref extracted) = page.extracted_text {
                state.increment_ocr_done(&task_id);
                extracted.clone()
            } else {
                state.increment_ocr_done(&task_id);
                String::new()
            };
            
            // Check cancelled before translate
            if state.is_cancelled(&task_id) {
                return Err("任务已取消".to_string());
            }
            
            // Translate
            match translate::translate_text(&config, &text).await {
                Ok(translated) => {
                    state.increment_translate_done(&task_id);
                    state.add_log(&task_id, format!("第 {} 页翻译完成", page_num));
                    Ok((page_num, translated))
                }
                Err(e) => {
                    Err(format!("第 {} 页翻译失败: {}", page_num, e))
                }
            }
        });
    }
    
    // Collect results, abort all on first error or cancellation
    let mut results = Vec::new();
    let mut has_error = false;
    
    while let Some(result) = join_set.join_next().await {
        // Check if cancelled
        if state.is_cancelled(task_id) {
            join_set.abort_all();
            results.push(Err("任务已取消".to_string()));
            break;
        }
        
        match result {
            Ok(Ok(r)) => results.push(Ok(r)),
            Ok(Err(e)) => {
                // First error - abort remaining tasks
                if !has_error {
                    has_error = true;
                    join_set.abort_all();
                }
                results.push(Err(e));
            }
            Err(e) => {
                results.push(Err(format!("任务执行错误: {}", e)));
            }
        }
    }
    
    results
}

// Guard to release task slot on drop
struct TaskGuard {
    state: Arc<AppState>,
}

impl Drop for TaskGuard {
    fn drop(&mut self) {
        self.state.release_task_slot();
    }
}

async fn cancel(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> impl IntoResponse {
    if state.cancel_task(&task_id) {
        (StatusCode::OK, "cancelled")
    } else {
        (StatusCode::NOT_FOUND, "not found or already done")
    }
}

async fn list_tasks(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<state::TaskSummary>> {
    Json(state.get_all_tasks())
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
            tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
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
