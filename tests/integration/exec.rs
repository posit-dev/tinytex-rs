//! `StdExec` deadlines and the `ExitStatus` test helper.

use std::ffi::OsString;
use std::process::Command;
use std::time::{Duration, Instant};

use tinytex::test_support::exit_status;
use tinytex::{CommandSpec, Exec, StdExec};

#[test]
fn exit_status_helper_builds_a_status_from_a_code() {
    assert!(exit_status(0).success());
    assert_eq!(exit_status(3).code(), Some(3));
    assert!(!exit_status(255).success());
}

#[cfg(unix)]
fn sleeper(secs: u64) -> CommandSpec {
    let mut c = CommandSpec::new("sleep");
    c.args = vec![OsString::from(secs.to_string())];
    c
}

#[cfg(not(unix))]
fn sleeper(secs: u64) -> CommandSpec {
    let mut c = CommandSpec::new("powershell");
    c.args = [
        "-NoProfile",
        "-Command",
        &format!("Start-Sleep -Seconds {secs}"),
    ]
    .map(OsString::from)
    .to_vec();
    c
}

/// A command that prints a lot to both pipes and exits.
fn chatty() -> CommandSpec {
    if cfg!(windows) {
        let mut c = CommandSpec::new("powershell");
        c.args = [
            "-NoProfile",
            "-Command",
            "1..20000 | ForEach-Object { 'x' * 100; [Console]::Error.WriteLine('y' * 100) }",
        ]
        .map(OsString::from)
        .to_vec();
        c
    } else {
        let mut c = CommandSpec::new("sh");
        c.args = ["-c", "i=0; while [ $i -lt 20000 ]; do echo xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; echo yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy >&2; i=$((i+1)); done"]
            .map(OsString::from)
            .to_vec();
        c
    }
}

#[test]
fn run_captures_stdout_stderr_and_status() {
    let mut spec = if cfg!(windows) {
        CommandSpec::new("cmd")
    } else {
        CommandSpec::new("sh")
    };
    spec.args = if cfg!(windows) {
        ["/C", "echo out& echo err 1>&2& exit 3"]
            .map(OsString::from)
            .to_vec()
    } else {
        ["-c", "echo out; echo err >&2; exit 3"]
            .map(OsString::from)
            .to_vec()
    };
    let out = StdExec::new().run(&spec).unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stdout).contains("out"));
    assert!(String::from_utf8_lossy(&out.stderr).contains("err"));
}

#[test]
fn a_chatty_child_does_not_deadlock() {
    let x = StdExec::with_deadline(Duration::from_secs(60));
    let out = x.run(&chatty()).unwrap();
    assert!(out.status.success());
    assert!(out.stdout.len() > 1_000_000 && out.stderr.len() > 1_000_000);
}

#[test]
fn a_slow_child_is_killed_when_the_budget_ends() {
    let x = StdExec::with_deadline(Duration::from_millis(400));
    let start = Instant::now();
    let err = x.run(&sleeper(30)).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
    assert!(start.elapsed() < Duration::from_secs(10));
    assert!(x.interrupted());
}

#[test]
fn each_later_child_gets_only_the_remaining_time() {
    let x = StdExec::with_deadline(Duration::from_millis(1500));
    // The first command burns most of the budget by finishing quickly in
    // wall-clock terms only if the budget is not reset: wait it out here.
    std::thread::sleep(Duration::from_millis(1100));
    let start = Instant::now();
    let err = x.run(&sleeper(30)).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
    // About 400 ms remained, not the whole 1500 ms.
    assert!(
        start.elapsed() < Duration::from_millis(1400),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn the_clock_starts_at_construction_not_at_the_first_command() {
    let x = StdExec::with_deadline(Duration::from_millis(300));
    assert!(!x.interrupted());
    std::thread::sleep(Duration::from_millis(500));
    assert!(x.interrupted());
    // Past the budget no child is spawned at all.
    let err = x.run(&sleeper(30)).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
}

#[test]
fn without_a_deadline_nothing_is_interrupted() {
    let x = StdExec::new();
    assert!(!x.interrupted());
    let _ = Command::new("x"); // keep `Command` imported on every platform
}
