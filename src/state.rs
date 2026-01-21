use parking_lot::RwLock;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;

pub const MAX_CONCURRENT_TASKS: usize = 3;
pub const MAX_PARALLEL_PAGES: usize = 20;

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
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
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
            },
            pdf_data: None,
            cancelled: false,
            started_at: now,
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
        }
    }

    pub fn set_processing(&self, task_id: &str) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.status = TaskStatus::Processing;
            task.progress.message = "并行处理中...".to_string();
            task.progress.logs.push(LogEntry { ts: now_ms(), msg: "开始并行 OCR + 翻译".to_string() });
        }
    }

    pub fn increment_ocr_done(&self, task_id: &str) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.ocr_done += 1;
            self.update_progress(task);
        }
    }

    pub fn increment_translate_done(&self, task_id: &str) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.translate_done += 1;
            self.update_progress(task);
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
            if task.progress.logs.len() > 30 {
                task.progress.logs.remove(0);
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

    pub fn cleanup_old_tasks(&self) {
        let now = now_ms();
        let mut tasks = self.tasks.write();
        tasks.retain(|_, t| {
            // Keep tasks less than 1 hour old, or not done yet
            !t.progress.is_done() || (now - t.started_at) < 3600_000
        });
    }
}
