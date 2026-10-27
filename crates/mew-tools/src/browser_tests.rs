use super::*;
#[cfg(target_os = "macos")]
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn advertised_browser_target_requires_exact_id_and_supported_page_type() {
    for kind in ["page", "webview"] {
        let target = serde_json::json!({"id": "owned-target", "type": kind});
        assert!(matches_browser_target(&target, "owned-target"));
        assert!(!matches_browser_target(&target, "other-target"));
    }
    for kind in ["iframe", "browser", "worker", "service_worker", ""] {
        let target = serde_json::json!({"id": "owned-target", "type": kind});
        assert!(!matches_browser_target(&target, "owned-target"));
    }
    for target in [
        serde_json::json!({"type": "webview"}),
        serde_json::json!({"id": "owned-target"}),
        serde_json::json!({"id": 7, "type": "webview"}),
        serde_json::Value::Null,
    ] {
        assert!(!matches_browser_target(&target, "owned-target"));
    }
}

#[test]
fn version_gate_rejects_old_and_unknown_cli() {
    for version in ["agent-browser 0.31.2", "agent-browser 0.33.9", "unknown"] {
        assert!(check_version(version).unwrap_err().contains("0.34"));
    }
    for version in ["agent-browser 0.34.0", "agent-browser 1.0.0"] {
        check_version(version).unwrap();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn failed_process_without_stderr_reports_its_exit_status() {
    let access = BrowserAccess::with_executable(PathBuf::from("/usr/bin/false"));
    let error = access
        .register(
            "owner",
            BrowserTarget {
                page_instance: "page".into(),
                cdp_endpoint: "http://127.0.0.1:1".into(),
                target_id: "target".into(),
                focus_url: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error, "agent-browser: exit status: 1");
    assert!(!access.is_available());
    assert!(!access.is_registered("owner", "page"));
}

#[cfg(unix)]
#[tokio::test]
async fn invalid_process_response_without_stderr_reports_its_exit_status() {
    let output = Command::new("/usr/bin/false").output().await.unwrap();
    assert_eq!(output.status.code(), Some(1));
    let error = serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout).unwrap_err();
    let diagnostic = invalid_command_response(&output, &error);
    assert!(diagnostic.contains("read browser command response:"));
    assert!(diagnostic.contains("exit status: 1"), "{diagnostic}");
}

#[cfg(unix)]
#[tokio::test]
async fn failed_process_preserves_existing_stderr_diagnostics() {
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing-file");
    let missing = missing.to_str().unwrap();
    let output = Command::new("/bin/ls").arg(missing).output().await.unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.trim().is_empty());
    let access = BrowserAccess::with_executable(PathBuf::from("/bin/ls"));
    let lease = access
        .register_pending(
            "owner",
            BrowserTarget {
                page_instance: "page".into(),
                cdp_endpoint: "http://127.0.0.1:1".into(),
                target_id: "target".into(),
                focus_url: None,
            },
        )
        .unwrap();
    let diagnostic = access
        .command(
            &lease,
            &CancellationToken::new(),
            &[missing],
            CommandScope::Unscoped,
        )
        .await
        .unwrap_err();
    assert_eq!(diagnostic, format!("agent-browser: {}", stderr.trim()));
    let error = serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout).unwrap_err();
    assert!(invalid_command_response(&output, &error).ends_with(stderr.trim()));
}

#[cfg(unix)]
#[tokio::test]
async fn invalid_process_response_preserves_signal_termination_without_stderr() {
    let mut child = Command::new("/bin/sleep")
        .arg("60")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    child.start_kill().unwrap();
    let output = child.wait_with_output().await.unwrap();
    assert!(!output.status.success());
    assert!(output.status.code().is_none());
    let error = serde_json::from_slice::<Vec<serde_json::Value>>(&output.stdout).unwrap_err();
    let diagnostic = invalid_command_response(&output, &error);
    assert!(
        diagnostic.contains(&output.status.to_string()),
        "{diagnostic}"
    );
}

#[tokio::test]
async fn registration_rejects_nonlocal_or_implicit_targets() {
    let access = BrowserAccess::default();
    for endpoint in [
        "9223",
        "https://remote.example",
        "file:///tmp/page",
        "http://127.0.0.1:9223@remote.example",
    ] {
        assert!(
            access
                .register(
                    "owner",
                    BrowserTarget {
                        page_instance: "page".into(),
                        cdp_endpoint: endpoint.into(),
                        target_id: "target".into(),
                        focus_url: None,
                    }
                )
                .await
                .is_err(),
            "accepted {endpoint}"
        );
    }
    assert!(!access.is_available());
}

#[test]
fn executable_resolution_prefers_the_packaged_binary() {
    let directory = tempfile::tempdir().unwrap();
    let daemon = directory.path().join("mew");
    assert_eq!(browser_executable(&daemon), PathBuf::from("agent-browser"));
    let packaged = directory.path().join("agent-browser");
    std::fs::write(&packaged, b"binary").unwrap();
    assert_eq!(browser_executable(&daemon), packaged);
}

#[test]
fn pending_registration_is_not_available_and_can_be_revoked_before_preparing() {
    let access = BrowserAccess::default();
    let preparation = access.register(
        "owner",
        BrowserTarget {
            page_instance: "page".into(),
            cdp_endpoint: "http://127.0.0.1:9223".into(),
            target_id: "0123456789ABCDEF0123456789ABCDEF".into(),
            focus_url: None,
        },
    );
    assert!(!access.is_available());
    assert!(access.is_registered("owner", "page"));
    assert!(!access.is_registered("other-owner", "page"));
    assert!(!access.is_registered("owner", "other-page"));
    assert!(access.clone().revoke("owner", "page"));
    assert!(!access.is_registered("owner", "page"));
    drop(preparation);
}

#[cfg(unix)]
mod focus {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct Server {
        url: String,
        requests: tokio::sync::mpsc::UnboundedReceiver<String>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Server {
        async fn start(response: Option<&'static str>) -> Self {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!(
                "http://{}/focus/page-token?generation=one",
                listener.local_addr().unwrap()
            );
            let (send, requests) = tokio::sync::mpsc::unbounded_channel();
            let task = tokio::spawn(async move {
                loop {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    let mut buffer = [0; 1024];
                    while !request.ends_with(b"\r\n\r\n") {
                        let size = socket.read(&mut buffer).await.unwrap();
                        if size == 0 {
                            break;
                        }
                        request.extend_from_slice(&buffer[..size]);
                        assert!(request.len() < 16 * 1024);
                    }
                    send.send(String::from_utf8(request).unwrap()).unwrap();
                    if let Some(response) = response {
                        let _ = socket.write_all(response.as_bytes()).await;
                    } else {
                        let _ = socket.read(&mut buffer).await;
                    }
                }
            });
            Self {
                url,
                requests,
                task,
            }
        }

        async fn request(&mut self) -> String {
            tokio::time::timeout(std::time::Duration::from_secs(2), self.requests.recv())
                .await
                .unwrap()
                .unwrap()
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    fn target(focus_url: Option<String>) -> BrowserTarget {
        BrowserTarget {
            page_instance: "page".into(),
            cdp_endpoint: "http://127.0.0.1:9223".into(),
            target_id: "target".into(),
            focus_url,
        }
    }

    fn ready_access(focus_url: Option<String>) -> (BrowserAccess, Arc<PageLease>) {
        let access = BrowserAccess::with_executable(PathBuf::from("/usr/bin/false"));
        let lease = access.register_pending("owner", target(focus_url)).unwrap();
        // Supply page readiness without simulating a successful CLI process.
        lease
            .bound
            .store(true, std::sync::atomic::Ordering::Relaxed);
        (access, lease)
    }

    #[test]
    fn focus_registration_accepts_only_explicit_local_http_capabilities() {
        for url in [
            "https://127.0.0.1:9223/focus/token",
            "ws://127.0.0.1:9223/focus/token",
            "file:///focus/token",
            "http://remote.example:9223/focus/token",
            "http://127.0.0.2:9223/focus/token",
            "http://localhost/focus/token",
            "http://user@127.0.0.1:9223/focus/token",
            "http://127.0.0.1:9223/focus/token#fragment",
            "http://127.0.0.1:9223",
            "",
        ] {
            let access = BrowserAccess::default();
            let error = access
                .register_pending("owner", target(Some(url.into())))
                .err();
            assert!(
                error.is_some_and(|error| error.contains("focus")),
                "accepted {url}"
            );
            assert!(!access.is_registered("owner", "page"));
        }
        for url in [
            None,
            Some("http://127.0.0.1:9223/focus/token"),
            Some("http://localhost:9223/focus/token"),
            Some("http://[::1]:9223/focus/token"),
        ] {
            BrowserAccess::default()
                .register_pending("owner", target(url.map(String::from)))
                .unwrap();
        }
    }

    #[test]
    fn focus_capability_cannot_change_within_a_page_generation() {
        let access = BrowserAccess::default();
        let first = access
            .register_pending(
                "owner",
                target(Some("http://127.0.0.1:9223/focus/first".into())),
            )
            .unwrap();
        assert!(Arc::ptr_eq(
            &first,
            &access
                .register_pending("owner", first.target.clone())
                .unwrap()
        ));
        assert!(access
            .register_pending(
                "owner",
                target(Some("http://127.0.0.1:9223/focus/second".into()))
            )
            .is_err());
        assert!(!first.revoked.is_cancelled());
    }

    #[tokio::test]
    async fn focus_preflight_posts_exact_capability_without_origin_and_requires_204() {
        let mut server = Server::start(Some(
            "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ))
        .await;
        let (access, lease) = ready_access(Some(server.url.clone()));
        access
            .focus(&lease, &CancellationToken::new())
            .await
            .unwrap();
        let request = server.request().await;
        assert!(
            request.starts_with("POST /focus/page-token?generation=one HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(!request.to_ascii_lowercase().contains("\r\norigin:"));
        assert!(!lease.cli_started.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[tokio::test]
    async fn focus_failure_stops_each_fill_and_press_before_cli() {
        for response in [
            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 302 Found\r\nLocation: http://remote.invalid/focus\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ] {
            let mut server = Server::start(Some(response)).await;
            let (access, lease) = ready_access(Some(server.url.clone()));
            for action in [BrowserAction::Fill { selector: "#input".into(), text: "text".into() }, BrowserAction::Press("Enter".into())] {
                let error = access.execute(action, Some("page"), &CancellationToken::new()).await.unwrap_err();
                assert!(error.contains("browser focus"), "{error}");
                let status = response.split_whitespace().nth(1).unwrap();
                assert!(error.contains(&format!("received {status}")), "{error}");
                assert!(server.request().await.starts_with("POST "));
                assert!(!lease.cli_started.load(std::sync::atomic::Ordering::Relaxed));
            }
        }
    }

    #[tokio::test]
    async fn missing_focus_capability_keeps_legacy_input_behavior() {
        let (access, lease) = ready_access(None);
        let error = access
            .execute(
                BrowserAction::Press("Enter".into()),
                None,
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(!error.contains("focus"), "{error}");
        assert!(lease.cli_started.load(std::sync::atomic::Ordering::Relaxed));
        access.clear();
    }

    #[tokio::test]
    async fn nonkeyboard_actions_do_not_post_focus_capability() {
        let mut server = Server::start(None).await;
        let (access, lease) = ready_access(Some(server.url.clone()));
        let error = access
            .execute(BrowserAction::Snapshot, None, &CancellationToken::new())
            .await
            .unwrap_err();
        assert!(error.contains("exit status: 1"), "{error}");
        assert!(server.requests.try_recv().is_err());
        assert!(lease.cli_started.load(std::sync::atomic::Ordering::Relaxed));
        access.clear();
    }

    #[tokio::test]
    async fn focus_timeout_blocks_input_before_cli() {
        let mut server = Server::start(None).await;
        let (access, lease) = ready_access(Some(server.url.clone()));
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(7),
            access.execute(
                BrowserAction::Press("Enter".into()),
                None,
                &CancellationToken::new(),
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(error.contains("browser focus"), "{error}");
        assert!(server.request().await.starts_with("POST "));
        assert!(!lease.cli_started.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[tokio::test]
    async fn focus_connection_failure_blocks_input_before_cli() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/focus/page-token", listener.local_addr().unwrap());
        drop(listener);
        let (access, lease) = ready_access(Some(url));
        let error = access
            .execute(
                BrowserAction::Press("Enter".into()),
                None,
                &CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(error.contains("browser focus"), "{error}");
        assert!(!lease.cli_started.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[tokio::test]
    async fn cancellation_revocation_and_replacement_stop_inflight_focus_before_cli() {
        for reason in ["cancel", "revoke", "replace"] {
            let mut server = Server::start(None).await;
            let (access, lease) = ready_access(Some(server.url.clone()));
            let cancel = CancellationToken::new();
            let task_access = access.clone();
            let task_cancel = cancel.clone();
            let task = tokio::spawn(async move {
                task_access
                    .execute(
                        BrowserAction::Press("Enter".into()),
                        Some("page"),
                        &task_cancel,
                    )
                    .await
            });
            assert!(server.request().await.starts_with("POST "));
            match reason {
                "cancel" => cancel.cancel(),
                "revoke" => {
                    assert!(access.revoke("owner", "page"));
                }
                "replace" => {
                    let mut replacement =
                        target(Some(server.url.replace("page-token", "new-token")));
                    replacement.page_instance = "replacement".into();
                    access.register_pending("owner", replacement).unwrap();
                }
                _ => unreachable!(),
            }
            let error = tokio::time::timeout(std::time::Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert!(
                error.contains(if reason == "cancel" {
                    "cancelled"
                } else {
                    "revoked"
                }),
                "{error}"
            );
            assert!(!lease.cli_started.load(std::sync::atomic::Ordering::Relaxed));
        }
    }
}

#[cfg(target_os = "macos")]
fn screenshot_files() -> std::collections::BTreeSet<PathBuf> {
    std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("mew-browser-")
        })
        .collect()
}

#[cfg(target_os = "macos")]
struct Chrome {
    child: tokio::process::Child,
    _profile: tempfile::TempDir,
    endpoint: String,
    http: reqwest::Client,
}

#[cfg(target_os = "macos")]
impl Chrome {
    async fn start() -> Self {
        let profile = tempfile::tempdir().unwrap();
        let child = Command::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
            .args([
                "--headless=new",
                "--remote-debugging-port=0",
                "--no-first-run",
                "--no-default-browser-check",
                "about:blank",
            ])
            .arg(format!("--user-data-dir={}", profile.path().display()))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let port = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if let Ok(data) =
                    tokio::fs::read_to_string(profile.path().join("DevToolsActivePort")).await
                {
                    break data.lines().next().unwrap().to_owned();
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        Self {
            child,
            _profile: profile,
            endpoint: format!("http://127.0.0.1:{port}"),
            http: reqwest::Client::new(),
        }
    }

    async fn page(&self, url: &str, instance: &str) -> BrowserTarget {
        let data: serde_json::Value = self
            .http
            .put(format!("{}/json/new?{url}", self.endpoint))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        BrowserTarget {
            page_instance: instance.into(),
            cdp_endpoint: self.endpoint.clone(),
            target_id: data["id"].as_str().unwrap().into(),
            focus_url: None,
        }
    }

    async fn targets(&self) -> Vec<serde_json::Value> {
        self.http
            .get(format!("{}/json/list", self.endpoint))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }
}

#[cfg(target_os = "macos")]
#[tokio::test]
#[ignore = "requires installed Chrome and MEW_BROWSER_TEST_CLI pointing to agent-browser >=0.34"]
async fn real_cdp_same_url_pages_keep_refs_and_revoked_commands_isolated() {
    let mut chrome = Chrome::start().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let slow_started = Arc::new(tokio::sync::Notify::new());
    let slow_release = Arc::new(tokio::sync::Notify::new());
    let started = slow_started.clone();
    let release = slow_release.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let started = started.clone();
            let release = release.clone();
            tokio::spawn(async move {
                let mut request = [0; 2048];
                let size = socket.read(&mut request).await.unwrap();
                if size == 0 {
                    return;
                }
                if request[..size].starts_with(b"GET /slow ") {
                    started.notify_one();
                    release.notified().await;
                }
                let body = r#"<html><title>same page</title><body><button onclick="this.textContent='clicked'">click me</button><input aria-label="name" onkeydown="if(event.key==='Enter')document.querySelector('button').textContent=this.value"></body></html>"#;
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                if let Err(error) = socket.write_all(response.as_bytes()).await {
                    assert!(
                        matches!(
                            error.kind(),
                            std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset
                        ),
                        "fixture response: {error}"
                    );
                }
            });
        }
    });
    let executable =
        PathBuf::from(std::env::var("MEW_BROWSER_TEST_CLI").expect("set compatible CLI path"));
    let a = BrowserAccess::with_executable(executable.clone());
    let b = BrowserAccess::with_executable(executable);
    let target_a = chrome.page(&url, "page-a").await;
    let target_b = chrome.page(&url, "page-b").await;
    let target_ids = |targets: Vec<serde_json::Value>| {
        targets
            .into_iter()
            .filter(|target| target["type"] == "page")
            .map(|target| target["id"].as_str().unwrap().to_owned())
            .collect::<std::collections::BTreeSet<_>>()
    };
    let original_targets = target_ids(chrome.targets().await);
    let cancel = CancellationToken::new();
    a.register("owner-a", target_a.clone()).await.unwrap();
    b.register("owner-b", target_b.clone()).await.unwrap();
    assert_eq!(
        target_ids(chrome.targets().await),
        original_targets,
        "registering existing pages must not create browser tabs"
    );
    assert!(a.clone().is_available());
    assert!(a.register("owner-b", target_b.clone()).await.is_err());
    let snapshot = a
        .execute(BrowserAction::Snapshot, None, &cancel)
        .await
        .unwrap();
    assert_eq!(target_ids(chrome.targets().await), original_targets);
    let data: serde_json::Value = serde_json::from_str(&snapshot.output).unwrap();
    assert!(a
        .execute(
            BrowserAction::Click("button".into()),
            Some("different-page"),
            &cancel
        )
        .await
        .unwrap_err()
        .contains("different"));
    let refs = data["data"]["refs"].as_object().expect("snapshot refs");
    let button = refs
        .iter()
        .find(|(_, info)| info["role"] == "button")
        .unwrap()
        .0;
    // Idempotent registration and state reads cannot invalidate semantic refs.
    a.register("owner-a", target_a.clone()).await.unwrap();
    a.execute(BrowserAction::Click(format!("@{button}")), None, &cancel)
        .await
        .unwrap();
    assert!(a
        .execute(BrowserAction::Snapshot, None, &cancel)
        .await
        .unwrap()
        .output
        .contains("clicked"));
    assert!(b
        .execute(BrowserAction::Snapshot, None, &cancel)
        .await
        .unwrap()
        .output
        .contains("click me"));
    assert!(a
        .execute(
            BrowserAction::Open("file:///etc/passwd".into()),
            None,
            &cancel
        )
        .await
        .unwrap_err()
        .contains("HTTP"));
    a.execute(
        BrowserAction::Fill {
            selector: "input".into(),
            text: "entered value".into(),
        },
        None,
        &cancel,
    )
    .await
    .unwrap();
    a.execute(BrowserAction::Press("Enter".into()), None, &cancel)
        .await
        .unwrap();
    assert!(a
        .execute(BrowserAction::Snapshot, None, &cancel)
        .await
        .unwrap()
        .output
        .contains("entered value"));
    for text in [
        "--json",
        "--session",
        "--pin-tab",
        "quotes \"and\"\nnewlines",
    ] {
        a.execute(
            BrowserAction::Fill {
                selector: "input".into(),
                text: text.into(),
            },
            None,
            &cancel,
        )
        .await
        .unwrap();
        a.execute(BrowserAction::Press("Enter".into()), None, &cancel)
            .await
            .unwrap();
        let snapshot = a
            .execute(BrowserAction::Snapshot, None, &cancel)
            .await
            .unwrap();
        let snapshot: serde_json::Value = serde_json::from_str(&snapshot.output).unwrap();
        assert!(
            snapshot["data"]["snapshot"]
                .as_str()
                .unwrap()
                .contains(text.split('\n').next().unwrap()),
            "literal fill lost {text:?}: {snapshot}"
        );
    }
    let neighbor_session =
        b.0.page
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .cli_session
            .clone();
    assert!(a
        .execute(
            BrowserAction::Fill {
                selector: "--session".into(),
                text: neighbor_session,
            },
            None,
            &cancel
        )
        .await
        .is_err());
    assert!(a
        .execute(BrowserAction::Click("--help".into()), None, &cancel)
        .await
        .is_err());
    assert!(b
        .execute(BrowserAction::Snapshot, None, &cancel)
        .await
        .unwrap()
        .output
        .contains("click me"));
    let screenshots_before = screenshot_files();
    let screenshot = a
        .execute(BrowserAction::Screenshot { annotate: false }, None, &cancel)
        .await
        .unwrap();
    assert!(screenshot
        .screenshot
        .unwrap()
        .starts_with(b"\x89PNG\r\n\x1a\n"));
    let shared = Arc::new(crate::ToolCtxShared {
        browser_access: a.clone(),
        ..Default::default()
    });
    let tool_context = || {
        crate::ToolCtx::new(
            shared.clone(),
            "browser-test".into(),
            CancellationToken::new(),
            tokio::sync::mpsc::channel(1).0,
        )
    };
    let browser_tools = crate::tools::browser::tools();
    let snapshot_tool = browser_tools
        .iter()
        .find(|tool| tool.name() == "browser_snapshot")
        .unwrap();
    let tool_output = snapshot_tool
        .execute(tool_context(), serde_json::json!({}))
        .await
        .unwrap();
    assert!(tool_output.output.starts_with("untrusted browser content:"));
    let screenshot_tool = browser_tools
        .iter()
        .find(|tool| tool.name() == "browser_screenshot")
        .unwrap();
    let tool_output = screenshot_tool
        .execute(tool_context(), serde_json::json!({}))
        .await
        .unwrap();
    let metadata = tool_output.metadata.unwrap();
    assert_eq!(metadata["mime"], "image/png");
    let bytes = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        metadata["data"].as_str().unwrap(),
    )
    .unwrap();
    assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(screenshot_files(), screenshots_before);
    let destination = format!("{url}?next");
    let opened = a
        .execute(BrowserAction::Open(destination.clone()), None, &cancel)
        .await
        .unwrap();
    assert_eq!(opened.url, destination);
    assert!(b
        .execute(BrowserAction::Snapshot, None, &cancel)
        .await
        .unwrap()
        .output
        .contains("click me"));
    let operation_cancel = CancellationToken::new();
    let cancelled = operation_cancel.clone();
    let access = a.clone();
    let slow_url = format!("{url}slow");
    let active = tokio::spawn(async move {
        access
            .execute(BrowserAction::Open(slow_url), None, &cancelled)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), slow_started.notified())
        .await
        .unwrap();
    operation_cancel.cancel();
    assert!(active.await.unwrap().unwrap_err().contains("cancelled"));
    slow_release.notify_one();
    let lock = a.0.operation.lock().await;
    let clone = a.clone();
    let queued = tokio::spawn(async move {
        clone
            .execute(BrowserAction::Snapshot, None, &CancellationToken::new())
            .await
    });
    tokio::task::yield_now().await;
    assert!(a.revoke("owner-a", "page-a"));
    assert!(snapshot_tool
        .execute(tool_context(), serde_json::json!({}))
        .await
        .unwrap_err()
        .to_string()
        .contains("unavailable"));
    drop(lock);
    assert!(queued.await.unwrap().unwrap_err().contains("revoked"));
    a.register("owner-a", target_b.clone()).await.unwrap();
    assert!(!a.revoke("owner-a", "page-a"));
    a.execute(BrowserAction::Close, None, &cancel)
        .await
        .unwrap();
    assert!(!a.is_available());
    let targets = chrome.targets().await;
    assert!(targets
        .iter()
        .any(|target| target["id"] == target_a.target_id));
    assert!(!targets
        .iter()
        .any(|target| target["id"] == target_b.target_id));
    assert!(b
        .execute(BrowserAction::Click("button".into()), None, &cancel)
        .await
        .is_err());
    // A second owned generation still closes only its exact target.
    let mut last = target_a.clone();
    last.page_instance = "page-last".into();
    a.register("owner-a", last).await.unwrap();
    a.execute(BrowserAction::Close, None, &cancel)
        .await
        .unwrap();
    let final_targets = chrome.targets().await;
    assert!(!final_targets
        .iter()
        .any(|target| target["id"] == target_a.target_id));
    assert_eq!(
        target_ids(final_targets),
        original_targets
            .into_iter()
            .filter(|id| id != &target_a.target_id && id != &target_b.target_id)
            .collect(),
        "closing owned pages must not create replacement tabs"
    );
    assert!(!a.is_available());
    assert!(a
        .execute(BrowserAction::Snapshot, None, &cancel)
        .await
        .is_err());
    let old = BrowserAccess::with_executable(PathBuf::from("/opt/homebrew/bin/agent-browser"));
    assert!(old
        .register("old", target_a)
        .await
        .unwrap_err()
        .contains("0.34"));
    assert!(!old.is_available());
    server.abort();
    chrome.child.kill().await.unwrap();
}
