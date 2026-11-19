//! Scheduled jobs (Phase 21): the encrypted job store + the security-critical command
//! hash. Jobs are created only through this API (which the UI calls) — never from
//! project files. Before any run, [`Store::job_for_run`] re-verifies the stored
//! `command_hash`, so an OS scheduler entry (which only ever calls
//! `magpie --run-job <id>`) can't be edited to run something else.

use crate::store::{Result, Store};
use rusqlite::OptionalExtension;

/// What a job does when it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    /// Show a notification (`command` is the message).
    Notification,
    /// Run a shell command (`command`) with the login shell.
    Shell,
    /// `open` a file / app / note (`command` is the path or note id).
    Open,
    /// Open a URL (`command`).
    Url,
}

impl JobKind {
    pub fn as_str(self) -> &'static str {
        match self {
            JobKind::Notification => "notification",
            JobKind::Shell => "shell",
            JobKind::Open => "open",
            JobKind::Url => "url",
        }
    }
    pub fn from_db(s: &str) -> JobKind {
        match s {
            "shell" => JobKind::Shell,
            "open" => JobKind::Open,
            "url" => JobKind::Url,
            _ => JobKind::Notification,
        }
    }
}

/// A scheduled job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: i64,
    pub name: String,
    pub kind: JobKind,
    pub command: String,
    pub cron: String,
    pub working_dir: String,
    pub timeout_secs: i64,
    pub notify_on_failure: bool,
    pub enabled: bool,
    /// false = Magpie's own while-open scheduler; true = exported to the OS scheduler.
    pub os_managed: bool,
    pub command_hash: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// One recorded run of a job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobRun {
    pub id: i64,
    pub job_id: i64,
    pub start_ms: i64,
    pub end_ms: Option<i64>,
    pub exit_code: Option<i64>,
    pub stdout: String,
    pub stderr: String,
}

/// The canonical hash of a job's action — what `job_for_run` verifies. Binds the kind
/// to the command so neither can be swapped without detection.
pub fn command_hash(kind: JobKind, command: &str) -> String {
    let mut h = blake3::Hasher::new();
    h.update(kind.as_str().as_bytes());
    h.update(b"\0");
    h.update(command.as_bytes());
    h.finalize().to_hex().to_string()
}

const JOB_COLS: &str = "id, name, kind, command, cron, working_dir, timeout_secs, \
     notify_on_failure, enabled, os_managed, command_hash, created_at_ms, updated_at_ms";

fn row_to_job(r: &rusqlite::Row) -> rusqlite::Result<Job> {
    Ok(Job {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: JobKind::from_db(&r.get::<_, String>(2)?),
        command: r.get(3)?,
        cron: r.get(4)?,
        working_dir: r.get(5)?,
        timeout_secs: r.get(6)?,
        notify_on_failure: r.get::<_, i64>(7)? != 0,
        enabled: r.get::<_, i64>(8)? != 0,
        os_managed: r.get::<_, i64>(9)? != 0,
        command_hash: r.get(10)?,
        created_at_ms: r.get(11)?,
        updated_at_ms: r.get(12)?,
    })
}

impl Store {
    /// Create a job, computing + storing its command hash. Returns the new id.
    #[allow(clippy::too_many_arguments)]
    pub fn add_job(
        &self,
        name: &str,
        kind: JobKind,
        command: &str,
        cron: &str,
        working_dir: &str,
        timeout_secs: i64,
        notify_on_failure: bool,
        os_managed: bool,
        now_ms: i64,
    ) -> Result<i64> {
        let hash = command_hash(kind, command);
        self.conn().execute(
            "INSERT INTO jobs
             (name, kind, command, cron, working_dir, timeout_secs, notify_on_failure,
              enabled, os_managed, command_hash, created_at_ms, updated_at_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7, 1, ?8, ?9, ?10, ?10)",
            rusqlite::params![
                name,
                kind.as_str(),
                command,
                cron,
                working_dir,
                timeout_secs,
                notify_on_failure as i64,
                os_managed as i64,
                hash,
                now_ms,
            ],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    /// Update a job's action/schedule, re-computing its command hash.
    #[allow(clippy::too_many_arguments)]
    pub fn update_job(
        &self,
        id: i64,
        name: &str,
        kind: JobKind,
        command: &str,
        cron: &str,
        working_dir: &str,
        timeout_secs: i64,
        notify_on_failure: bool,
        os_managed: bool,
        now_ms: i64,
    ) -> Result<()> {
        let hash = command_hash(kind, command);
        self.conn().execute(
            "UPDATE jobs SET name=?2, kind=?3, command=?4, cron=?5, working_dir=?6,
             timeout_secs=?7, notify_on_failure=?8, os_managed=?9, command_hash=?10,
             updated_at_ms=?11 WHERE id=?1",
            rusqlite::params![
                id,
                name,
                kind.as_str(),
                command,
                cron,
                working_dir,
                timeout_secs,
                notify_on_failure as i64,
                os_managed as i64,
                hash,
                now_ms,
            ],
        )?;
        Ok(())
    }

    pub fn set_job_enabled(&self, id: i64, enabled: bool, now_ms: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE jobs SET enabled=?2, updated_at_ms=?3 WHERE id=?1",
            rusqlite::params![id, enabled as i64, now_ms],
        )?;
        Ok(())
    }

    pub fn delete_job(&self, id: i64) -> Result<()> {
        self.conn()
            .execute("DELETE FROM job_runs WHERE job_id=?1", [id])?;
        self.conn().execute("DELETE FROM jobs WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn list_jobs(&self) -> Result<Vec<Job>> {
        let mut stmt = self
            .conn()
            .prepare(&format!("SELECT {JOB_COLS} FROM jobs ORDER BY id"))?;
        let rows = stmt.query_map([], row_to_job)?;
        rows.collect()
    }

    pub fn get_job(&self, id: i64) -> Result<Option<Job>> {
        self.conn()
            .query_row(
                &format!("SELECT {JOB_COLS} FROM jobs WHERE id=?1"),
                [id],
                row_to_job,
            )
            .optional()
    }

    /// Load a job for execution, verifying its stored command hash. Returns `None` if
    /// the job is missing or the hash doesn't match its command (tamper guard) — the
    /// caller must then refuse to run it.
    pub fn job_for_run(&self, id: i64) -> Result<Option<Job>> {
        let job = self.get_job(id)?;
        Ok(job.filter(|j| j.command_hash == command_hash(j.kind, &j.command)))
    }

    /// Start a run row; returns its id.
    pub fn start_job_run(&self, job_id: i64, now_ms: i64) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO job_runs (job_id, start_ms) VALUES (?1, ?2)",
            rusqlite::params![job_id, now_ms],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    /// Finish a run row with exit code + captured output, and cap history to the last
    /// `keep` runs for that job.
    #[allow(clippy::too_many_arguments)]
    pub fn finish_job_run(
        &self,
        run_id: i64,
        job_id: i64,
        end_ms: i64,
        exit_code: Option<i64>,
        stdout: &str,
        stderr: &str,
        keep: i64,
    ) -> Result<()> {
        // Cap each captured stream so a chatty job can't bloat the DB.
        let cap = |s: &str| s.chars().take(8_000).collect::<String>();
        self.conn().execute(
            "UPDATE job_runs SET end_ms=?2, exit_code=?3, stdout=?4, stderr=?5 WHERE id=?1",
            rusqlite::params![run_id, end_ms, exit_code, cap(stdout), cap(stderr)],
        )?;
        // Retention: keep the newest `keep` runs for this job.
        self.conn().execute(
            "DELETE FROM job_runs WHERE job_id=?1 AND id NOT IN
             (SELECT id FROM job_runs WHERE job_id=?1 ORDER BY start_ms DESC LIMIT ?2)",
            rusqlite::params![job_id, keep],
        )?;
        Ok(())
    }

    pub fn recent_job_runs(&self, job_id: i64, limit: i64) -> Result<Vec<JobRun>> {
        let mut stmt = self.conn().prepare(
            "SELECT id, job_id, start_ms, end_ms, exit_code, stdout, stderr FROM job_runs
             WHERE job_id=?1 ORDER BY start_ms DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(rusqlite::params![job_id, limit], |r| {
            Ok(JobRun {
                id: r.get(0)?,
                job_id: r.get(1)?,
                start_ms: r.get(2)?,
                end_ms: r.get(3)?,
                exit_code: r.get(4)?,
                stdout: r.get(5)?,
                stderr: r.get(6)?,
            })
        })?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::open_in_memory;

    #[test]
    fn add_get_list_roundtrip() {
        let s = open_in_memory().unwrap();
        let id = s
            .add_job(
                "Backup",
                JobKind::Shell,
                "rsync -a ~/src ~/bak",
                "0 2 * * *",
                "/home/me",
                600,
                true,
                false,
                100,
            )
            .unwrap();
        let j = s.get_job(id).unwrap().unwrap();
        assert_eq!(j.name, "Backup");
        assert_eq!(j.kind, JobKind::Shell);
        assert_eq!(j.cron, "0 2 * * *");
        assert!(j.enabled && j.notify_on_failure && !j.os_managed);
        assert_eq!(s.list_jobs().unwrap().len(), 1);
    }

    #[test]
    fn command_hash_binds_kind_and_command() {
        // A different kind or command → a different hash.
        assert_ne!(
            command_hash(JobKind::Shell, "ls"),
            command_hash(JobKind::Url, "ls")
        );
        assert_ne!(
            command_hash(JobKind::Shell, "ls"),
            command_hash(JobKind::Shell, "rm -rf /")
        );
        assert_eq!(
            command_hash(JobKind::Shell, "ls"),
            command_hash(JobKind::Shell, "ls")
        );
    }

    #[test]
    fn job_for_run_refuses_tampered_command() {
        let s = open_in_memory().unwrap();
        let id = s
            .add_job(
                "J",
                JobKind::Shell,
                "echo ok",
                "* * * * *",
                "",
                60,
                true,
                true,
                1,
            )
            .unwrap();
        // Honest job runs.
        assert!(s.job_for_run(id).unwrap().is_some());
        // Tamper with the command directly, leaving the old hash → refused.
        s.conn()
            .execute("UPDATE jobs SET command='rm -rf /' WHERE id=?1", [id])
            .unwrap();
        assert!(
            s.job_for_run(id).unwrap().is_none(),
            "a command that doesn't match its stored hash must not run"
        );
    }

    #[test]
    fn runs_record_and_cap() {
        let s = open_in_memory().unwrap();
        let id = s
            .add_job(
                "J",
                JobKind::Shell,
                "echo hi",
                "* * * * *",
                "",
                60,
                false,
                false,
                1,
            )
            .unwrap();
        for i in 0..5 {
            let run = s.start_job_run(id, 1000 + i).unwrap();
            s.finish_job_run(run, id, 2000 + i, Some(0), "out", "", 3)
                .unwrap();
        }
        // Retention keeps only the newest 3.
        let runs = s.recent_job_runs(id, 10).unwrap();
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].exit_code, Some(0));
        assert!(runs[0].start_ms > runs[1].start_ms, "newest first");
    }

    #[test]
    fn enable_disable_and_delete() {
        let s = open_in_memory().unwrap();
        let id = s
            .add_job(
                "J",
                JobKind::Notification,
                "hi",
                "* * * * *",
                "",
                60,
                true,
                false,
                1,
            )
            .unwrap();
        s.set_job_enabled(id, false, 2).unwrap();
        assert!(!s.get_job(id).unwrap().unwrap().enabled);
        s.delete_job(id).unwrap();
        assert!(s.get_job(id).unwrap().is_none());
        assert!(s.list_jobs().unwrap().is_empty());
    }
}
