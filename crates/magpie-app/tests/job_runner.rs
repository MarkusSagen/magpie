use magpie_app::job_runner::execute_job;
use magpie_core::{jobs::JobKind, open_in_memory};
use std::time::{Duration, Instant};

fn run(command: &str, working_dir: &str, timeout_secs: i64) -> magpie_app::job_runner::RunResult {
    let store = open_in_memory().unwrap();
    let id = store
        .add_job(
            "test",
            JobKind::Shell,
            command,
            "* * * * *",
            working_dir,
            timeout_secs,
            false,
            false,
            0,
        )
        .unwrap();
    execute_job(&store.job_for_run(id).unwrap().unwrap())
}

#[test]
fn shell_jobs_capture_both_streams_and_exit_status() {
    let result = run("echo magpie-stdout & echo magpie-stderr >&2", "", 10);
    assert_eq!(result.exit_code, Some(0), "{}", result.stderr);
    assert!(result.stdout.contains("magpie-stdout"));
    assert!(result.stderr.contains("magpie-stderr"));

    let result = run("exit 7", "", 10);
    assert_eq!(result.exit_code, Some(7));
}

#[test]
fn shell_jobs_use_the_requested_directory_with_spaces() {
    let dir = std::env::temp_dir().join(format!("magpie job cwd {}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let result = run("echo cwd-marker > marker.txt", dir.to_str().unwrap(), 10);
    assert_eq!(result.exit_code, Some(0), "{}", result.stderr);
    assert!(std::fs::read_to_string(dir.join("marker.txt"))
        .unwrap()
        .contains("cwd-marker"));
    #[cfg(unix)]
    let (script, command) = ("run script.sh", "/bin/sh \"run script.sh\"");
    #[cfg(target_os = "windows")]
    let (script, command) = ("run script.cmd", "\"run script.cmd\"");
    std::fs::write(dir.join(script), "echo magpie-quoted\n").unwrap();
    let result = run(command, dir.to_str().unwrap(), 10);
    assert_eq!(result.exit_code, Some(0), "{}", result.stderr);
    assert!(result.stdout.contains("magpie-quoted"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn timeout_kills_shell_children_holding_output_pipes() {
    #[cfg(unix)]
    let command = "sleep 10; echo should-not-run";
    #[cfg(target_os = "windows")]
    let command = "ping -n 11 127.0.0.1 >nul & echo should-not-run";

    let start = Instant::now();
    let result = run(command, "", 1);
    assert!(
        start.elapsed() < Duration::from_secs(8),
        "timeout must include descendants and output readers"
    );
    assert_eq!(result.exit_code, None);
    assert!(result.stderr.contains("timed out"));
    assert!(!result.stdout.contains("should-not-run"));
}

#[cfg(unix)]
#[test]
fn timeout_includes_pipes_inherited_by_background_children() {
    let start = Instant::now();
    let result = run("sleep 10 & exit 0", "", 1);
    assert!(start.elapsed() < Duration::from_secs(8));
    assert_eq!(result.exit_code, None);
    assert!(result.stderr.contains("timed out"));
}
