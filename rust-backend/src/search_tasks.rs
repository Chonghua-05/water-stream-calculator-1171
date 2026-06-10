use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchTaskStatus {
    Queued,
    Running,
    Completed,
    Cancelled,
    Failed,
}

impl SearchTaskStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

impl Display for SearchTaskStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        };
        write!(f, "{text}")
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SearchTaskProgress {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passing_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unique_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expanded_states: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_workers: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub written: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_total: Option<u64>,
    #[serde(flatten, default)]
    pub extra: BTreeMap<String, Value>,
}

impl SearchTaskProgress {
    pub fn queued() -> Self {
        Self {
            stage: Some("queued".to_string()),
            message: Some("queued in Rust backend".to_string()),
            checked: Some(0),
            total: None,
            percent: Some(0.0),
            ..Self::default()
        }
    }

    pub fn running() -> Self {
        Self {
            stage: Some("running".to_string()),
            message: Some("search task is running".to_string()),
            checked: Some(0),
            total: None,
            percent: Some(0.0),
            ..Self::default()
        }
    }

    pub fn merge_patch(&mut self, patch: SearchTaskProgress) {
        macro_rules! apply_if_some {
            ($field:ident) => {
                if patch.$field.is_some() {
                    self.$field = patch.$field;
                }
            };
        }

        apply_if_some!(stage);
        apply_if_some!(message);
        apply_if_some!(checked);
        apply_if_some!(total);
        apply_if_some!(percent);
        apply_if_some!(candidate_count);
        apply_if_some!(passing_count);
        apply_if_some!(unique_count);
        apply_if_some!(expanded_states);
        apply_if_some!(bucket_count);
        apply_if_some!(parallel_workers);
        apply_if_some!(written);
        apply_if_some!(write_total);
        self.extra.extend(patch.extra);
        if patch.percent.is_none() {
            self.recompute_percent();
        }
    }

    fn recompute_percent(&mut self) {
        if let (Some(checked), Some(total)) = (self.checked, self.total)
            && total > 0
        {
            let percent = ((checked as f64 / total as f64) * 1000.0).round() / 10.0;
            self.percent = Some(percent.clamp(0.0, 100.0));
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublicSearchTask {
    #[serde(rename = "task_id")]
    pub task_id: String,
    pub status: SearchTaskStatus,
    #[serde(rename = "started_at")]
    pub started_at: String,
    #[serde(rename = "updated_at")]
    pub updated_at: String,
    #[serde(rename = "finished_at", default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub progress: SearchTaskProgress,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing)]
    pub cancel_requested: bool,
}

impl PublicSearchTask {
    pub fn queued(task_id: impl Into<String>) -> Self {
        let now = timestamp_now();
        Self {
            task_id: task_id.into(),
            status: SearchTaskStatus::Queued,
            started_at: now.clone(),
            updated_at: now,
            finished_at: None,
            progress: SearchTaskProgress::queued(),
            result: None,
            error: None,
            cancel_requested: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchTaskEnvelope {
    pub ok: bool,
    pub task: PublicSearchTask,
}

#[derive(Clone, Debug)]
pub enum SearchTaskError {
    Cancelled {
        message: Option<String>,
        result: Option<Value>,
    },
    Failed(String),
}

impl SearchTaskError {
    pub fn cancelled() -> Self {
        Self::Cancelled {
            message: None,
            result: None,
        }
    }

    pub fn cancelled_with_message(message: impl Into<String>) -> Self {
        Self::Cancelled {
            message: Some(message.into()),
            result: None,
        }
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }
}

impl Display for SearchTaskError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled { message, .. } => {
                write!(f, "{}", message.as_deref().unwrap_or("search task cancelled"))
            }
            Self::Failed(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for SearchTaskError {}

impl From<String> for SearchTaskError {
    fn from(value: String) -> Self {
        Self::Failed(value)
    }
}

impl From<&str> for SearchTaskError {
    fn from(value: &str) -> Self {
        Self::Failed(value.to_string())
    }
}

#[derive(Clone, Default)]
pub struct SearchTaskManager {
    inner: Arc<SearchTaskManagerInner>,
}

#[derive(Default)]
struct SearchTaskManagerInner {
    next_id: AtomicU64,
    tasks: RwLock<BTreeMap<String, Arc<SearchTaskState>>>,
}

impl SearchTaskManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start<F>(&self, request: Value, worker: F) -> SearchTaskEnvelope
    where
        F: FnOnce(SearchTaskContext, Value) -> Result<Value, SearchTaskError> + Send + 'static,
    {
        let task_id = self.next_task_id();
        let state = Arc::new(SearchTaskState::queued(task_id.clone()));
        self.inner
            .tasks
            .write()
            .expect("search task write lock poisoned")
            .insert(task_id, Arc::clone(&state));

        let context = SearchTaskContext { state };
        let snapshot = context.snapshot();
        thread::spawn(move || {
            context.run(worker, request);
        });

        SearchTaskEnvelope {
            ok: true,
            task: snapshot,
        }
    }

    pub fn get(&self, task_id: &str) -> Option<SearchTaskEnvelope> {
        self.task_state(task_id).map(|state| SearchTaskEnvelope {
            ok: true,
            task: state.snapshot(),
        })
    }

    pub fn cancel(&self, task_id: &str) -> Option<SearchTaskEnvelope> {
        let state = self.task_state(task_id)?;
        let task = state.request_cancel();
        Some(SearchTaskEnvelope { ok: true, task })
    }

    pub fn cleanup_finished_older_than(&self, max_age: Duration) -> usize {
        let cutoff = Instant::now()
            .checked_sub(max_age)
            .unwrap_or_else(Instant::now);
        let mut tasks = self
            .inner
            .tasks
            .write()
            .expect("search task write lock poisoned");
        let before = tasks.len();
        tasks.retain(|_, task| {
            task.finished_at_instant()
                .map(|finished| finished >= cutoff)
                .unwrap_or(true)
        });
        before.saturating_sub(tasks.len())
    }

    fn next_task_id(&self) -> String {
        let next = self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        format!("{next:012x}")
    }

    fn task_state(&self, task_id: &str) -> Option<Arc<SearchTaskState>> {
        self.inner
            .tasks
            .read()
            .expect("search task read lock poisoned")
            .get(task_id)
            .cloned()
    }
}

#[derive(Clone)]
pub struct SearchTaskContext {
    state: Arc<SearchTaskState>,
}

impl SearchTaskContext {
    pub fn task_id(&self) -> String {
        self.state.snapshot().task_id
    }

    pub fn snapshot(&self) -> PublicSearchTask {
        self.state.snapshot()
    }

    pub fn should_cancel(&self) -> bool {
        self.state.cancel_requested.load(Ordering::SeqCst)
    }

    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.state.cancel_requested
    }

    pub fn is_terminal(&self) -> bool {
        self.snapshot().status.is_terminal()
    }

    pub fn check_cancelled(&self) -> Result<(), SearchTaskError> {
        if self.should_cancel() {
            Err(SearchTaskError::cancelled())
        } else {
            Ok(())
        }
    }

    pub fn mark_running(&self) -> PublicSearchTask {
        self.state.mark_running()
    }

    pub fn update_progress(&self, patch: SearchTaskProgress) -> PublicSearchTask {
        self.state.update_progress(patch)
    }

    pub fn complete(&self, result: Value) -> PublicSearchTask {
        self.state.complete(result)
    }

    pub fn fail(&self, error: impl Into<String>) -> PublicSearchTask {
        self.state.fail(error.into())
    }

    pub fn cancel(
        &self,
        message: Option<String>,
        result: Option<Value>,
    ) -> PublicSearchTask {
        self.state.finish_cancelled(message, result)
    }

    fn run<F>(&self, worker: F, request: Value)
    where
        F: FnOnce(SearchTaskContext, Value) -> Result<Value, SearchTaskError> + Send + 'static,
    {
        if self.snapshot().status.is_terminal() {
            return;
        }
        if self.should_cancel() {
            self.cancel(None, None);
            return;
        }
        self.mark_running();

        let outcome = catch_unwind(AssertUnwindSafe(|| worker(self.clone(), request)));
        match outcome {
            Ok(Ok(result)) => {
                if !self.is_terminal() {
                    self.complete(result);
                }
            }
            Ok(Err(SearchTaskError::Cancelled { message, result })) => {
                self.cancel(message, result);
            }
            Ok(Err(SearchTaskError::Failed(message))) => {
                self.fail(message);
            }
            Err(payload) => {
                self.fail(panic_message(payload));
            }
        }
    }
}

struct SearchTaskState {
    snapshot: Mutex<PublicSearchTask>,
    finished_at_instant: Mutex<Option<Instant>>,
    cancel_requested: AtomicBool,
}

impl SearchTaskState {
    fn queued(task_id: String) -> Self {
        Self {
            snapshot: Mutex::new(PublicSearchTask::queued(task_id)),
            finished_at_instant: Mutex::new(None),
            cancel_requested: AtomicBool::new(false),
        }
    }

    fn snapshot(&self) -> PublicSearchTask {
        let mut task = self
            .snapshot
            .lock()
            .expect("search task snapshot lock poisoned")
            .clone();
        task.cancel_requested = self.cancel_requested.load(Ordering::SeqCst);
        task
    }

    fn finished_at_instant(&self) -> Option<Instant> {
        *self
            .finished_at_instant
            .lock()
            .expect("search task instant lock poisoned")
    }

    fn request_cancel(&self) -> PublicSearchTask {
        self.cancel_requested.store(true, Ordering::SeqCst);
        let mut task = self
            .snapshot
            .lock()
            .expect("search task snapshot lock poisoned");
        task.cancel_requested = true;
        task.updated_at = timestamp_now();
        if task.status == SearchTaskStatus::Queued {
            task.status = SearchTaskStatus::Cancelled;
            task.progress = SearchTaskProgress {
                stage: Some("cancelled".to_string()),
                message: Some("search task cancelled before start".to_string()),
                checked: Some(0),
                total: None,
                percent: Some(0.0),
                ..SearchTaskProgress::default()
            };
            self.set_finished_locked(&mut task);
        } else if !task.status.is_terminal() {
            task.progress.merge_patch(SearchTaskProgress {
                message: Some("cancel requested".to_string()),
                ..SearchTaskProgress::default()
            });
        }
        task.clone()
    }

    fn mark_running(&self) -> PublicSearchTask {
        let mut task = self
            .snapshot
            .lock()
            .expect("search task snapshot lock poisoned");
        if task.status.is_terminal() {
            return task.clone();
        }
        task.status = SearchTaskStatus::Running;
        task.updated_at = timestamp_now();
        task.progress.merge_patch(SearchTaskProgress::running());
        task.clone()
    }

    fn update_progress(&self, patch: SearchTaskProgress) -> PublicSearchTask {
        let mut task = self
            .snapshot
            .lock()
            .expect("search task snapshot lock poisoned");
        if task.status.is_terminal() {
            return task.clone();
        }
        task.updated_at = timestamp_now();
        task.progress.merge_patch(patch);
        task.clone()
    }

    fn complete(&self, result: Value) -> PublicSearchTask {
        let mut task = self
            .snapshot
            .lock()
            .expect("search task snapshot lock poisoned");
        if task.status.is_terminal() {
            return task.clone();
        }
        task.status = SearchTaskStatus::Completed;
        task.result = Some(result);
        task.error = None;
        task.progress.merge_patch(SearchTaskProgress {
            stage: Some("completed".to_string()),
            message: Some("search task completed".to_string()),
            percent: Some(100.0),
            ..SearchTaskProgress::default()
        });
        self.set_finished_locked(&mut task);
        task.clone()
    }

    fn fail(&self, error: String) -> PublicSearchTask {
        let mut task = self
            .snapshot
            .lock()
            .expect("search task snapshot lock poisoned");
        if task.status.is_terminal() {
            return task.clone();
        }
        task.status = SearchTaskStatus::Failed;
        task.error = Some(error);
        task.progress.merge_patch(SearchTaskProgress {
            stage: Some("failed".to_string()),
            message: Some("search task failed".to_string()),
            ..SearchTaskProgress::default()
        });
        self.set_finished_locked(&mut task);
        task.clone()
    }

    fn finish_cancelled(
        &self,
        message: Option<String>,
        result: Option<Value>,
    ) -> PublicSearchTask {
        let mut task = self
            .snapshot
            .lock()
            .expect("search task snapshot lock poisoned");
        if task.status.is_terminal() {
            return task.clone();
        }
        self.cancel_requested.store(true, Ordering::SeqCst);
        task.cancel_requested = true;
        task.status = SearchTaskStatus::Cancelled;
        task.result = Some(result.unwrap_or_else(|| json!({ "ok": false, "cancelled": true })));
        task.error = None;
        task.progress.merge_patch(SearchTaskProgress {
            stage: Some("cancelled".to_string()),
            message: Some(message.unwrap_or_else(|| "search task cancelled".to_string())),
            ..SearchTaskProgress::default()
        });
        self.set_finished_locked(&mut task);
        task.clone()
    }

    fn set_finished_locked(&self, task: &mut PublicSearchTask) {
        let now = timestamp_now();
        task.updated_at = now.clone();
        task.finished_at = Some(now);
        *self
            .finished_at_instant
            .lock()
            .expect("search task instant lock poisoned") = Some(Instant::now());
    }
}

fn panic_message(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "search task panicked".to_string()
    }
}

fn timestamp_now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn wait_for_status(
        manager: &SearchTaskManager,
        task_id: &str,
        expected: SearchTaskStatus,
    ) -> PublicSearchTask {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let task = manager
                .get(task_id)
                .expect("task should exist")
                .task;
            if task.status == expected {
                return task;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for task {task_id} to reach status {expected}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn search_tasks_start_and_complete() {
        let manager = SearchTaskManager::new();
        let (started_tx, started_rx) = mpsc::channel();
        let envelope = manager.start(json!({ "ticks": 800 }), move |ctx, request| {
            let _ = ctx.update_progress(SearchTaskProgress {
                stage: Some("scanning".to_string()),
                message: Some("scanning candidates".to_string()),
                checked: Some(5),
                total: Some(10),
                ..SearchTaskProgress::default()
            });
            started_tx.send(()).expect("send running signal");
            Ok(json!({ "ok": true, "request": request }))
        });

        assert_eq!(envelope.task.status, SearchTaskStatus::Queued);
        started_rx.recv_timeout(Duration::from_secs(1)).expect("worker should run");
        let task = wait_for_status(&manager, &envelope.task.task_id, SearchTaskStatus::Completed);
        assert_eq!(task.progress.stage.as_deref(), Some("completed"));
        assert_eq!(task.progress.percent, Some(100.0));
        assert_eq!(task.result.as_ref().and_then(|value| value.get("ok")).and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn search_tasks_cancel_running_task() {
        let manager = SearchTaskManager::new();
        let (started_tx, started_rx) = mpsc::channel();
        let envelope = manager.start(json!({}), move |ctx, _request| {
            started_tx.send(()).expect("send running signal");
            loop {
                if ctx.should_cancel() {
                    return Err(SearchTaskError::cancelled_with_message(
                        "cancelled from test worker",
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
        });

        started_rx.recv_timeout(Duration::from_secs(1)).expect("worker should run");
        let cancelled = manager
            .cancel(&envelope.task.task_id)
            .expect("task should exist")
            .task;
        assert!(cancelled.cancel_requested);

        let task = wait_for_status(&manager, &envelope.task.task_id, SearchTaskStatus::Cancelled);
        assert_eq!(task.progress.stage.as_deref(), Some("cancelled"));
        assert_eq!(
            task.result
                .as_ref()
                .and_then(|value| value.get("cancelled"))
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn search_tasks_mark_failed_task() {
        let manager = SearchTaskManager::new();
        let envelope = manager.start(json!({}), move |_ctx, _request| {
            Err(SearchTaskError::failed("boom"))
        });

        let task = wait_for_status(&manager, &envelope.task.task_id, SearchTaskStatus::Failed);
        assert_eq!(task.progress.stage.as_deref(), Some("failed"));
        assert_eq!(task.error.as_deref(), Some("boom"));
        assert!(task.finished_at.is_some());
    }
}
