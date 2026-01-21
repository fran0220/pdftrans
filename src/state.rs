use parking_lot::RwLock;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;

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
pub struct TaskProgress {
    pub status: TaskStatus,
    pub current_page: usize,
    pub total_pages: usize,
    pub message: String,
}

impl TaskProgress {
    pub fn is_done(&self) -> bool {
        matches!(self.status, TaskStatus::Complete | TaskStatus::Error)
    }
}

pub struct TaskData {
    pub progress: TaskProgress,
    pub pdf_data: Option<Arc<Vec<u8>>>,
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
        let task = TaskData {
            progress: TaskProgress {
                status: TaskStatus::Rendering,
                current_page: 0,
                total_pages: 0,
                message: "正在渲染 PDF...".to_string(),
            },
            pdf_data: None,
        };
        self.tasks.write().insert(task_id.to_string(), task);
    }

    pub fn set_rendering(&self, task_id: &str, total_pages: usize) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.status = TaskStatus::Rendering;
            task.progress.total_pages = total_pages;
            task.progress.message = format!("正在渲染 PDF ({} 页)...", total_pages);
        }
    }

    pub fn set_recognizing(&self, task_id: &str, current: usize, total: usize) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.status = TaskStatus::Recognizing;
            task.progress.current_page = current;
            task.progress.total_pages = total;
            task.progress.message = format!("正在识别第 {} 页 (共 {} 页)...", current, total);
        }
    }

    pub fn set_translating(&self, task_id: &str, current: usize, total: usize) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.status = TaskStatus::Translating;
            task.progress.current_page = current;
            task.progress.total_pages = total;
            task.progress.message = format!("正在翻译第 {} 页 (共 {} 页)...", current, total);
        }
    }

    pub fn set_generating(&self, task_id: &str) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.status = TaskStatus::Generating;
            task.progress.message = "正在生成 PDF...".to_string();
        }
    }

    pub fn set_complete(&self, task_id: &str, pdf_data: Vec<u8>) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.status = TaskStatus::Complete;
            task.progress.message = "翻译完成！".to_string();
            task.pdf_data = Some(Arc::new(pdf_data));
        }
    }

    pub fn set_error(&self, task_id: &str, error: String) {
        if let Some(task) = self.tasks.write().get_mut(task_id) {
            task.progress.status = TaskStatus::Error;
            task.progress.message = error;
        }
    }

    pub fn get_progress(&self, task_id: &str) -> Option<TaskProgress> {
        self.tasks.read().get(task_id).map(|t| t.progress.clone())
    }

    pub fn get_pdf_data(&self, task_id: &str) -> Option<Arc<Vec<u8>>> {
        self.tasks.read().get(task_id).and_then(|t| t.pdf_data.clone())
    }
}
