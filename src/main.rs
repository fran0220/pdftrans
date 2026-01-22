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


use crate::state::{AppState, MAX_CONCURRENT_TASKS, PageDetail};

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
        .route("/retry/{task_id}", post(retry_task))
        .route("/download/{task_id}", get(download))
        .route("/tasks", get(list_tasks))
        .route("/tasks/{task_id}/pages/{page_num}", get(get_page_detail))
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
            
            let data_vec = data.to_vec();
            
            // 保存输入 PDF 到磁盘
            if let Err(e) = state::save_input_pdf(&task_id, &data_vec) {
                state.release_task_slot();
                return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("保存文件失败: {}", e)));
            }
            
            let state_clone = state.clone();
            let task_id_clone = task_id.clone();
            
            tokio::spawn(async move {
                process_pdf_parallel(state_clone, task_id_clone, data_vec).await;
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
    
    // Load all texts from disk (more reliable than in-memory)
    let texts = state::load_all_translated_pages(&task_id, total_pages);
    
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

const BATCH_SIZE: usize = 3;

async fn process_pages_parallel(
    state: &Arc<AppState>,
    task_id: &str,
    pages: Vec<pdf::PdfPage>,
) -> Vec<Result<(usize, String), String>> {
    use tokio::task::JoinSet;
    
    let mut all_results = Vec::new();
    let mut pages_iter = pages.into_iter().peekable();
    
    // Process pages in batches: 1-3 OCR → 1-3 Translate → 4-6 OCR → 4-6 Translate → ...
    while pages_iter.peek().is_some() {
        if state.is_cancelled(task_id) {
            all_results.push(Err("任务已取消".to_string()));
            break;
        }
        
        let batch: Vec<pdf::PdfPage> = pages_iter.by_ref().take(BATCH_SIZE).collect();
        let page_nums: Vec<usize> = batch.iter().map(|p| p.page_num).collect();
        
        // === Phase 1: OCR all pages in batch concurrently ===
        state.add_log(task_id, format!("开始 OCR 第 {:?} 页", page_nums));
        
        let mut ocr_set: JoinSet<Result<(usize, String), String>> = JoinSet::new();
        for page in batch {
            let state = state.clone();
            let task_id = task_id.to_string();
            let config = state.config.clone();
            
            ocr_set.spawn(async move {
                if state.is_cancelled(&task_id) {
                    return Err("任务已取消".to_string());
                }
                
                let page_num = page.page_num;
                state.start_page_ocr(&task_id, page_num);
                let page_task_id = format!("{}-p{}", task_id, page_num);
                
                let text = if let Some(ref image_base64) = page.image_base64 {
                    match translate::recognize_text(&config, image_base64, &page_task_id).await {
                        Ok(t) => {
                            let _ = state::save_page_ocr(&task_id, page_num, &t);
                            let preview = t.chars().take(300).collect::<String>();
                            state.finish_page_ocr(&task_id, page_num, t.chars().count(), preview);
                            state.add_log(&task_id, format!("第 {} 页 OCR 完成 ({} 字符)", page_num, t.chars().count()));
                            t
                        }
                        Err(e) => {
                            state.set_page_error(&task_id, page_num, e.clone());
                            return Err(format!("第 {} 页 OCR 失败: {}", page_num, e));
                        }
                    }
                } else if let Some(ref extracted) = page.extracted_text {
                    let _ = state::save_page_ocr(&task_id, page_num, extracted);
                    let preview = extracted.chars().take(300).collect::<String>();
                    state.finish_page_ocr(&task_id, page_num, extracted.chars().count(), preview);
                    extracted.clone()
                } else {
                    state.finish_page_ocr(&task_id, page_num, 0, String::new());
                    String::new()
                };
                
                Ok((page_num, text))
            });
        }
        
        // Collect OCR results
        let mut ocr_results: Vec<(usize, String)> = Vec::new();
        let mut batch_has_error = false;
        
        while let Some(result) = ocr_set.join_next().await {
            if state.is_cancelled(task_id) {
                ocr_set.abort_all();
                all_results.push(Err("任务已取消".to_string()));
                batch_has_error = true;
                break;
            }
            
            match result {
                Ok(Ok(r)) => ocr_results.push(r),
                Ok(Err(e)) => {
                    batch_has_error = true;
                    ocr_set.abort_all();
                    all_results.push(Err(e));
                    break;
                }
                Err(e) => {
                    batch_has_error = true;
                    all_results.push(Err(format!("OCR 任务执行错误: {}", e)));
                    break;
                }
            }
        }
        
        if batch_has_error {
            break;
        }
        
        // === Phase 2: Translate all pages in batch concurrently ===
        state.add_log(task_id, format!("开始翻译第 {:?} 页", page_nums));
        
        let mut translate_set: JoinSet<Result<(usize, String), String>> = JoinSet::new();
        for (page_num, text) in ocr_results {
            let state = state.clone();
            let task_id = task_id.to_string();
            let config = state.config.clone();
            
            translate_set.spawn(async move {
                if state.is_cancelled(&task_id) {
                    return Err("任务已取消".to_string());
                }
                
                state.start_page_translate(&task_id, page_num);
                let page_task_id = format!("{}-p{}", task_id, page_num);
                
                match translate::translate_text(&config, &text, &page_task_id).await {
                    Ok(translated) => {
                        let _ = state::save_page_translated(&task_id, page_num, &translated);
                        let char_count = translated.chars().count();
                        let preview = translated.chars().take(300).collect::<String>();
                        state.finish_page_translate(&task_id, page_num, char_count, preview);
                        state.add_log(&task_id, format!("第 {} 页翻译完成 ({} 字符)", page_num, char_count));
                        Ok((page_num, translated))
                    }
                    Err(e) => {
                        state.set_page_error(&task_id, page_num, e.clone());
                        Err(format!("第 {} 页翻译失败: {}", page_num, e))
                    }
                }
            });
        }
        
        // Collect translate results
        while let Some(result) = translate_set.join_next().await {
            if state.is_cancelled(task_id) {
                translate_set.abort_all();
                all_results.push(Err("任务已取消".to_string()));
                batch_has_error = true;
                break;
            }
            
            match result {
                Ok(Ok(r)) => all_results.push(Ok(r)),
                Ok(Err(e)) => {
                    batch_has_error = true;
                    translate_set.abort_all();
                    all_results.push(Err(e));
                    break;
                }
                Err(e) => {
                    batch_has_error = true;
                    all_results.push(Err(format!("翻译任务执行错误: {}", e)));
                    break;
                }
            }
        }
        
        if batch_has_error {
            break;
        }
    }
    
    all_results
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

async fn retry_task(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // 先检查文件是否存在（在改变状态之前）
    let pdf_bytes = match state::load_input_pdf(&task_id) {
        Ok(bytes) => bytes,
        Err(_) => {
            return Err((StatusCode::GONE, "任务已过期，原始 PDF 已清理".to_string()));
        }
    };
    
    // 尝试获取并发槽位（在改变状态之前）
    if !state.try_acquire_task_slot() {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            format!("服务繁忙，当前已有 {} 个任务在处理，请稍后重试", MAX_CONCURRENT_TASKS)
        ));
    }
    
    // 所有前置检查通过后，才改变任务状态
    if let Err(e) = state.try_start_retry(&task_id) {
        state.release_task_slot();
        return Err((StatusCode::BAD_REQUEST, e));
    }
    
    let state_clone = state.clone();
    let task_id_clone = task_id.clone();
    
    tokio::spawn(async move {
        process_retry(state_clone, task_id_clone, pdf_bytes).await;
    });
    
    Ok(Json(serde_json::json!({ "status": "retrying" })))
}

async fn process_retry(state: Arc<AppState>, task_id: String, pdf_bytes: Vec<u8>) {
    let _guard = TaskGuard { state: state.clone() };
    
    // Re-render pages
    let pages = match pdf::process_pdf_pages(&pdf_bytes) {
        Ok(p) => p,
        Err(e) => {
            state.set_error(&task_id, format!("PDF 处理失败: {}", e));
            state.finish_retry(&task_id);
            return;
        }
    };
    
    let total_pages = pages.len();
    if total_pages == 0 {
        state.set_error(&task_id, "PDF 没有页面".to_string());
        state.finish_retry(&task_id);
        return;
    }
    
    // Get completed page count from disk
    let completed_count = state::get_completed_page_count(&task_id);
    
    // Filter pending pages (check if translated file exists)
    let pending_pages: Vec<_> = pages.into_iter()
        .filter(|p| state::load_page_translated(&task_id, p.page_num).is_none())
        .collect();
    
    if pending_pages.is_empty() {
        // All pages done, generate PDF from disk
        let texts = state::load_all_translated_pages(&task_id, total_pages);
        
        state.set_generating(&task_id);
        match pdf::generate_pdf(&texts) {
            Ok(pdf_data) => {
                state.set_complete(&task_id, pdf_data);
            }
            Err(e) => {
                state.set_error(&task_id, format!("生成 PDF 失败: {}", e));
            }
        }
        state.finish_retry(&task_id);
        return;
    }
    
    // Initialize progress
    state.init_retry_progress(&task_id, completed_count, total_pages);
    state.add_log(&task_id, format!("继续处理，已完成 {}/{} 页", completed_count, total_pages));
    
    // Process pending pages
    let results = process_pages_parallel(&state, &task_id, pending_pages).await;
    
    // Check if cancelled
    if state.is_cancelled(&task_id) {
        state.finish_retry(&task_id);
        return;
    }
    
    // Merge results
    let mut has_error = false;
    for result in results {
        match result {
            Ok((page_num, text)) => {
                // Already saved to disk in process_pages_parallel
                let _ = (page_num, text);
            }
            Err(e) => {
                state.set_error(&task_id, e);
                has_error = true;
                break;
            }
        }
    }
    
    if has_error || state.is_cancelled(&task_id) {
        state.finish_retry(&task_id);
        return;
    }
    
    // Load all texts from disk
    let texts = state::load_all_translated_pages(&task_id, total_pages);
    
    // Generate PDF
    state.set_generating(&task_id);
    match pdf::generate_pdf(&texts) {
        Ok(pdf_data) => {
            state.set_complete(&task_id, pdf_data);
        }
        Err(e) => {
            state.set_error(&task_id, format!("生成 PDF 失败: {}", e));
        }
    }
    state.finish_retry(&task_id);
}

async fn list_tasks(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<state::TaskSummary>> {
    Json(state.get_all_tasks())
}

async fn get_page_detail(
    Path((task_id, page_num)): Path<(String, usize)>,
) -> Result<Json<PageDetail>, (StatusCode, String)> {
    state::load_page_detail(&task_id, page_num)
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, "页面不存在或未处理".to_string()))
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
