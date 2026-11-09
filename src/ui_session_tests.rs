use super::UiSession;
use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

fn directory() -> std::path::PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "kittx-ui-lock-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn exclusive_until_dropped_and_existing_file_is_reusable() {
    let directory = directory();
    let path = directory.join("session.lock");
    let first = UiSession::at(&path).unwrap();
    assert!(
        matches!(UiSession::at(&path), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
    );
    drop(first);
    assert!(path.exists());
    drop(UiSession::at(&path).unwrap());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn file_errors_are_reported_without_removing_evidence() {
    let directory = directory();
    assert!(UiSession::at(&directory).is_err());
    assert!(directory.is_dir());
    std::fs::remove_dir_all(directory).unwrap();
}

fn child(directory: &std::path::Path, busy: bool) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "ui_session::tests::child_lock",
            "--ignored",
            "--nocapture",
        ])
        .env("KITTX_LOCK_TEST_HOME", directory)
        .env("KITTX_LOCK_TEST_BUSY", if busy { "yes" } else { "no" });
    #[cfg(target_os = "windows")]
    command.env("LOCALAPPDATA", directory);
    #[cfg(not(target_os = "windows"))]
    command.env("HOME", directory);
    command
}

#[test]
fn other_process_is_rejected_and_crash_releases_lock() {
    let directory = directory();
    // Exercise the production path selection in separate processes, without
    // changing environment variables in the multithreaded test runner.
    let mut owner = child(&directory, false)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = std::io::BufReader::new(owner.stdout.take().unwrap()).lines();
    assert!(lines.any(|line| line.unwrap() == "LOCKED"));
    assert!(child(&directory, true).output().unwrap().status.success());
    owner.kill().unwrap();
    owner.wait().unwrap();
    let path = directory.join(".kittx/desktop-ui.lock");
    drop(UiSession::at(&path).unwrap());
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
#[ignore = "subprocess fixture; invoked by other_process_is_rejected_and_crash_releases_lock"]
fn child_lock() {
    if std::env::var_os("KITTX_LOCK_TEST_HOME").is_none() {
        return;
    }
    if std::env::var("KITTX_LOCK_TEST_BUSY").unwrap() == "yes" {
        let _launcher = UiSession::acquire("launcher").unwrap();
        assert!(
            matches!(UiSession::acquire("launcher"), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
        assert!(
            matches!(UiSession::acquire("desktop-ui"), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    } else {
        let _guard = UiSession::acquire("desktop-ui").unwrap();
        println!("LOCKED");
        std::io::stdout().flush().unwrap();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).unwrap();
    }
}
