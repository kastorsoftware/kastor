pub mod validate;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, Semaphore};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: String,
    pub kind: String,
    pub description: String,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    Queued,
    Running,
    Done,
    Failed,
    Stopped,
}

const MAX_HISTORY: usize = 50;

pub struct TaskQueue {
    semaphore: Arc<Semaphore>,
    tasks: Arc<Mutex<Vec<TaskInfo>>>,
    cancellation: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
}

impl TaskQueue {
    pub fn new(concurrency: usize) -> Self {
        let history = load_task_history();
        Self {
            semaphore: Arc::new(Semaphore::new(concurrency)),
            tasks: Arc::new(Mutex::new(history)),
            cancellation: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    // enqueue a task with semaphore slot (for checker, validate, proxy)
    pub fn enqueue<F, Fut>(&self, id: String, kind: String, description: String, work: F)
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
    {
        let info = TaskInfo {
            id: id.clone(),
            kind,
            description,
            status: TaskStatus::Queued,
        };

        let tasks = Arc::clone(&self.tasks);
        let sem = Arc::clone(&self.semaphore);

        let tasks_clone = Arc::clone(&tasks);
        tokio::spawn(async move {
            {
                let mut list = tasks_clone.lock().await;
                list.push(info);
            }

            let _permit = sem.acquire().await.unwrap();

            {
                let mut list = tasks_clone.lock().await;
                if let Some(t) = list.iter_mut().find(|t| t.id == id) {
                    t.status = TaskStatus::Running;
                }
            }

            let result = work().await;

            {
                let mut list = tasks_clone.lock().await;
                if let Some(t) = list.iter_mut().find(|t| t.id == id) {
                    t.status = if result.is_ok() { TaskStatus::Done } else { TaskStatus::Failed };
                }
                trim_history(&mut list);
                save_task_history(&list);
            }
        });
    }

    // register a task that runs immediately (no semaphore) and get a cancellation token
    pub async fn register_task(&self, id: String, kind: String, description: String) -> Arc<AtomicBool> {
        let token = Arc::new(AtomicBool::new(true)); // true = running
        {
            let mut cancel_map = self.cancellation.lock().await;
            cancel_map.insert(id.clone(), token.clone());
        }
        {
            let mut list = self.tasks.lock().await;
            list.push(TaskInfo {
                id,
                kind,
                description,
                status: TaskStatus::Running,
            });
        }
        token
    }

    // mark task as done/failed/stopped
    pub async fn finish_task(&self, id: &str, success: bool) {
        {
            let mut list = self.tasks.lock().await;
            if let Some(t) = list.iter_mut().find(|t| t.id == id) {
                let was_stopped = {
                    let cancel_map = self.cancellation.lock().await;
                    cancel_map.get(id).map(|t| !t.load(Ordering::Relaxed)).unwrap_or(false)
                };
                t.status = if was_stopped {
                    TaskStatus::Stopped
                } else if success {
                    TaskStatus::Done
                } else {
                    TaskStatus::Failed
                };
            }
            trim_history(&mut list);
            save_task_history(&list);
        }
        {
            let mut cancel_map = self.cancellation.lock().await;
            cancel_map.remove(id);
        }
    }

    // stop a task by id
    pub async fn stop_task(&self, id: &str) -> bool {
        let cancel_map = self.cancellation.lock().await;
        if let Some(token) = cancel_map.get(id) {
            token.store(false, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    #[allow(dead_code)]
    pub async fn is_running(&self, id: &str) -> bool {
        let cancel_map = self.cancellation.lock().await;
        cancel_map.get(id).map(|t| t.load(Ordering::Relaxed)).unwrap_or(false)
    }

    pub async fn get_tasks(&self) -> Vec<TaskInfo> {
        self.tasks.lock().await.clone()
    }

    pub async fn queue_size(&self) -> u32 {
        let list = self.tasks.lock().await;
        list.iter().filter(|t| t.status == TaskStatus::Queued).count() as u32
    }

    pub async fn running_count(&self) -> u32 {
        let list = self.tasks.lock().await;
        list.iter().filter(|t| t.status == TaskStatus::Running).count() as u32
    }
}

fn history_path() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("kastor")
        .join("task_history.json")
}

fn load_task_history() -> Vec<TaskInfo> {
    let path = history_path();
    if !path.exists() {
        return Vec::new();
    }
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let mut tasks: Vec<TaskInfo> = serde_json::from_str(&content).unwrap_or_default();
            for t in tasks.iter_mut() {
                if t.status == TaskStatus::Running || t.status == TaskStatus::Queued {
                    t.status = TaskStatus::Failed;
                }
            }
            tasks
        }
        Err(_) => Vec::new(),
    }
}

fn save_task_history(tasks: &[TaskInfo]) {
    let path = history_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    if let Ok(json) = serde_json::to_string(tasks) {
        std::fs::write(&path, json).ok();
    }
}

fn trim_history(tasks: &mut Vec<TaskInfo>) {
    let active: Vec<TaskInfo> = tasks.iter()
        .filter(|t| t.status == TaskStatus::Queued || t.status == TaskStatus::Running)
        .cloned()
        .collect();
    let mut completed: Vec<TaskInfo> = tasks.iter()
        .filter(|t| t.status != TaskStatus::Queued && t.status != TaskStatus::Running)
        .cloned()
        .collect();

    if completed.len() > MAX_HISTORY {
        completed = completed.split_off(completed.len() - MAX_HISTORY);
    }

    tasks.clear();
    tasks.extend(completed);
    tasks.extend(active);
}

// global stop_task command
#[tauri::command]
pub async fn stop_task(task_id: String, queue: tauri::State<'_, TaskQueue>) -> Result<bool, String> {
    Ok(queue.stop_task(&task_id).await)
}
