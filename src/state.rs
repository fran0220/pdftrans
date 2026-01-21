use parking_lot::RwLock;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Config;

#[derive(Clone, Serialize, PartialEq)]
pub enum TaskStatus {
    Rendering,
    Recognizing,
    Translating,
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
    pub current_page: usize,
    pub total_pages: usize,
    pub message: String,
    // New fields
    pub overall_percent: u8,
    pub stage_label: String,
    pub ocr_pages: usize,      // Pages needing OCR
    pub text_pages: usize,     // Pages with extracted text
    pub eta_seconds: Option<u64>,
    pub logs: Vec<LogEntry>,
}

impl TaskProgress {
    pub fn is_done(&self) -> bool {
        matches!(self.status, TaskStatus::Complete | TaskStatus::Error)
    }
}

pub struct TaskData {
    pub progress: TaskProgress,
    pub pdf_data: Option<Arc<Vec<u8>>>,
    pub cancelled: bool,
    // Timing stats
    started_at: u64,
    page_times: Vec<u64>,  // Time per page in ms
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
}

impl AppState {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            tasks: RwLock::new(HashMap::new()),
        }
    }

    pub fn create_task(&self, task_id: &str) {
        let now = now_ms();
        let task = TaskData {
            progress: TaskProgress {
                status: TaskStatus::Rendering,
                current_page: 0,
                total_pages: 0,
                message: "正在处理 PDF...".to_string(),
                overall_percent: 0,
                stage_label: "解析".to_string(),
                ocr_pages: 0,
                text_pages: 0,
                eta_seconds: None,
                logs: vec![LogEntry { ts: now, msg: "任务开始".to_string() }],
            },
            pdf_data: None,
            cancelled: false,
            started_at: now,
            page_times: Vec::new(),
        };
        self.tasks.write().insert(task_id.to_string(), task);
    }

    pub fn cancel_task(&self, task_id: &str) -> bool {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            if !task.progress.is_done() {
                task.cancelled = true;
                task.progress.status = TaskStatus::Error;
                task.progress.message = "任务已取消".to_string();
                task.progress.logs.push(LogEntry { ts: now_ms(), msg: "用户取消任务".to_string() });
                return true;
            }
        }
        false
    }

    pub fn is_cancelled(&self, task_id: &str) -> bool {
        self.tasks.read().get(task_id).map(|t| t.cancelled).unwrap_or(false)
    }

    pub fn set_rendering(&self, task_id: &str, total_pages: usize, ocr_pages: usize, text_pages: usize) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.status = TaskStatus::Rendering;
            task.progress.total_pages = total_pages;
            task.progress.ocr_pages = ocr_pages;
            task.progress.text_pages = text_pages;
            task.progress.overall_percent = 5;
            task.progress.stage_label = "解析".to_string();
            
            let msg = if text_pages > 0 {
                format!("共 {} 页：{} 页直接提取，{} 页需要 OCR", total_pages, text_pages, ocr_pages)
            } else {
                format!("共 {} 页，全部需要 OCR", total_pages)
            };
            task.progress.message = msg.clone();
            task.progress.logs.push(LogEntry { ts: now_ms(), msg });
        }
    }

    pub fn set_recognizing(&self, task_id: &str, current: usize, total: usize, is_ocr: bool) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.status = TaskStatus::Recognizing;
            task.progress.current_page = current;
            task.progress.total_pages = total;
            task.progress.stage_label = "识别".to_string();
            
            // Progress: 5% (render) + 45% (recognize) = 5-50%
            let recognize_progress = (current as f32 / total as f32) * 45.0;
            task.progress.overall_percent = (5.0 + recognize_progress) as u8;
            
            let mode = if is_ocr { "OCR" } else { "提取" };
            task.progress.message = format!("正在{}第 {} 页 (共 {} 页)", mode, current, total);
            
            // Calculate ETA
            if !task.page_times.is_empty() {
                let avg_ms: u64 = task.page_times.iter().sum::<u64>() / task.page_times.len() as u64;
                let remaining_pages = (total - current) * 2 + total; // recognize + translate remaining
                task.progress.eta_seconds = Some((remaining_pages as u64 * avg_ms) / 1000);
            }
        }
    }

    pub fn set_translating(&self, task_id: &str, current: usize, total: usize) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            // Record page time for ETA calculation
            let elapsed = now_ms() - task.started_at;
            if task.page_times.len() < current {
                task.page_times.push(elapsed / current as u64);
            }
            
            task.progress.status = TaskStatus::Translating;
            task.progress.current_page = current;
            task.progress.total_pages = total;
            task.progress.stage_label = "翻译".to_string();
            
            // Progress: 50% (recognize done) + 45% (translate) = 50-95%
            let translate_progress = (current as f32 / total as f32) * 45.0;
            task.progress.overall_percent = (50.0 + translate_progress) as u8;
            
            task.progress.message = format!("正在翻译第 {} 页 (共 {} 页)", current, total);
            
            // Update ETA
            if !task.page_times.is_empty() {
                let avg_ms: u64 = task.page_times.iter().sum::<u64>() / task.page_times.len() as u64;
                let remaining = total - current;
                task.progress.eta_seconds = Some((remaining as u64 * avg_ms) / 1000);
            }
        }
    }

    pub fn set_generating(&self, task_id: &str) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.status = TaskStatus::Generating;
            task.progress.overall_percent = 95;
            task.progress.stage_label = "生成".to_string();
            task.progress.message = "正在生成 PDF...".to_string();
            task.progress.eta_seconds = Some(2);
            task.progress.logs.push(LogEntry { ts: now_ms(), msg: "开始生成 PDF".to_string() });
        }
    }

    pub fn set_complete(&self, task_id: &str, pdf_data: Vec<u8>) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            let elapsed = (now_ms() - task.started_at) / 1000;
            task.progress.status = TaskStatus::Complete;
            task.progress.overall_percent = 100;
            task.progress.stage_label = "完成".to_string();
            task.progress.message = format!("翻译完成！用时 {} 秒", elapsed);
            task.progress.eta_seconds = None;
            task.progress.logs.push(LogEntry { ts: now_ms(), msg: format!("完成，总用时 {} 秒", elapsed) });
            task.pdf_data = Some(Arc::new(pdf_data));
        }
    }

    pub fn set_error(&self, task_id: &str, error: String) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.status = TaskStatus::Error;
            task.progress.stage_label = "错误".to_string();
            task.progress.message = error.clone();
            task.progress.eta_seconds = None;
            task.progress.logs.push(LogEntry { ts: now_ms(), msg: format!("错误: {}", error) });
        }
    }

    pub fn add_log(&self, task_id: &str, msg: String) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.logs.push(LogEntry { ts: now_ms(), msg });
            // Keep only last 20 logs
            if task.progress.logs.len() > 20 {
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
}
