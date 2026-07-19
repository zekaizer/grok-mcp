//! In-memory async jobs for long tool calls (timeout_secs + poll via job_status).

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::envelope::{ErrorCode, Fail};

/// Max concurrent background jobs (SuperGrok quota protection).
pub const MAX_INFLIGHT: usize = 10;

/// Keep finished jobs this long for polling.
pub const JOB_TTL: Duration = Duration::from_secs(30 * 60);

/// Clamp for `timeout_secs` tool args (seconds).
pub const TIMEOUT_SECS_MIN: u32 = 1;
pub const TIMEOUT_SECS_MAX: u32 = 300;

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
    Running,
    Completed(Value),
    Failed { code: String, message: String },
}

#[derive(Debug, Clone)]
struct JobEntry {
    kind: JobKind,
    created: Instant,
    finished: Option<Instant>,
    state: JobState,
}

/// Shared job registry (cheap to clone).
#[derive(Clone, Default)]
pub struct JobStore {
    inner: Arc<Mutex<HashMap<String, JobEntry>>>,
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

    fn inflight_locked(map: &HashMap<String, JobEntry>) -> usize {
        map.values()
            .filter(|e| matches!(e.state, JobState::Running))
            .count()
    }

    /// Allocate a running job slot. Errors if at inflight cap.
    pub fn start(&self, kind: JobKind) -> Result<String, Fail> {
        let mut map = self.inner.lock().expect("job store lock");
        Self::purge_locked(&mut map);
        if Self::inflight_locked(&map) >= MAX_INFLIGHT {
            return Err(Fail::new(
                ErrorCode::RateLimited,
                format!("too many inflight jobs (max {MAX_INFLIGHT}); poll job_status or retry"),
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
                state: JobState::Running,
            },
        );
        Ok(id)
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
    Running { job_id: String, elapsed_secs: u64 },
}

/// Validate and clamp optional timeout.
pub fn parse_timeout_secs(raw: Option<u32>) -> Result<Option<u32>, Fail> {
    match raw {
        None => Ok(None),
        Some(0) => Err(Fail::new(
            ErrorCode::InvalidParams,
            "timeout_secs must be >= 1 when set (omit for fully synchronous wait)",
            false,
        )),
        Some(n) => Ok(Some(n.clamp(TIMEOUT_SECS_MIN, TIMEOUT_SECS_MAX))),
    }
}

/// Run `work` to completion, or return `Running` after `timeout_secs` while work continues.
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
    let timeout_secs = parse_timeout_secs(timeout_secs)?;

    match timeout_secs {
        None => {
            let result = work().await?;
            Ok(RunOutcome::Completed(result))
        }
        Some(secs) => {
            let job_id = store.start(kind)?;
            let store_bg = store.clone();
            let jid = job_id.clone();
            let handle = tokio::spawn(async move {
                match work().await {
                    Ok(val) => match serde_json::to_value(&val) {
                        Ok(json) => store_bg.complete_json(&jid, json),
                        Err(e) => store_bg.fail(&jid, "UPSTREAM_ERROR", e.to_string()),
                    },
                    Err(fail) => store_bg.fail(&jid, fail.error.code.as_str(), fail.error.message),
                }
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
                    Ok(RunOutcome::Running {
                        job_id,
                        elapsed_secs: u64::from(secs),
                    })
                }
            }
        }
    }
}

/// Hint for hosts after a running response.
#[must_use]
pub fn next_poll_hint(job_id: &str) -> String {
    format!("call job_status with job_id={job_id} until status is completed or failed")
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
    async fn sync_path_no_timeout() {
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
            } => {
                assert_eq!(elapsed_secs, 1);
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
    fn inflight_cap_enforced() {
        let store = JobStore::new();
        // Fill every slot.
        for _ in 0..MAX_INFLIGHT {
            store.start(JobKind::XSearch).expect("slot within cap");
        }
        // One past the cap is rejected as retryable RATE_LIMITED.
        let err = store.start(JobKind::XSearch).expect_err("over cap");
        assert_eq!(err.error.code, ErrorCode::RateLimited);
        assert!(err.error.retryable);
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
