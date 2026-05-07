use crate::job::{Job, JobStatus};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{Mutex, broadcast};
use uuid::Uuid;

/// Sent to all subscribers (TUI, IPC) whenever the queue changes.
#[derive(Debug, Clone)]
pub enum QueueEvent {
    JobAdded(Job),
    JobUpdated(Job),
}

#[derive(Debug)]
pub struct Queue {
    jobs: Mutex<VecDeque<Job>>,
    tx: broadcast::Sender<QueueEvent>,
}

impl Queue {
    pub fn new() -> Arc<Self> {
        let (tx, _) = broadcast::channel(64);
        Arc::new(Self {
            jobs: Mutex::new(VecDeque::new()),
            tx,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<QueueEvent> {
        self.tx.subscribe()
    }

    pub async fn push(&self, job: Job) {
        let mut jobs = self.jobs.lock().await;
        let _ = self.tx.send(QueueEvent::JobAdded(job.clone()));
        jobs.push_back(job);
    }

    /// Returns the next queued job, marking it as Running.
    pub async fn pop_next(&self) -> Option<Job> {
        let mut jobs = self.jobs.lock().await;
        let job = jobs.iter_mut().find(|j| j.status == JobStatus::Queued)?;
        job.status = JobStatus::Running;
        let job = job.clone();
        let _ = self.tx.send(QueueEvent::JobUpdated(job.clone()));
        Some(job)
    }

    pub async fn update(&self, id: Uuid, f: impl FnOnce(&mut Job)) {
        let mut jobs = self.jobs.lock().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            f(job);
            let _ = self.tx.send(QueueEvent::JobUpdated(job.clone()));
        }
    }

    pub async fn cancel(&self, id: Uuid) -> bool {
        let mut jobs = self.jobs.lock().await;
        if let Some(job) = jobs.iter_mut().find(|j| j.id == id) {
            if !job.status.is_terminal() {
                job.status = JobStatus::Cancelled;
                let _ = self.tx.send(QueueEvent::JobUpdated(job.clone()));
                return true;
            }
        }
        false
    }

    pub async fn snapshot(&self) -> Vec<Job> {
        self.jobs.lock().await.iter().cloned().collect()
    }
}
