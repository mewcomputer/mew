//! Opt-in checks against real provider APIs.

#[test]
#[ignore = "requires MEW_CRED_OPENCODE_ZEN and access to OpenCode Go"]
fn opencode_go_accepts_noninteractive_session() {
    let temp = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mew"))
        .args([
            "run",
            "--provider",
            "opencode-go",
            "--model",
            "hy4-flash-free",
            "Reply with the word OK.",
        ])
        .current_dir(temp.path())
        .env("MEW_CONFIG_DIR", temp.path().join("config"))
        .env("MEW_DATA_DIR", temp.path().join("data"))
        .env("MEW_CACHE_DIR", temp.path().join("cache"))
        .env("MEW_SESSION_DIR", temp.path().join("sessions"))
        .output()
        .expect("run mew");

    assert!(
        output.status.success(),
        "OpenCode Go rejected the run: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.stdout.is_empty(),
        "the model should return a response"
    );

    let sessions = temp.path().join("sessions");
    let session = std::fs::read_dir(sessions)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.join("session.jsonl").is_file())
        .expect("run should persist a session");
    let session_id = session.file_name().unwrap().to_str().unwrap();
    let history = std::fs::read_to_string(session.join("session.jsonl")).unwrap();
    assert!(!history.is_empty());
    for line in history.lines() {
        let message: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(message["session_id"].as_str(), Some(session_id));
    }
}
