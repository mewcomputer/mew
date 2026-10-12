//! A revocable native page shared by a session and all of its agent clones.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserTarget {
    pub page_instance: String,
    pub cdp_endpoint: String,
    pub target_id: String,
}

pub enum BrowserAction {
    Open(String),
    Snapshot,
    Screenshot { annotate: bool },
    Click(String),
    Fill { selector: String, text: String },
    Press(String),
    Close,
}

#[derive(Default, Debug)]
pub struct BrowserOutput {
    pub output: String,
    pub url: String,
    pub title: String,
    pub screenshot: Option<Vec<u8>>,
}

struct PageLease {
    owner: String,
    target: BrowserTarget,
    cli_session: String,
    revoked: CancellationToken,
    bound: std::sync::atomic::AtomicBool,
    cli_started: std::sync::atomic::AtomicBool,
}

struct BrowserState {
    page: Mutex<Option<Arc<PageLease>>>,
    operation: tokio::sync::Mutex<()>,
    executable: PathBuf,
}

#[derive(Clone)]
pub struct BrowserAccess(Arc<BrowserState>);

impl Default for BrowserAccess {
    fn default() -> Self {
        let executable = std::env::current_exe()
            .map(|path| browser_executable(&path))
            .unwrap_or_else(|_| PathBuf::from("agent-browser"));
        Self::with_executable(executable)
    }
}

impl BrowserAccess {
    pub fn with_executable(executable: PathBuf) -> Self {
        Self(Arc::new(BrowserState {
            page: Mutex::new(None),
            operation: tokio::sync::Mutex::new(()),
            executable,
        }))
    }

    fn register_pending(
        &self,
        owner: &str,
        target: BrowserTarget,
    ) -> Result<Arc<PageLease>, String> {
        let endpoint = reqwest::Url::parse(&target.cdp_endpoint)
            .map_err(|_| "browser CDP endpoint must be an explicit local URL".to_owned())?;
        if !matches!(endpoint.scheme(), "http" | "ws")
            || !matches!(
                endpoint.host_str(),
                Some("127.0.0.1" | "localhost" | "[::1]")
            )
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.port().is_none()
            || target.page_instance.is_empty()
            || target.target_id.is_empty()
            || owner.is_empty()
        {
            return Err(
                "browser registration requires a local CDP endpoint and explicit page identity"
                    .into(),
            );
        }
        let mut page = self.0.page.lock().expect("browser page lock");
        if let Some(previous) = page.as_ref() {
            if previous.owner != owner {
                return Err(
                    "this conversation's browser is owned by another desktop connection".into(),
                );
            }
            if previous.target == target {
                return Ok(previous.clone());
            }
            if previous.target.page_instance == target.page_instance {
                return Err("a page generation cannot change its CDP identity".into());
            }
            self.disconnect_cli(previous);
        }
        let lease = Arc::new(PageLease {
            owner: owner.into(),
            target,
            cli_session: format!("mew-page-{}", ulid::Ulid::new()),
            revoked: CancellationToken::new(),
            bound: std::sync::atomic::AtomicBool::new(false),
            cli_started: std::sync::atomic::AtomicBool::new(false),
        });
        *page = Some(lease.clone());
        Ok(lease)
    }

    pub fn register(
        &self,
        owner: &str,
        target: BrowserTarget,
    ) -> impl std::future::Future<Output = Result<(), String>> + Send + 'static {
        // Create the pending lease now so receiver order survives task scheduling.
        let pending = self.register_pending(owner, target);
        let access = self.clone();
        async move {
            let lease = pending?;
            let result = access.prepare(&lease).await;
            if result.is_err() {
                access.revoke_matching(|current| Arc::ptr_eq(current, &lease));
            }
            result
        }
    }

    async fn prepare(&self, lease: &PageLease) -> Result<(), String> {
        let _operation = tokio::select! {
            biased;
            _ = lease.revoked.cancelled() => return Err("browser page registration was revoked".into()),
            lock = self.0.operation.lock() => lock,
        };
        if lease.bound.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
        let cancel = CancellationToken::new();
        let version = self.command(lease, &cancel, &["--version"], false).await?;
        check_version(&version)?;
        let targets: Vec<serde_json::Value> = self
            .cdp_get(lease, &cancel, "list")
            .await?
            .json()
            .await
            .map_err(|error| format!("read browser targets: {error}"))?;
        if !targets.iter().any(|target| {
            target["id"].as_str() == Some(&lease.target.target_id) && target["type"] == "page"
        }) {
            return Err("registered browser page no longer exists".into());
        }
        // Tab selection clears snapshot refs, so bind only once per page lease.
        self.command(lease, &cancel, &["tab", &lease.target.target_id], true)
            .await?;
        if lease.revoked.is_cancelled() {
            return Err("browser page registration was revoked".into());
        }
        lease
            .bound
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    pub fn revoke(&self, owner: &str, page_instance: &str) -> bool {
        self.revoke_matching(|page| {
            page.owner == owner && page.target.page_instance == page_instance
        })
    }

    pub fn disconnect(&self, owner: &str) -> bool {
        self.revoke_matching(|page| page.owner == owner)
    }

    pub fn clear(&self) {
        self.revoke_matching(|_| true);
    }

    fn revoke_matching(&self, matches: impl FnOnce(&Arc<PageLease>) -> bool) -> bool {
        let mut page = self.0.page.lock().expect("browser page lock");
        if page.as_ref().is_some_and(matches) {
            let lease = page.take().expect("matching page");
            self.disconnect_cli(&lease);
            true
        } else {
            false
        }
    }

    fn disconnect_cli(&self, lease: &PageLease) {
        lease.revoked.cancel();
        if !lease
            .cli_started
            .swap(false, std::sync::atomic::Ordering::Relaxed)
        {
            return;
        }
        let executable = self.0.executable.clone();
        let session = lease.cli_session.clone();
        let endpoint = lease.target.cdp_endpoint.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    Command::new(executable)
                        .env("AGENT_BROWSER_IDLE_TIMEOUT_MS", "300000")
                        .args([
                            "--session",
                            &session,
                            "--cdp",
                            &endpoint,
                            "--pin-tab",
                            "close",
                        ])
                        .kill_on_drop(true)
                        .output(),
                )
                .await;
            });
        }
    }

    pub fn is_available(&self) -> bool {
        self.0
            .page
            .lock()
            .expect("browser page lock")
            .as_ref()
            .is_some_and(|lease| lease.bound.load(std::sync::atomic::Ordering::Relaxed))
    }

    pub fn is_registered(&self, owner: &str, page_instance: &str) -> bool {
        self.0
            .page
            .lock()
            .expect("browser page lock")
            .as_ref()
            .is_some_and(|lease| {
                lease.owner == owner && lease.target.page_instance == page_instance
            })
    }

    pub fn page_instance(&self) -> Option<String> {
        self.0
            .page
            .lock()
            .expect("browser page lock")
            .as_ref()
            .map(|lease| lease.target.page_instance.clone())
    }

    pub async fn execute(
        &self,
        action: BrowserAction,
        page_instance: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<BrowserOutput, String> {
        // Capture before queuing: old commands must never move onto a replacement page.
        let lease = self
            .0
            .page
            .lock()
            .expect("browser page lock")
            .clone()
            .ok_or(
                "in-app browser is unavailable; open this conversation in the mew desktop app",
            )?;
        if page_instance.is_some_and(|expected| lease.target.page_instance != expected) {
            return Err("browser operation belongs to a different page generation".into());
        }
        let _operation = tokio::select! {
            biased;
            _ = lease.revoked.cancelled() => return Err("browser page registration was revoked".into()),
            _ = cancel.cancelled() => return Err("browser operation cancelled".into()),
            lock = self.0.operation.lock() => lock,
        };
        if !lease.bound.load(std::sync::atomic::Ordering::Relaxed) {
            return Err("browser page is not ready".into());
        }
        let mut screenshot = None;
        let output = match action {
            BrowserAction::Open(url) => {
                let parsed = reqwest::Url::parse(&url)
                    .map_err(|_| "browser navigation requires an HTTP(S) URL")?;
                if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
                    return Err("browser navigation only supports HTTP(S) URLs".into());
                }
                self.data_command(&lease, cancel, &["open", &url]).await?
            }
            BrowserAction::Snapshot => {
                self.command(&lease, cancel, &["snapshot", "--json"], true)
                    .await?
            }
            BrowserAction::Click(selector) => {
                self.data_command(&lease, cancel, &["click", &selector])
                    .await?
            }
            BrowserAction::Fill { selector, text } => {
                self.data_command(&lease, cancel, &["fill", &selector, &text])
                    .await?
            }
            BrowserAction::Press(key) => {
                self.data_command(&lease, cancel, &["press", &key]).await?
            }
            BrowserAction::Close => {
                self.cdp_get(&lease, cancel, &format!("close/{}", lease.target.target_id))
                    .await?;
                self.revoke(&lease.owner, &lease.target.page_instance);
                return Ok(BrowserOutput::default());
            }
            BrowserAction::Screenshot { annotate } => {
                let path =
                    std::env::temp_dir().join(format!("mew-browser-{}.png", ulid::Ulid::new()));
                let path_text = path.to_string_lossy();
                let mut args = vec!["screenshot", path_text.as_ref()];
                if annotate {
                    args.push("--annotate");
                }
                let result = self.command(&lease, cancel, &args, true).await;
                let bytes = match result {
                    Ok(_) => tokio::fs::read(&path)
                        .await
                        .map_err(|e| format!("read browser screenshot: {e}")),
                    Err(error) => Err(error),
                };
                let _ = tokio::fs::remove_file(&path).await;
                screenshot = Some(bytes?);
                String::new()
            }
        };
        let url = self.command(&lease, cancel, &["get", "url"], true).await?;
        let title = self
            .command(&lease, cancel, &["get", "title"], true)
            .await?;
        Ok(BrowserOutput {
            output,
            url,
            title,
            screenshot,
        })
    }

    async fn cdp_get(
        &self,
        lease: &PageLease,
        cancel: &CancellationToken,
        resource: &str,
    ) -> Result<reqwest::Response, String> {
        let mut url = reqwest::Url::parse(&lease.target.cdp_endpoint).expect("validated endpoint");
        url.set_scheme("http")
            .map_err(|_| "invalid local CDP endpoint".to_owned())?;
        url.set_path(&format!("/json/{resource}"));
        url.set_query(None);
        url.set_fragment(None);
        tokio::select! {
            biased;
            _ = lease.revoked.cancelled() => Err("browser page registration was revoked".into()),
            _ = cancel.cancelled() => Err("browser operation cancelled".into()),
            response = reqwest::Client::new().get(url).timeout(std::time::Duration::from_secs(5)).send() => {
                response.and_then(reqwest::Response::error_for_status).map_err(|error| format!("native browser endpoint: {error}"))
            },
        }
    }

    async fn command(
        &self,
        lease: &PageLease,
        cancel: &CancellationToken,
        args: &[&str],
        scoped: bool,
    ) -> Result<String, String> {
        let output = self.run_command(lease, cancel, args, scoped, None).await?;
        if !output.status.success() {
            return Err(format!(
                "agent-browser: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    async fn data_command(
        &self,
        lease: &PageLease,
        cancel: &CancellationToken,
        args: &[&str],
    ) -> Result<String, String> {
        // Batch stdin bypasses the CLI's global option scanner for page data.
        let input = serde_json::to_vec(&[args]).expect("browser command strings serialize");
        let output = self
            .run_command(lease, cancel, &["--json", "batch"], true, Some(input))
            .await?;
        let responses: Vec<serde_json::Value> =
            serde_json::from_slice(&output.stdout).map_err(|error| {
                format!(
                    "read browser command response: {error}; {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                )
            })?;
        let response = responses
            .first()
            .filter(|_| responses.len() == 1)
            .ok_or_else(|| "browser command returned no single result".to_owned())?;
        if !output.status.success() || response["success"] != true {
            return Err(format!(
                "agent-browser: {}",
                response["error"]
                    .as_str()
                    .unwrap_or("browser command failed")
            ));
        }
        Ok(response["result"].to_string())
    }

    async fn run_command(
        &self,
        lease: &PageLease,
        cancel: &CancellationToken,
        args: &[&str],
        scoped: bool,
        input: Option<Vec<u8>>,
    ) -> Result<std::process::Output, String> {
        let mut command = Command::new(&self.0.executable);
        command.kill_on_drop(true);
        command.env("AGENT_BROWSER_IDLE_TIMEOUT_MS", "300000");
        if scoped {
            lease
                .cli_started
                .store(true, std::sync::atomic::Ordering::Relaxed);
            command.args([
                "--session",
                &lease.cli_session,
                "--cdp",
                &lease.target.cdp_endpoint,
                "--pin-tab",
            ]);
        }
        command.args(args);
        let process = async move {
            if let Some(input) = input {
                use tokio::io::AsyncWriteExt;
                command
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());
                let mut child = command.spawn()?;
                if let Some(mut stdin) = child.stdin.take() {
                    stdin.write_all(&input).await?;
                    stdin.shutdown().await?;
                }
                child.wait_with_output().await
            } else {
                command.output().await
            }
        };
        tokio::select! {
            biased;
            _ = lease.revoked.cancelled() => Err("browser page registration was revoked".into()),
            _ = cancel.cancelled() => Err("browser operation cancelled".into()),
            result = tokio::time::timeout(std::time::Duration::from_secs(30), process) => result
                .map_err(|_| "browser operation timed out".to_owned())?
                .map_err(|e| format!("run agent-browser: {e}; install agent-browser 0.34 or newer")),
        }
    }
}

fn check_version(output: &str) -> Result<(), String> {
    let parsed = output.split_whitespace().last().and_then(|version| {
        let mut numbers = version.trim_start_matches('v').split('.');
        Some((
            numbers.next()?.parse::<u32>().ok()?,
            numbers.next()?.parse::<u32>().ok()?,
        ))
    });
    if !parsed.is_some_and(|version| version >= (0, 34)) {
        return Err(format!("incompatible agent-browser version '{output}'; native page isolation requires 0.34 or newer"));
    }
    Ok(())
}

fn browser_executable(daemon: &std::path::Path) -> PathBuf {
    daemon
        .parent()
        .map(|directory| directory.join("agent-browser"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| PathBuf::from("agent-browser"))
}

#[cfg(test)]
#[path = "browser_tests.rs"]
mod tests;
