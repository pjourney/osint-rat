use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use super::{EVENT_CHANNEL_CAPACITY, JobContext, JobEvent, JobOutcome, JobOutput};

/// How long cancelled work may take to shut down cleanly (killing subprocess groups, ...)
/// before its future is dropped.
const CANCEL_GRACE: Duration = Duration::from_secs(4);

struct RunningJob {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
}

/// Spawns and tracks background jobs.
pub struct JobManager {
    sender: mpsc::Sender<JobEvent>,
    receiver: Option<mpsc::Receiver<JobEvent>>,
    running: HashMap<Uuid, RunningJob>,
    permits: Arc<Semaphore>,
}

impl std::fmt::Debug for JobManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("JobManager")
            .field("running", &self.running.len())
            .finish()
    }
}

impl JobManager {
    pub fn new(max_concurrent: usize) -> Self {
        let (sender, receiver) = mpsc::channel(EVENT_CHANNEL_CAPACITY);
        Self {
            sender,
            receiver: Some(receiver),
            running: HashMap::new(),
            permits: Arc::new(Semaphore::new(max_concurrent.max(1))),
        }
    }

    /// The event receiver; the UI event loop takes it once.
    pub fn take_receiver(&mut self) -> Option<mpsc::Receiver<JobEvent>> {
        self.receiver.take()
    }

    /// Runs `work` in the background. The job waits in the queue until a concurrency permit
    /// is available. `work` should observe `JobContext::cancel` and stop promptly.
    pub fn spawn<F, Fut>(&mut self, job_id: Uuid, work: F)
    where
        F: FnOnce(JobContext) -> Fut + Send + 'static,
        Fut: Future<Output = Result<JobOutput, String>> + Send + 'static,
    {
        let cancel = CancellationToken::new();
        let context = JobContext::attached(job_id, cancel.clone(), self.sender.clone());
        let sender = self.sender.clone();
        let permits = Arc::clone(&self.permits);
        let task_cancel = cancel.clone();
        let handle = tokio::spawn(async move {
            let permit = tokio::select! {
                permit = permits.acquire_owned() => permit.ok(),
                _ = task_cancel.cancelled() => None,
            };
            let outcome = match permit {
                Some(_permit) => {
                    let _ = sender.send(JobEvent::Started { job_id }).await;
                    run_with_cancellation(work(context), &task_cancel).await
                }
                None => JobOutcome::Cancelled,
            };
            let _ = sender.send(JobEvent::Finished { job_id, outcome }).await;
        });
        self.running.insert(job_id, RunningJob { cancel, handle });
    }

    /// Requests cancellation. Returns `false` when the job is not running.
    pub fn cancel(&mut self, job_id: Uuid) -> bool {
        match self.running.get(&job_id) {
            Some(job) => {
                job.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Stops tracking a finished job.
    pub fn finished(&mut self, job_id: Uuid) {
        self.running.remove(&job_id);
    }

    pub fn is_running(&self, job_id: Uuid) -> bool {
        self.running.contains_key(&job_id)
    }

    pub fn running_count(&self) -> usize {
        self.running.len()
    }

    /// Cancels every job and waits (bounded) for them to clean up.
    pub async fn shutdown(&mut self, timeout: Duration) {
        for job in self.running.values() {
            job.cancel.cancel();
        }
        let handles: Vec<JoinHandle<()>> =
            self.running.drain().map(|(_, job)| job.handle).collect();
        let _ = tokio::time::timeout(timeout, futures::future::join_all(handles)).await;
    }
}

async fn run_with_cancellation(
    work: impl Future<Output = Result<JobOutput, String>>,
    cancel: &CancellationToken,
) -> JobOutcome {
    tokio::pin!(work);
    let result = tokio::select! {
        result = &mut work => Some(result),
        _ = cancel.cancelled() => tokio::time::timeout(CANCEL_GRACE, &mut work).await.ok(),
    };
    if cancel.is_cancelled() {
        return JobOutcome::Cancelled;
    }
    match result {
        Some(Ok(output)) => JobOutcome::Succeeded(output),
        Some(Err(error)) => JobOutcome::Failed(error),
        None => JobOutcome::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::ReportOutput;

    fn report_output() -> JobOutput {
        JobOutput::Report(ReportOutput {
            relative_path: "reports/r.md".into(),
            format: "markdown".into(),
            size_bytes: 1,
            sha256: "00".into(),
        })
    }

    async fn next_finished(receiver: &mut mpsc::Receiver<JobEvent>) -> (Uuid, JobOutcome) {
        loop {
            match receiver.recv().await.expect("event") {
                JobEvent::Finished { job_id, outcome } => return (job_id, outcome),
                _ => continue,
            }
        }
    }

    #[tokio::test]
    async fn successful_jobs_report_started_and_finished() {
        let mut manager = JobManager::new(2);
        let mut receiver = manager.take_receiver().expect("receiver");
        let job_id = Uuid::new_v4();
        manager.spawn(job_id, |context| async move {
            context.progress("working");
            Ok(report_output())
        });
        let first = receiver.recv().await.expect("event");
        let (finished_id, outcome) = next_finished(&mut receiver).await;
        assert!(matches!(first, JobEvent::Started { .. }));
        assert_eq!(finished_id, job_id);
        assert!(matches!(
            outcome,
            JobOutcome::Succeeded(JobOutput::Report(_))
        ));
    }

    #[tokio::test]
    async fn failures_are_reported_not_panicked() {
        let mut manager = JobManager::new(1);
        let mut receiver = manager.take_receiver().expect("receiver");
        manager.spawn(Uuid::new_v4(), |_| async {
            Err("HTTP 429: rate limited".to_string())
        });
        let (_, outcome) = next_finished(&mut receiver).await;
        assert!(matches!(outcome, JobOutcome::Failed(message) if message.contains("429")));
    }

    #[tokio::test]
    async fn cancellation_stops_cooperative_work() {
        let mut manager = JobManager::new(1);
        let mut receiver = manager.take_receiver().expect("receiver");
        let job_id = Uuid::new_v4();
        manager.spawn(job_id, |context| async move {
            context.cancel.cancelled().await;
            Err("stopped".to_string())
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(manager.cancel(job_id));
        let (_, outcome) = next_finished(&mut receiver).await;
        assert!(matches!(outcome, JobOutcome::Cancelled));
    }

    #[tokio::test]
    async fn uncooperative_work_is_dropped_after_grace_period() {
        tokio::time::pause();
        let mut manager = JobManager::new(1);
        let mut receiver = manager.take_receiver().expect("receiver");
        let job_id = Uuid::new_v4();
        manager.spawn(job_id, |_| async {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            Ok(report_output())
        });
        tokio::time::sleep(Duration::from_millis(10)).await;
        manager.cancel(job_id);
        let (_, outcome) = next_finished(&mut receiver).await;
        assert!(matches!(outcome, JobOutcome::Cancelled));
    }

    #[tokio::test]
    async fn queued_jobs_wait_for_permits_and_can_be_cancelled() {
        let mut manager = JobManager::new(1);
        let mut receiver = manager.take_receiver().expect("receiver");
        let blocker = Uuid::new_v4();
        let queued = Uuid::new_v4();
        manager.spawn(blocker, |context| async move {
            context.cancel.cancelled().await;
            Err("stopped".to_string())
        });
        manager.spawn(queued, |_| async { Ok(report_output()) });
        tokio::time::sleep(Duration::from_millis(50)).await;
        manager.cancel(queued);
        let (finished_id, outcome) = next_finished(&mut receiver).await;
        assert_eq!(finished_id, queued);
        assert!(matches!(outcome, JobOutcome::Cancelled));
        manager.shutdown(Duration::from_secs(1)).await;
    }
}
