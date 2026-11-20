//! Noninteractive structured output tests.

#[test]
fn jsonl_reports_provider_startup_failures() {
    let temp = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mew"))
        .args([
            "run",
            "--format",
            "jsonl",
            "--provider",
            "missing-provider",
            "--model",
            "test-model",
            "hello",
        ])
        .current_dir(temp.path())
        .env("MEW_CONFIG_DIR", temp.path().join("config"))
        .env("MEW_DATA_DIR", temp.path().join("data"))
        .env("MEW_CACHE_DIR", temp.path().join("cache"))
        .env("MEW_SESSION_DIR", temp.path().join("sessions"))
        .output()
        .expect("run mew");
    assert!(!output.status.success());
    let records: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).expect("each stdout line must be JSON"))
        .collect();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["type"], "run_started");
    assert_eq!(records[0]["provider"], "missing-provider");
    assert_eq!(records[1]["type"], "run_finished");
    assert_eq!(records[1]["status"], "failed");
    assert!(records[1]["error"].as_str().unwrap().contains("provider"));
}
