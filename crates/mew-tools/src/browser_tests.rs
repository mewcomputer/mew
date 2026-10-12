use super::*;
#[cfg(target_os = "macos")]
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn version_gate_rejects_old_and_unknown_cli() {
    for version in ["agent-browser 0.31.2", "agent-browser 0.33.9", "unknown"] {
        assert!(check_version(version).unwrap_err().contains("0.34"));
    }
    for version in ["agent-browser 0.34.0", "agent-browser 1.0.0"] {
        check_version(version).unwrap();
    }
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
    let cancel = CancellationToken::new();
    a.register("owner-a", target_a.clone()).await.unwrap();
    b.register("owner-b", target_b.clone()).await.unwrap();
    assert!(a.clone().is_available());
    assert!(a.register("owner-b", target_b.clone()).await.is_err());
    let snapshot = a
        .execute(BrowserAction::Snapshot, None, &cancel)
        .await
        .unwrap();
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
