use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use std::path::PathBuf;
use std::fs;
use std::io::Write;

use crate::config::Config;

const DATA_DIR: &str = "data/tasks";

pub const MAX_CONCURRENT_TASKS: usize = 1;
const MAX_LOGS: usize = 50;

#[derive(Clone, Serialize, PartialEq)]
pub enum TaskStatus {
    Rendering,
    Processing,  // Combined OCR + Translate (parallel)
    Generating,
    Complete,
    Error,
}

#[derive(Clone, Serialize)]
pub struct LogEntry {
    pub ts: u64,
    pub msg: String,
}

#[derive(Clone, Serialize, Default)]
pub struct PageSummary {
    pub page_num: usize,
    pub ocr_started: Option<u64>,
    pub ocr_duration_ms: Option<u64>,
    pub ocr_chars: Option<usize>,
    pub ocr_text_preview: Option<String>,      // OCR 识别的文本预览（前200字）
    pub translate_started: Option<u64>,
    pub translate_duration_ms: Option<u64>,
    pub translated_chars: Option<usize>,
    pub translated_text_preview: Option<String>, // 翻译结果预览（前200字）
    pub status: String,  // "pending", "ocr", "translating", "done", "error"
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct TaskProgress {
    pub status: TaskStatus,
    pub total_pages: usize,
    pub ocr_done: usize,
    pub translate_done: usize,
    pub message: String,
    pub overall_percent: u8,
    pub filename: String,
    pub logs: Vec<LogEntry>,
    pub page_summaries: Vec<PageSummary>,
}

impl TaskProgress {
    pub fn is_done(&self) -> bool {
        matches!(self.status, TaskStatus::Complete | TaskStatus::Error)
    }
}

#[derive(Clone, Serialize)]
pub struct TaskSummary {
    pub task_id: String,
    pub filename: String,
    pub status: TaskStatus,
    pub overall_percent: u8,
    pub ocr_done: usize,
    pub translate_done: usize,
    pub total_pages: usize,
}

pub struct TaskData {
    pub progress: TaskProgress,
    pub pdf_data: Option<Arc<Vec<u8>>>,
    pub cancelled: bool,
    pub started_at: u64,
    pub is_retrying: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PageDetail {
    pub page_num: usize,
    pub ocr_text: String,
    pub translated_text: String,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn task_dir(task_id: &str) -> PathBuf {
    PathBuf::from(DATA_DIR).join(task_id)
}

fn pages_dir(task_id: &str) -> PathBuf {
    task_dir(task_id).join("pages")
}

pub fn save_input_pdf(task_id: &str, data: &[u8]) -> std::io::Result<()> {
    let dir = task_dir(task_id);
    fs::create_dir_all(&dir)?;
    let path = dir.join("input.pdf");
    let tmp_path = dir.join("input.pdf.tmp");
    let mut file = fs::File::create(&tmp_path)?;
    file.write_all(data)?;
    file.sync_all()?;
    fs::rename(tmp_path, path)?;
    Ok(())
}

pub fn load_input_pdf(task_id: &str) -> std::io::Result<Vec<u8>> {
    let path = task_dir(task_id).join("input.pdf");
    fs::read(path)
}

pub fn save_page_ocr(task_id: &str, page_num: usize, text: &str) -> std::io::Result<()> {
    let dir = pages_dir(task_id);
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.ocr.txt", page_num));
    let tmp_path = dir.join(format!("{}.ocr.txt.tmp", page_num));
    fs::write(&tmp_path, text)?;
    fs::rename(tmp_path, path)?;
    Ok(())
}

pub fn save_page_translated(task_id: &str, page_num: usize, text: &str) -> std::io::Result<()> {
    let dir = pages_dir(task_id);
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.translated.txt", page_num));
    let tmp_path = dir.join(format!("{}.translated.txt.tmp", page_num));
    fs::write(&tmp_path, text)?;
    fs::rename(tmp_path, path)?;
    Ok(())
}

pub fn load_page_ocr(task_id: &str, page_num: usize) -> Option<String> {
    let path = pages_dir(task_id).join(format!("{}.ocr.txt", page_num));
    fs::read_to_string(path).ok()
}

pub fn load_page_translated(task_id: &str, page_num: usize) -> Option<String> {
    let path = pages_dir(task_id).join(format!("{}.translated.txt", page_num));
    fs::read_to_string(path).ok()
}

pub fn load_page_detail(task_id: &str, page_num: usize) -> Option<PageDetail> {
    let ocr_text = load_page_ocr(task_id, page_num).unwrap_or_default();
    let translated_text = load_page_translated(task_id, page_num).unwrap_or_default();
    if ocr_text.is_empty() && translated_text.is_empty() {
        return None;
    }
    Some(PageDetail {
        page_num,
        ocr_text,
        translated_text,
    })
}

pub fn load_all_translated_pages(task_id: &str, total_pages: usize) -> Vec<String> {
    (1..=total_pages)
        .map(|i| load_page_translated(task_id, i).unwrap_or_default())
        .collect()
}

pub fn get_completed_page_count(task_id: &str) -> usize {
    let dir = pages_dir(task_id);
    if !dir.exists() {
        return 0;
    }
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map(|ext| ext == "txt").unwrap_or(false))
                .filter(|e| e.file_name().to_string_lossy().ends_with(".translated.txt"))
                .count()
        })
        .unwrap_or(0)
}

fn cleanup_task_files(task_id: &str) {
    let dir = task_dir(task_id);
    if dir.exists() {
        let _ = fs::remove_dir_all(dir);
    }
}

pub struct AppState {
    pub config: Config,
    tasks: RwLock<HashMap<String, TaskData>>,
    active_task_count: AtomicUsize,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            tasks: RwLock::new(HashMap::new()),
            active_task_count: AtomicUsize::new(0),
        }
    }

    pub fn try_acquire_task_slot(&self) -> bool {
        // Use CAS loop for atomic check-and-increment
        loop {
            let current = self.active_task_count.load(Ordering::SeqCst);
            if current >= MAX_CONCURRENT_TASKS {
                return false;
            }
            match self.active_task_count.compare_exchange(
                current,
                current + 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return true,
                Err(_) => continue, // Retry
            }
        }
    }

    pub fn release_task_slot(&self) {
        self.active_task_count.fetch_sub(1, Ordering::SeqCst);
    }

    #[allow(dead_code)]
    pub fn active_task_count(&self) -> usize {
        self.active_task_count.load(Ordering::SeqCst)
    }

    pub fn create_task(&self, task_id: &str, filename: &str) {
        let now = now_ms();
        let task = TaskData {
            progress: TaskProgress {
                status: TaskStatus::Rendering,
                total_pages: 0,
                ocr_done: 0,
                translate_done: 0,
                message: "正在处理 PDF...".to_string(),
                overall_percent: 0,
                filename: filename.to_string(),
                logs: vec![LogEntry { ts: now, msg: "任务开始".to_string() }],
                page_summaries: Vec::new(),
            },
            pdf_data: None,
            cancelled: false,
            started_at: now,
            is_retrying: false,
        };
        self.tasks.write().insert(task_id.to_string(), task);
    }

    pub fn cancel_task(&self, task_id: &str) -> bool {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            if !task.progress.is_done() {
                task.cancelled = true;
                task.progress.status = TaskStatus::Error;
                task.progress.message = "任务已取消".to_string();
                task.progress.logs.push(LogEntry { ts: now_ms(), msg: "任务取消".to_string() });
                return true;
            }
        }
        false
    }

    pub fn is_cancelled(&self, task_id: &str) -> bool {
        self.tasks.read().get(task_id).map(|t| t.cancelled).unwrap_or(false)
    }

    pub fn set_rendering(&self, task_id: &str, total_pages: usize) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.status = TaskStatus::Rendering;
            task.progress.total_pages = total_pages;
            task.progress.overall_percent = 5;
            task.progress.message = format!("共 {} 页，开始并行处理...", total_pages);
            task.progress.logs.push(LogEntry { ts: now_ms(), msg: format!("渲染完成，共 {} 页", total_pages) });
            // Initialize page summaries
            task.progress.page_summaries = (1..=total_pages)
                .map(|i| PageSummary {
                    page_num: i,
                    status: "pending".to_string(),
                    ..Default::default()
                })
                .collect();
        }
    }

    pub fn set_processing(&self, task_id: &str) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.status = TaskStatus::Processing;
            task.progress.message = "并行处理中...".to_string();
            task.progress.logs.push(LogEntry { ts: now_ms(), msg: "开始并行 OCR + 翻译".to_string() });
        }
    }

    fn update_progress(&self, task: &mut TaskData) {
        let total = task.progress.total_pages;
        if total == 0 { return; }
        
        let ocr = task.progress.ocr_done;
        let trans = task.progress.translate_done;
        
        // Progress: 5% (render) + 45% (OCR) + 45% (translate) + 5% (generate)
        let ocr_pct = (ocr as f32 / total as f32) * 45.0;
        let trans_pct = (trans as f32 / total as f32) * 45.0;
        task.progress.overall_percent = (5.0 + ocr_pct + trans_pct) as u8;
        
        task.progress.message = format!("OCR: {}/{}, 翻译: {}/{}", ocr, total, trans, total);
    }

    pub fn set_generating(&self, task_id: &str) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.status = TaskStatus::Generating;
            task.progress.overall_percent = 95;
            task.progress.message = "正在生成 PDF...".to_string();
            task.progress.logs.push(LogEntry { ts: now_ms(), msg: "开始生成 PDF".to_string() });
        }
    }

    pub fn set_complete(&self, task_id: &str, pdf_data: Vec<u8>) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            let elapsed = (now_ms() - task.started_at) / 1000;
            task.progress.status = TaskStatus::Complete;
            task.progress.overall_percent = 100;
            task.progress.message = format!("完成！用时 {} 秒", elapsed);
            task.progress.logs.push(LogEntry { ts: now_ms(), msg: format!("完成，用时 {} 秒", elapsed) });
            task.pdf_data = Some(Arc::new(pdf_data));
        }
    }

    pub fn set_error(&self, task_id: &str, error: String) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.status = TaskStatus::Error;
            task.progress.message = error.clone();
            task.progress.logs.push(LogEntry { ts: now_ms(), msg: format!("错误: {}", error) });
        }
    }

    pub fn add_log(&self, task_id: &str, msg: String) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.logs.push(LogEntry { ts: now_ms(), msg });
            if task.progress.logs.len() > MAX_LOGS {
                task.progress.logs.remove(0);
            }
        }
    }

    pub fn start_page_ocr(&self, task_id: &str, page_num: usize) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            if let Some(ps) = task.progress.page_summaries.get_mut(page_num - 1) {
                ps.ocr_started = Some(now_ms());
                ps.status = "ocr".to_string();
                ps.error = None; // 清除之前的错误
            }
        }
    }

    pub fn finish_page_ocr(&self, task_id: &str, page_num: usize, char_count: usize, text_preview: String) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.ocr_done += 1;
            if let Some(ps) = task.progress.page_summaries.get_mut(page_num - 1) {
                if let Some(started) = ps.ocr_started {
                    ps.ocr_duration_ms = Some(now_ms() - started);
                }
                ps.ocr_chars = Some(char_count);
                ps.ocr_text_preview = Some(text_preview);
            }
            self.update_progress(task);
        }
    }

    pub fn start_page_translate(&self, task_id: &str, page_num: usize) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            if let Some(ps) = task.progress.page_summaries.get_mut(page_num - 1) {
                ps.translate_started = Some(now_ms());
                ps.status = "translating".to_string();
            }
        }
    }

    pub fn finish_page_translate(&self, task_id: &str, page_num: usize, char_count: usize, text_preview: String) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.translate_done += 1;
            if let Some(ps) = task.progress.page_summaries.get_mut(page_num - 1) {
                if let Some(started) = ps.translate_started {
                    ps.translate_duration_ms = Some(now_ms() - started);
                }
                ps.translated_chars = Some(char_count);
                ps.translated_text_preview = Some(text_preview);
                ps.status = "done".to_string();
                ps.error = None; // 确保成功时清除错误
            }
            self.update_progress(task);
        }
    }

    pub fn set_page_error(&self, task_id: &str, page_num: usize, error: String) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            if let Some(ps) = task.progress.page_summaries.get_mut(page_num - 1) {
                ps.status = "error".to_string();
                ps.error = Some(error);
            }
        }
    }

    pub fn get_progress(&self, task_id: &str) -> Option<TaskProgress> {
        self.tasks.read().get(task_id).map(|t| t.progress.clone())
    }

    pub fn get_pdf_data(&self, task_id: &str) -> Option<Arc<Vec<u8>>> {
        self.tasks.read().get(task_id).and_then(|t| t.pdf_data.clone())
    }

    pub fn get_all_tasks(&self) -> Vec<TaskSummary> {
        self.tasks.read().iter().map(|(id, t)| TaskSummary {
            task_id: id.clone(),
            filename: t.progress.filename.clone(),
            status: t.progress.status.clone(),
            overall_percent: t.progress.overall_percent,
            ocr_done: t.progress.ocr_done,
            translate_done: t.progress.translate_done,
            total_pages: t.progress.total_pages,
        }).collect()
    }

    #[allow(dead_code)]
    pub fn cleanup_old_tasks(&self) {
        let now = now_ms();
        let mut tasks = self.tasks.write();
        let to_cleanup: Vec<String> = tasks.iter()
            .filter(|(_, t)| t.progress.is_done() && (now - t.started_at) >= 3600_000)
            .map(|(id, _)| id.clone())
            .collect();
        
        for task_id in &to_cleanup {
            cleanup_task_files(task_id);
        }
        
        tasks.retain(|id, _| !to_cleanup.contains(id));
    }

    pub fn try_start_retry(&self, task_id: &str) -> Result<(), String> {
        let mut tasks = self.tasks.write();
        let task = tasks.get_mut(task_id).ok_or("任务不存在")?;
        
        if task.progress.status != TaskStatus::Error {
            return Err("只能重试失败的任务".to_string());
        }
        if task.cancelled {
            return Err("已取消的任务不能重试".to_string());
        }
        if task.is_retrying {
            return Err("任务正在重试中".to_string());
        }
        
        task.is_retrying = true;
        task.progress.status = TaskStatus::Processing;
        task.progress.message = "重试中...".to_string();
        task.progress.logs.push(LogEntry { ts: now_ms(), msg: "开始重试".to_string() });
        Ok(())
    }

    pub fn finish_retry(&self, task_id: &str) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.is_retrying = false;
        }
    }

    pub fn init_retry_progress(&self, task_id: &str, completed_count: usize, total_pages: usize) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.translate_done = completed_count;
            task.progress.ocr_done = completed_count;
            task.progress.total_pages = total_pages;
            self.update_progress(task);
        }
    }
    
    #[allow(dead_code)]
    pub fn get_total_pages(&self, task_id: &str) -> usize {
        self.tasks.read().get(task_id)
            .map(|t| t.progress.total_pages)
            .unwrap_or(0)
    }
}
