#![cfg(unix)]

use mew_agy::process::{LaunchSpec, ManagedProcess, ProcessOutcome, UnsandboxedBoundary};
use std::time::Duration;
use tokio::io::AsyncReadExt;

#[tokio::test]
async fn local_process_inherits_identity_without_unrelated_mew_credentials() {
    let workspace = tempfile::tempdir().unwrap();
    let home = std::env::var_os("HOME").expect("normal local account has a home");
    let spec = LaunchSpec {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), "if [ \"$HOME\" = \"$1\" ] && [ -n \"$PATH\" ] && [ \"$AGY_TEST_VISIBLE\" = expected ] && [ -z \"${MEW_CRED_UNRELATED+x}\" ] && [ -z \"${MEW_CONFIG_DIR+x}\" ]; then printf ok; else printf bad; fi".into(), "test".into(), home],
        env: vec![("MEW_CRED_UNRELATED".into(), "must-not-leak".into()), ("AGY_TEST_VISIBLE".into(), "expected".into())],
        cwd: workspace.path().to_owned(),
    };
    let mut process = ManagedProcess::spawn(
        spec,
        &UnsandboxedBoundary,
        workspace,
        Duration::from_secs(5),
    )
    .unwrap();
    let mut stdout = String::new();
    process
        .take_stdout()
        .unwrap()
        .read_to_string(&mut stdout)
        .await
        .unwrap();
    assert_eq!(stdout, "ok");
    assert!(
        matches!(process.wait().await.unwrap(), ProcessOutcome::Exited(status) if status.success())
    );
}

#[tokio::test]
async fn local_process_cancellation_reaps_its_descendant_group() {
    let workspace = tempfile::tempdir().unwrap();
    let spec = LaunchSpec {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), "sleep 60 & echo $!; wait".into()],
        env: vec![],
        cwd: workspace.path().to_owned(),
    };
    let mut process = ManagedProcess::spawn(
        spec,
        &UnsandboxedBoundary,
        workspace,
        Duration::from_secs(10),
    )
    .unwrap();
    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut line = String::new();
    BufReader::new(process.take_stdout().unwrap())
        .read_line(&mut line)
        .await
        .unwrap();
    let child: libc::pid_t = line.trim().parse().unwrap();
    assert!(matches!(
        process.cancel().await.unwrap(),
        ProcessOutcome::Cancelled
    ));
    assert_eq!(unsafe { libc::kill(child, 0) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::ESRCH)
    );
}
