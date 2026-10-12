use super::*;
use tokio_tungstenite::WebSocketStream;

type TestSocket = WebSocketStream<tokio::io::DuplexStream>;

fn server(directory: &std::path::Path) -> DaemonServer {
    DaemonServer::with_session_dir(
        Arc::new(|params| {
            Ok((
                Agent::new(
                    Arc::new(mew_provider_fake::FakeProvider::new(Vec::new())),
                    Arc::new(mew_hooks::NopDispatcher),
                    Some(params.writer),
                    Vec::new(),
                    None,
                ),
                None,
                None,
            ))
        }),
        directory.to_path_buf(),
    )
}

fn git(root: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn repository(root: &std::path::Path, content: &str) {
    std::fs::create_dir(root).unwrap();
    git(root, &["init", "--quiet"]);
    std::fs::write(root.join("answer.txt"), content).unwrap();
    git(root, &["add", "answer.txt"]);
    git(
        root,
        &[
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "base",
        ],
    );
}

fn commit_workspace(root: &std::path::Path) {
    git(root, &["add", "--all"]);
    git(
        root,
        &[
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "external commit",
        ],
    );
}

#[test]
fn workspace_snapshot_invalidates_review_on_external_commit_from_nested_cwd() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("workspace");
    repository(&root, "base\n");
    let nested = root.join("nested");
    std::fs::create_dir(&nested).unwrap();
    let file = nested.join("answer.txt");
    std::fs::write(&file, "base\n").unwrap();
    commit_workspace(&root);
    std::fs::write(&file, "changed\n").unwrap();
    assert_eq!(mew_diff::load_session_diffs(&nested).unwrap().len(), 1);
    let before_root = workspace_snapshot(&root);
    let before_nested = workspace_snapshot(&nested);
    let file_before = std::fs::metadata(&file).unwrap();

    commit_workspace(&root);

    let file_after = std::fs::metadata(&file).unwrap();
    assert_eq!(
        file_before.modified().unwrap(),
        file_after.modified().unwrap()
    );
    assert_eq!(file_before.len(), file_after.len());
    assert!(mew_diff::load_session_diffs(&nested).unwrap().is_empty());
    assert!(!changed_workspace_paths(&before_root, &workspace_snapshot(&root)).is_empty());
    assert!(!changed_workspace_paths(&before_nested, &workspace_snapshot(&nested)).is_empty());
}

#[test]
fn workspace_snapshot_invalidates_review_on_linked_worktree_commit() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("workspace");
    repository(&root, "base\n");
    let worktree = directory.path().join("linked workspace");
    git(
        &root,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "linked",
            worktree.to_str().unwrap(),
        ],
    );
    std::fs::write(worktree.join("answer.txt"), "changed\n").unwrap();
    let before = workspace_snapshot(&worktree);
    assert_eq!(mew_diff::load_session_diffs(&worktree).unwrap().len(), 1);

    commit_workspace(&worktree);

    assert!(mew_diff::load_session_diffs(&worktree).unwrap().is_empty());
    assert!(!changed_workspace_paths(&before, &workspace_snapshot(&worktree)).is_empty());
}

#[test]
fn workspace_snapshot_invalidates_review_on_index_only_change() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("workspace");
    repository(&root, "base\n");
    std::fs::write(root.join("answer.txt"), "changed\n").unwrap();
    git(&root, &["add", "answer.txt"]);
    let before = workspace_snapshot(&root);
    let file_before = std::fs::metadata(root.join("answer.txt")).unwrap();

    git(&root, &["reset", "--quiet", "HEAD", "--", "answer.txt"]);

    let file_after = std::fs::metadata(root.join("answer.txt")).unwrap();
    assert_eq!(
        file_before.modified().unwrap(),
        file_after.modified().unwrap()
    );
    assert!(!changed_workspace_paths(&before, &workspace_snapshot(&root)).is_empty());
}

#[test]
fn workspace_snapshot_invalidates_review_on_packed_ref_and_detached_head_updates() {
    for detached in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("workspace");
        repository(&root, "base\n");
        if detached {
            git(&root, &["checkout", "--quiet", "--detach"]);
        }
        std::fs::write(root.join("answer.txt"), "changed\n").unwrap();
        commit_workspace(&root);
        git(&root, &["pack-refs", "--all"]);
        let previous_head = std::process::Command::new("git")
            .args(["rev-parse", "HEAD~1"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(previous_head.status.success());
        let previous_head = std::str::from_utf8(&previous_head.stdout).unwrap().trim();
        assert!(mew_diff::load_session_diffs(&root).unwrap().is_empty());
        let before = workspace_snapshot(&root);
        let index_before = std::fs::metadata(root.join(".git/index")).unwrap();
        let file_before = std::fs::metadata(root.join("answer.txt")).unwrap();

        git(&root, &["update-ref", "HEAD", previous_head]);

        assert_eq!(
            index_before.modified().unwrap(),
            std::fs::metadata(root.join(".git/index"))
                .unwrap()
                .modified()
                .unwrap()
        );
        assert_eq!(
            file_before.modified().unwrap(),
            std::fs::metadata(root.join("answer.txt"))
                .unwrap()
                .modified()
                .unwrap()
        );
        assert!(!changed_workspace_paths(&before, &workspace_snapshot(&root)).is_empty());
        assert_eq!(mew_diff::load_session_diffs(&root).unwrap().len(), 1);
    }
}

async fn create_session(server: &DaemonServer, root: &std::path::Path) -> Arc<Session> {
    let mut socket = request_tests::connect(server).await;
    request_tests::send(
        &mut socket,
        ClientMessage::NewSession {
            cwd: Some(root.to_string_lossy().into_owned()),
            client_kind: ClientKind::Unknown,
        },
    )
    .await;
    let replies = request_tests::flush(&mut socket).await;
    let session_id = replies
        .into_iter()
        .find_map(|message| match message {
            ServerMessage::SessionReady { session_id, .. } => Some(session_id),
            _ => None,
        })
        .expect("new session must acknowledge its workspace");
    server.session_manager.attach(&session_id).await.unwrap()
}

async fn review(socket: &mut TestSocket, session_id: &str, request_id: &str) -> serde_json::Value {
    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "read_session_diff", "session_id": session_id, "request_id": request_id
            })
            .to_string(),
        ))
        .await
        .unwrap();
    loop {
        let frame = tokio::time::timeout(std::time::Duration::from_secs(3), socket.next())
            .await
            .expect("review response timed out")
            .unwrap()
            .unwrap();
        if let Message::Text(text) = frame {
            let message: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_ne!(
                message["type"], "error",
                "review failures must preserve session and request identity: {message}"
            );
            if message["request_id"] == request_id {
                assert_eq!(message["session_id"], session_id);
                return message;
            }
        }
    }
}

#[tokio::test]
async fn review_reads_selected_daemon_workspace_and_refreshes_same_count_edits() {
    let directory = tempfile::tempdir().unwrap();
    let a = directory.path().join("root-a");
    let b = directory.path().join("selected-b");
    repository(&a, "wrong base\n");
    repository(&b, "selected base\n");
    std::fs::write(a.join("answer.txt"), "wrong workspace\n").unwrap();
    std::fs::write(b.join("answer.txt"), "first edit\n").unwrap();
    let server = server(&directory.path().join("sessions"));
    let other = create_session(&server, &a).await;
    let selected = create_session(&server, &b).await;
    let mut socket = request_tests::connect(&server).await;
    let mut observer = request_tests::connect(&server).await;
    request_tests::send(
        &mut observer,
        ClientMessage::AttachSession {
            session_id: selected.id.clone(),
            client_kind: ClientKind::Unknown,
        },
    )
    .await;
    request_tests::flush(&mut observer).await;
    request_tests::send(
        &mut socket,
        ClientMessage::AttachSession {
            session_id: other.id.clone(),
            client_kind: ClientKind::Unknown,
        },
    )
    .await;
    request_tests::flush(&mut socket).await;

    let first = review(&mut socket, &selected.id, "first").await;
    assert_eq!(first["type"], "session_diff");
    assert_eq!(first["files"][0]["path"], "answer.txt");
    let lines = first["files"][0]["hunks"][0]["lines"].as_array().unwrap();
    assert!(lines.iter().any(|line| line["text"] == "first edit"));
    assert!(!lines.iter().any(|line| line["text"] == "wrong workspace"));
    assert!(
        request_tests::flush(&mut observer).await.is_empty(),
        "review replies are private to the requester"
    );

    std::fs::write(b.join("answer.txt"), "fresh edit\n").unwrap();
    let second = review(&mut socket, &selected.id, "second").await;
    assert_eq!(first["files"][0]["added"], second["files"][0]["added"]);
    assert_eq!(first["files"][0]["removed"], second["files"][0]["removed"]);
    assert_ne!(first["files"][0]["hunks"], second["files"][0]["hunks"]);
    assert!(second["files"][0]["hunks"][0]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .any(|line| line["text"] == "fresh edit"));
}

#[tokio::test]
async fn review_errors_preserve_identity_for_missing_workspace_and_unknown_session() {
    let directory = tempfile::tempdir().unwrap();
    let server = server(&directory.path().join("sessions"));
    let no_workspace = server.session_manager.create(None).await.unwrap();
    let mut socket = request_tests::connect(&server).await;
    for (session_id, request_id) in [
        (no_workspace.id.as_str(), "no-cwd"),
        ("unknown-session", "missing"),
    ] {
        let response = review(&mut socket, session_id, request_id).await;
        assert_eq!(response["type"], "session_diff_error");
        assert!(!response["message"].as_str().unwrap().is_empty());
    }
}

#[tokio::test]
async fn review_reports_repository_failure_instead_of_an_empty_diff() {
    let directory = tempfile::tempdir().unwrap();
    let server = server(&directory.path().join("sessions"));
    let session = create_session(&server, directory.path()).await;
    let mut socket = request_tests::connect(&server).await;
    let response = review(&mut socket, &session.id, "invalid-repo").await;
    assert_eq!(response["type"], "session_diff_error");
    assert!(response["message"]
        .as_str()
        .unwrap()
        .contains("not a git repository"));
}

#[tokio::test]
async fn oversized_review_reports_identified_error_and_keeps_connection_usable() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("workspace");
    repository(&root, "base\n");
    let content = "x".repeat(1024 * 1024 + 64 * 1024);
    for index in 0..8 {
        std::fs::write(root.join(format!("generated-{index}.txt")), &content).unwrap();
    }
    let server = server(&directory.path().join("sessions"));
    let session = create_session(&server, &root).await;
    let mut socket = request_tests::connect(&server).await;
    let response = review(&mut socket, &session.id, "oversized").await;
    assert_eq!(response["type"], "session_diff_error");
    assert!(response["message"].as_str().unwrap().contains("size limit"));
    assert!(request_tests::flush(&mut socket).await.is_empty());
}
