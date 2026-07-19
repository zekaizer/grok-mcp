//! In-memory async jobs for long tool calls (timeout_secs + poll via job_status).

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::Semaphore;

use crate::envelope::{ErrorCode, Fail};

/// Max concurrent background jobs actually calling xAI (SuperGrok quota protection).
pub const MAX_INFLIGHT: usize = 10;

/// Max jobs allowed to wait for an inflight slot. Beyond `MAX_INFLIGHT + MAX_QUEUED`
/// admission is refused with a retryable `RATE_LIMITED` (bounds quota and memory).
pub const MAX_QUEUED: usize = 20;

/// Keep finished jobs this long for polling.
pub const JOB_TTL: Duration = Duration::from_secs(30 * 60);

/// Clamp for `timeout_secs` tool args (seconds).
pub const TIMEOUT_SECS_MIN: u32 = 1;
pub const TIMEOUT_SECS_MAX: u32 = 300;

/// Default offload window when `timeout_secs` is omitted. Kept comfortably under
/// typical MCP client request timeouts (~60s) so calls offload to a job rather
/// than blocking until the client gives up.
pub const DEFAULT_TIMEOUT_SECS: u32 = 25;

static JOB_SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Research,
    AskGrok,
    XSearch,
}

impl JobKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Research => "research",
            Self::AskGrok => "ask_grok",
            Self::XSearch => "x_search",
        }
    }
}

#[derive(Debug, Clone)]
pub enum JobState {
    /// Admitted, waiting for an inflight slot (in the queue).
    Queued,
    Running,
    Completed(Value),
    Failed {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone)]
struct JobEntry {
    kind: JobKind,
    created: Instant,
    finished: Option<Instant>,
    state: JobState,
}

/// Shared job registry (cheap to clone).
#[derive(Clone)]
pub struct JobStore {
    inner: Arc<Mutex<HashMap<String, JobEntry>>>,
    /// Permits = `MAX_INFLIGHT`; a job holds one for the duration of its xAI call.
    sem: Arc<Semaphore>,
}

impl Default for JobStore {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            sem: Arc::new(Semaphore::new(MAX_INFLIGHT)),
        }
    }
}

impl JobStore {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn purge_locked(map: &mut HashMap<String, JobEntry>) {
        let now = Instant::now();
        map.retain(|_, e| match e.finished {
            Some(done) => now.duration_since(done) < JOB_TTL,
            None => now.duration_since(e.created) < JOB_TTL + Duration::from_secs(300),
        });
    }

    /// Jobs occupying the system: queued (waiting) or running (holding a permit).
    fn active_locked(map: &HashMap<String, JobEntry>) -> usize {
        map.values()
            .filter(|e| matches!(e.state, JobState::Queued | JobState::Running))
            .count()
    }

    /// Admit a job into the queue. Returns a `job_id` immediately (state `Queued`);
    /// the caller must then acquire a permit via [`Self::semaphore`] before running.
    /// Errors with retryable `RATE_LIMITED` only when the queue itself is full.
    pub fn admit(&self, kind: JobKind) -> Result<String, Fail> {
        let mut map = self.inner.lock().expect("job store lock");
        Self::purge_locked(&mut map);
        if Self::active_locked(&map) >= MAX_INFLIGHT + MAX_QUEUED {
            return Err(Fail::new(
                ErrorCode::RateLimited,
                format!(
                    "too many jobs (max {MAX_INFLIGHT} running + {MAX_QUEUED} queued); \
                     poll job_status or retry"
                ),
                true,
            ));
        }
        let id = format!("job_{}", JOB_SEQ.fetch_add(1, Ordering::Relaxed));
        map.insert(
            id.clone(),
            JobEntry {
                kind,
                created: Instant::now(),
                finished: None,
                state: JobState::Queued,
            },
        );
        Ok(id)
    }

    /// Transition a queued job to running once it holds a permit.
    pub fn mark_running(&self, id: &str) {
        let mut map = self.inner.lock().expect("job store lock");
        if let Some(e) = map.get_mut(id)
            && matches!(e.state, JobState::Queued)
        {
            e.state = JobState::Running;
        }
    }

    /// Concurrency limiter shared across all jobs (permits = `MAX_INFLIGHT`).
    #[must_use]
    fn semaphore(&self) -> Arc<Semaphore> {
        self.sem.clone()
    }

    pub fn complete_json(&self, id: &str, value: Value) {
        let mut map = self.inner.lock().expect("job store lock");
        if let Some(e) = map.get_mut(id) {
            e.state = JobState::Completed(value);
            e.finished = Some(Instant::now());
        }
    }

    pub fn fail(&self, id: &str, code: &str, message: impl Into<String>) {
        let mut map = self.inner.lock().expect("job store lock");
        if let Some(e) = map.get_mut(id) {
            e.state = JobState::Failed {
                code: code.to_string(),
                message: message.into(),
            };
            e.finished = Some(Instant::now());
        }
    }

    pub fn get(&self, id: &str) -> Option<JobSnapshot> {
        let mut map = self.inner.lock().expect("job store lock");
        Self::purge_locked(&mut map);
        let e = map.get(id)?;
        Some(JobSnapshot {
            job_id: id.to_string(),
            kind: e.kind,
            status: match &e.state {
                JobState::Queued => "queued",
                JobState::Running => "running",
                JobState::Completed(_) => "completed",
                JobState::Failed { .. } => "failed",
            }
            .to_string(),
            elapsed_secs: e.created.elapsed().as_secs(),
            result: match &e.state {
                JobState::Completed(v) => Some(v.clone()),
                _ => None,
            },
            error_code: match &e.state {
                JobState::Failed { code, .. } => Some(code.clone()),
                _ => None,
            },
            error_message: match &e.state {
                JobState::Failed { message, .. } => Some(message.clone()),
                _ => None,
            },
        })
    }
}

#[derive(Debug, Clone)]
pub struct JobSnapshot {
    pub job_id: String,
    pub kind: JobKind,
    pub status: String,
    pub elapsed_secs: u64,
    pub result: Option<Value>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

/// Outcome of a tool call that may defer to a job.
#[derive(Debug)]
pub enum RunOutcome<T> {
    Completed(T),
    Running {
        job_id: String,
        elapsed_secs: u64,
        /// Job state at hand-off: `"running"` (executing) or `"queued"` (waiting for a slot).
        status: String,
    },
}

/// Validate and clamp an explicit timeout, if any. `None` stays `None` here;
/// [`effective_timeout_secs`] applies the default.
pub fn parse_timeout_secs(raw: Option<u32>) -> Result<Option<u32>, Fail> {
    match raw {
        None => Ok(None),
        Some(0) => Err(Fail::new(
            ErrorCode::InvalidParams,
            "timeout_secs must be >= 1 when set (omit to use the default offload window)",
            false,
        )),
        Some(n) => Ok(Some(n.clamp(TIMEOUT_SECS_MIN, TIMEOUT_SECS_MAX))),
    }
}

/// Resolve the offload window actually used: an explicit (validated, clamped)
/// `timeout_secs`, or [`DEFAULT_TIMEOUT_SECS`] when omitted. Async is the default —
/// work still returns inline via `Completed` when it finishes within the window.
pub fn effective_timeout_secs(raw: Option<u32>) -> Result<u32, Fail> {
    Ok(parse_timeout_secs(raw)?.unwrap_or(DEFAULT_TIMEOUT_SECS))
}

/// Run `work` to completion inline, or return `Running` after the offload window
/// (explicit `timeout_secs`, else [`DEFAULT_TIMEOUT_SECS`]) while work continues as a job.
pub async fn run_with_timeout<T, F, Fut>(
    store: &JobStore,
    kind: JobKind,
    timeout_secs: Option<u32>,
    work: F,
) -> Result<RunOutcome<T>, Fail>
where
    T: Serialize + DeserializeOwned + Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<T, Fail>> + Send + 'static,
{
    let secs = effective_timeout_secs(timeout_secs)?;

    {
        let job_id = store.admit(kind)?;
        let store_bg = store.clone();
        let jid = job_id.clone();
        let sem = store.semaphore();
        let handle = tokio::spawn(async move {
            // Wait for an inflight slot — this await is the queue.
            let _permit = match sem.acquire_owned().await {
                Ok(p) => p,
                Err(_) => {
                    store_bg.fail(&jid, "UPSTREAM_ERROR", "job scheduler shut down");
                    return;
                }
            };
            store_bg.mark_running(&jid);
            match work().await {
                Ok(val) => match serde_json::to_value(&val) {
                    Ok(json) => store_bg.complete_json(&jid, json),
                    Err(e) => store_bg.fail(&jid, "UPSTREAM_ERROR", e.to_string()),
                },
                Err(fail) => store_bg.fail(&jid, fail.error.code.as_str(), fail.error.message),
            }
            // Permit dropped here — the next queued job proceeds.
        });

        let sleep = tokio::time::sleep(Duration::from_secs(u64::from(secs)));
        tokio::pin!(sleep);

        tokio::select! {
            biased;
            join = handle => {
                match join {
                    Ok(()) => {
                        // Finished within timeout — read result from store.
                        match store.get(&job_id) {
                            Some(snap) if snap.status == "completed" => {
                                let val: T = serde_json::from_value(
                                    snap.result.unwrap_or(Value::Null),
                                )
                                .map_err(|e| {
                                    Fail::new(
                                        ErrorCode::UpstreamError,
                                        format!("job result decode: {e}"),
                                        false,
                                    )
                                })?;
                                Ok(RunOutcome::Completed(val))
                            }
                            Some(snap) if snap.status == "failed" => Err(Fail::new(
                                ErrorCode::UpstreamError,
                                snap.error_message.unwrap_or_else(|| "job failed".into()),
                                false,
                            )),
                            _ => Err(Fail::new(
                                ErrorCode::UpstreamError,
                                "job finished in unknown state",
                                false,
                            )),
                        }
                    }
                    Err(e) => {
                        store.fail(&job_id, "UPSTREAM_ERROR", format!("task join: {e}"));
                        Err(Fail::new(
                            ErrorCode::UpstreamError,
                            format!("background task failed: {e}"),
                            false,
                        ))
                    }
                }
            }
            () = &mut sleep => {
                // Report whether the job is actually executing or still queued.
                let status = store
                    .get(&job_id)
                    .map_or_else(|| "running".to_string(), |s| s.status);
                Ok(RunOutcome::Running {
                    job_id,
                    elapsed_secs: u64::from(secs),
                    status,
                })
            }
        }
    }
}

/// Hint for hosts after a deferred response, phrased for the job's current state.
#[must_use]
pub fn next_poll_hint(job_id: &str, status: &str) -> String {
    let state = if status == "queued" {
        "queued, waiting for a free slot"
    } else {
        "in progress"
    };
    format!("{state}; call job_status with job_id={job_id} until status is completed or failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Dummy {
        n: u32,
    }

    #[tokio::test]
    async fn fast_work_completes_inline_without_timeout_arg() {
        // Async is the default (None → DEFAULT_TIMEOUT_SECS), but work that
        // finishes within the window still returns inline as Completed.
        let store = JobStore::new();
        let out = run_with_timeout(&store, JobKind::AskGrok, None, || async {
            Ok::<_, Fail>(Dummy { n: 1 })
        })
        .await
        .unwrap();
        match out {
            RunOutcome::Completed(d) => assert_eq!(d.n, 1),
            RunOutcome::Running { .. } => panic!("expected completed"),
        }
    }

    #[test]
    fn poll_hint_distinguishes_queued_and_running() {
        let q = next_poll_hint("job_1", "queued");
        let r = next_poll_hint("job_1", "running");
        assert!(q.contains("queued"), "hint={q}");
        assert!(r.contains("in progress"), "hint={r}");
        assert!(q.contains("job_1") && r.contains("job_1"));
    }

    #[test]
    fn effective_timeout_defaults_when_absent() {
        assert_eq!(effective_timeout_secs(None).unwrap(), DEFAULT_TIMEOUT_SECS);
    }

    #[test]
    fn effective_timeout_uses_and_clamps_explicit() {
        assert_eq!(effective_timeout_secs(Some(5)).unwrap(), 5);
        assert_eq!(
            effective_timeout_secs(Some(9999)).unwrap(),
            TIMEOUT_SECS_MAX
        );
        assert!(effective_timeout_secs(Some(0)).is_err());
    }

    #[tokio::test]
    async fn timeout_returns_running_then_completes() {
        let store = JobStore::new();
        let store2 = store.clone();
        let out = run_with_timeout(&store, JobKind::Research, Some(1), || async {
            tokio::time::sleep(Duration::from_millis(1500)).await;
            Ok::<_, Fail>(Dummy { n: 42 })
        })
        .await
        .unwrap();
        let job_id = match out {
            RunOutcome::Running {
                job_id,
                elapsed_secs,
                status,
            } => {
                assert_eq!(elapsed_secs, 1);
                // Executing (permit held), so the hand-off status is running, not queued.
                assert_eq!(status, "running");
                job_id
            }
            RunOutcome::Completed(_) => panic!("expected running"),
        };
        // Still running briefly
        let snap = store2.get(&job_id).unwrap();
        assert_eq!(snap.status, "running");
        tokio::time::sleep(Duration::from_millis(800)).await;
        let snap = store2.get(&job_id).unwrap();
        assert_eq!(snap.status, "completed");
        let d: Dummy = serde_json::from_value(snap.result.unwrap()).unwrap();
        assert_eq!(d.n, 42);
    }

    #[tokio::test]
    async fn fast_work_completes_before_timeout() {
        let store = JobStore::new();
        let out = run_with_timeout(&store, JobKind::XSearch, Some(5), || async {
            Ok::<_, Fail>(Dummy { n: 7 })
        })
        .await
        .unwrap();
        match out {
            RunOutcome::Completed(d) => assert_eq!(d.n, 7),
            RunOutcome::Running { .. } => panic!("expected completed"),
        }
    }

    #[test]
    fn admit_queues_until_capacity_then_rate_limited() {
        let store = JobStore::new();
        // Running slots + queue slots are all admissible.
        for _ in 0..(MAX_INFLIGHT + MAX_QUEUED) {
            store
                .admit(JobKind::XSearch)
                .expect("within queue capacity");
        }
        // One past the queue is rejected as retryable RATE_LIMITED.
        let err = store
            .admit(JobKind::XSearch)
            .expect_err("over queue capacity");
        assert_eq!(err.error.code, ErrorCode::RateLimited);
        assert!(err.error.retryable);
    }

    #[tokio::test]
    async fn over_inflight_is_queued_then_drains() {
        let store = JobStore::new();
        // Hold every inflight permit with jobs that block on a gate.
        let gate = std::sync::Arc::new(tokio::sync::Notify::new());
        let mut blockers = Vec::new();
        for _ in 0..MAX_INFLIGHT {
            let store = store.clone();
            let g = gate.clone();
            blockers.push(tokio::spawn(async move {
                run_with_timeout(&store, JobKind::XSearch, Some(1), move || async move {
                    g.notified().await;
                    Ok::<_, Fail>(Dummy { n: 0 })
                })
                .await
                .unwrap()
            }));
        }
        for b in blockers {
            assert!(matches!(b.await.unwrap(), RunOutcome::Running { .. }));
        }

        // One more job: admitted, but no permit — it must sit in the queue and
        // still return immediately (offloaded) rather than block.
        let out = run_with_timeout(&store, JobKind::XSearch, Some(1), || async {
            Ok::<_, Fail>(Dummy { n: 99 })
        })
        .await
        .unwrap();
        let job_id = match out {
            RunOutcome::Running { job_id, status, .. } => {
                // Hand-off status reflects that it is waiting, not executing.
                assert_eq!(status, "queued");
                job_id
            }
            RunOutcome::Completed(_) => panic!("expected queued/running"),
        };
        assert_eq!(store.get(&job_id).unwrap().status, "queued");

        // Release the blockers; the queued job acquires a freed slot and completes.
        gate.notify_waiters();
        let mut done = false;
        for _ in 0..100 {
            if store.get(&job_id).unwrap().status == "completed" {
                done = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(done, "queued job never drained");
        let d: Dummy = serde_json::from_value(store.get(&job_id).unwrap().result.unwrap()).unwrap();
        assert_eq!(d.n, 99);
    }

    #[test]
    fn timeout_zero_rejected() {
        assert!(parse_timeout_secs(Some(0)).is_err());
    }

    #[test]
    fn timeout_clamped() {
        assert_eq!(parse_timeout_secs(Some(999)).unwrap(), Some(300));
    }
}
