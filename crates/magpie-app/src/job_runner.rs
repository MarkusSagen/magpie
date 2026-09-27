//! Execute scheduled jobs (Phase 21) — shared by the CLI (`magpie --run-job <id>`,
//! what OS schedulers call) and the in-app while-open scheduler. Runs a job's action
//! (notification / shell / open / URL), captures output with a timeout, and records
//! the run. The command hash is re-verified in `run_job_by_id` before anything runs.

use magpie_core::jobs::{Job, JobKind};
use magpie_core::Store;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The outcome of one execution.
pub struct RunResult {
    pub exit_code: Option<i64>,
    pub stdout: String,
    pub stderr: String,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Run a job's action. Never verifies the hash itself — callers use
/// [`run_job_by_id`], which does.
pub fn execute_job(job: &Job) -> RunResult {
    match job.kind {
        JobKind::Notification => {
            magpie_platform::notify(&job.name, &job.command);
            RunResult {
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            }
        }
        JobKind::Open | JobKind::Url => {
            let mut c = Command::new(open_program());
            c.arg(&job.command)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            spawn_with_timeout(c, 30)
        }
        JobKind::Shell => {
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
            let mut c = Command::new(shell);
            c.arg("-lc")
                .arg(&job.command)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if !job.working_dir.is_empty() {
                c.current_dir(&job.working_dir);
            }
            spawn_with_timeout(c, job.timeout_secs.max(1) as u64)
        }
    }
}

fn open_program() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "open"
    }
    #[cfg(target_os = "windows")]
    {
        "cmd" // handled below via /C start — see note
    }
    #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
    {
        "xdg-open"
    }
}

/// Spawn `c`, draining stdout/stderr on threads (so a chatty child can't deadlock),
/// and enforcing a wall-clock timeout (killed on expiry).
fn spawn_with_timeout(mut c: Command, timeout_secs: u64) -> RunResult {
    let mut child = match c.spawn() {
        Ok(ch) => ch,
        Err(e) => {
            return RunResult {
                exit_code: None,
                stdout: String::new(),
                stderr: format!("spawn failed: {e}"),
            }
        }
    };
    let out = child.stdout.take();
    let err = child.stderr.take();
    let out_h = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut o) = out {
            let _ = o.read_to_string(&mut s);
        }
        s
    });
    let err_h = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut e) = err {
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break None,
        }
    };
    let stdout = out_h.join().unwrap_or_default();
    let mut stderr = err_h.join().unwrap_or_default();
    if timed_out {
        stderr.push_str("\n[magpie] job timed out and was killed");
    }
    RunResult {
        exit_code: status.and_then(|s| s.code().map(|c| c as i64)),
        stdout,
        stderr,
    }
}

/// Load a job by id, **verify its command hash**, run it, record the run (capped
/// history), and notify on failure. Returns true if the job ran and exited 0. Refuses
/// (returns false) if the job is missing or its stored hash doesn't match — the guard
/// that stops a tampered OS scheduler entry from running something else.
pub fn run_job_by_id(store: &Store, id: i64) -> bool {
    let job = match store.job_for_run(id) {
        Ok(Some(j)) => j,
        _ => {
            magpie_platform::notify(
                "Magpie — job refused",
                "A scheduled job could not be verified and was not run.",
            );
            return false;
        }
    };
    let run = store.start_job_run(id, now_ms()).ok();
    let res = execute_job(&job);
    let ok = res.exit_code == Some(0) || matches!(job.kind, JobKind::Notification);
    if let Some(run) = run {
        let _ = store.finish_job_run(
            run,
            id,
            now_ms(),
            res.exit_code,
            &res.stdout,
            &res.stderr,
            20,
        );
    }
    if !ok && job.notify_on_failure {
        let code = res
            .exit_code
            .map(|c| format!("exit {c}"))
            .unwrap_or_else(|| "did not complete".to_string());
        magpie_platform::notify(&format!("Magpie job failed: {}", job.name), &code);
    }
    ok
}
