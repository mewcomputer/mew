use std::{
    io,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

use async_trait::async_trait;
#[cfg(target_os = "macos")]
use mew_agy::sandbox::SeatbeltBoundary;
use mew_agy::{
    launch::{self, Capabilities},
    mcp::McpServer,
    process::{IsolationBoundary, LaunchSpec, ManagedProcess, ProcessOutcome, UnsandboxedBoundary},
    provider::{AgyProvider, Config},
    proxy::EgressProxy,
};
use mew_provider::{EventStream, Provider, ProviderError, Request};
use tokio::io::AsyncReadExt;
use tokio::sync::OnceCell;

pub(crate) struct ModelCache {
    models: std::sync::RwLock<Vec<mew_protocol::ModelInfo>>,
    refreshing: std::sync::atomic::AtomicBool,
    isolated: bool,
}

impl ModelCache {
    pub(crate) fn new(isolated: bool) -> Self {
        Self {
            models: Default::default(),
            refreshing: Default::default(),
            isolated,
        }
    }
    pub(crate) fn refresh(self: &std::sync::Arc<Self>) {
        use std::sync::atomic::Ordering;
        let Some(executable) =
            std::env::var_os("PATH").and_then(|path| super::providers::find_agy_on_path(&path))
        else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        if self.refreshing.swap(true, Ordering::AcqRel) {
            return;
        }
        let cache = self.clone();
        runtime.spawn(async move {
            match discover_models(executable, cache.isolated).await {
                Ok(models) => *cache.models.write().unwrap_or_else(|e| e.into_inner()) = models,
                Err(_) => {
                    cache
                        .models
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .clear();
                    tracing::warn!(
                        "Antigravity model discovery failed; check its sign-in and sandbox setup"
                    );
                }
            }
            cache.refreshing.store(false, Ordering::Release);
        });
    }

    pub(crate) fn models(&self) -> Vec<mew_protocol::ModelInfo> {
        self.models
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

pub(super) struct LocalAgyProvider {
    executable: PathBuf,
    model: String,
    isolated: bool,
    ready: OnceCell<Ready>,
}

struct Ready {
    provider: AgyProvider<LocalBoundary>,
    proxy: Option<EgressProxy>,
    workspace_parent: PathBuf,
}

impl LocalAgyProvider {
    pub(super) fn new(executable: PathBuf, model: String, isolated: bool) -> Self {
        Self {
            executable,
            model,
            isolated,
            ready: OnceCell::new(),
        }
    }

    async fn ready(&self) -> Result<&Ready, ProviderError> {
        self.ready
            .get_or_try_init(|| async {
                let workspace_parent = setup_workspaces(&mew_config::data_dir())?;
                let (boundary, proxy) = local_boundary(self.isolated).await?;
                let provider = AgyProvider::new(
                    Config {
                        executable: self.executable.clone(),
                        workspace_parent: workspace_parent.clone(),
                        environment: Vec::new(),
                        lifetime: Duration::from_secs(3600),
                    },
                    boundary,
                );
                Ok(Ready {
                    provider,
                    proxy,
                    workspace_parent,
                })
            })
            .await
    }
}

enum LocalBoundary {
    #[cfg(target_os = "macos")]
    Isolated(SeatbeltBoundary),
    Unsandboxed(UnsandboxedBoundary),
}

impl IsolationBoundary for LocalBoundary {
    fn wrap(&self, spec: LaunchSpec, workspace: &Path) -> io::Result<LaunchSpec> {
        match self {
            #[cfg(target_os = "macos")]
            Self::Isolated(boundary) => boundary.wrap(spec, workspace),
            Self::Unsandboxed(boundary) => boundary.wrap(spec, workspace),
        }
    }
}

async fn local_boundary(isolated: bool) -> io::Result<(LocalBoundary, Option<EgressProxy>)> {
    if !isolated {
        return Ok((LocalBoundary::Unsandboxed(UnsandboxedBoundary), None));
    }
    #[cfg(target_os = "macos")]
    {
        let proxy = EgressProxy::start().await?;
        let boundary = SeatbeltBoundary::new(proxy.port())?;
        Ok((LocalBoundary::Isolated(boundary), Some(proxy)))
    }
    #[cfg(not(target_os = "macos"))]
    Err(io::Error::other(
        "Antigravity isolation is unavailable on this platform",
    ))
}

fn setup_workspaces(data: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(data)?;
    let root = data.join("antigravity");
    private_dir(&root)?;
    let workspaces = root.join("workspaces");
    private_dir(&workspaces)?;
    Ok(workspaces)
}

fn login_command(executable: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(executable);
    command.args(["acp", "--login"]);
    command
}

async fn run_login(command: &mut tokio::process::Command) -> io::Result<()> {
    let status = command.status().await?;
    if !status.success() {
        return Err(io::Error::other(format!("AGY sign-in failed ({status})")));
    }
    Ok(())
}

pub(crate) async fn login() -> anyhow::Result<()> {
    let executable = std::env::var_os("PATH")
        .and_then(|path| super::providers::find_agy_on_path(&path))
        .ok_or_else(|| anyhow::anyhow!("agy executable not found on PATH"))?;
    eprintln!("Signing in through the official AGY CLI.");
    run_login(&mut login_command(&executable)).await?;
    Ok(())
}

pub(super) async fn discover_models(
    executable: PathBuf,
    isolated: bool,
) -> Result<Vec<mew_protocol::ModelInfo>, ProviderError> {
    let local = LocalAgyProvider::new(executable.clone(), String::new(), isolated);
    let ready = local.ready().await?;
    let (mut server, _calls) = McpServer::bind(Vec::new(), 1)
        .await
        .map_err(ProviderError::Message)?;
    let launch = launch::prepare(
        &ready.workspace_parent,
        "models",
        &[],
        server.url(),
        Capabilities::default(),
        "",
    )?;
    let cwd = launch.workspace().to_owned();
    let (workspace, _) = launch.into_parts();
    let boundary = match &ready.proxy {
        #[cfg(target_os = "macos")]
        Some(proxy) => LocalBoundary::Isolated(SeatbeltBoundary::new(proxy.port())?),
        #[cfg(not(target_os = "macos"))]
        Some(_) => {
            return Err(ProviderError::Message(
                "Antigravity isolation is unavailable on this platform".into(),
            ))
        }
        None if !isolated => LocalBoundary::Unsandboxed(UnsandboxedBoundary),
        None => {
            return Err(ProviderError::Message(
                "Antigravity isolation is unavailable".into(),
            ))
        }
    };
    let mut process = ManagedProcess::spawn(
        LaunchSpec {
            program: executable,
            args: vec!["models".into()],
            env: Vec::new(),
            cwd,
        },
        &boundary,
        workspace,
        Duration::from_secs(30),
    )?;
    drop(process.take_stdin());
    let stdout = process
        .take_stdout()
        .ok_or_else(|| io::Error::other("AGY output unavailable"))?;
    let result = tokio::time::timeout(Duration::from_secs(25), async {
        let mut output = Vec::new();
        stdout.take(65_537).read_to_end(&mut output).await?;
        if output.len() > 65_536 {
            return Err(io::Error::other("AGY model list is too large"));
        }
        match process.wait().await? {
            ProcessOutcome::Exited(status) if status.success() => parse_models(&output),
            _ => Err(io::Error::other("AGY model discovery failed")),
        }
    })
    .await;
    if !matches!(result, Ok(Ok(_))) {
        let _ = process.cancel().await;
    }
    server.shutdown().await?;
    result
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "AGY model discovery timed out"))?
        .map_err(ProviderError::from)
}

fn parse_models(output: &[u8]) -> io::Result<Vec<mew_protocol::ModelInfo>> {
    let output =
        std::str::from_utf8(output).map_err(|_| io::Error::other("invalid AGY model list"))?;
    let mut seen = std::collections::HashSet::new();
    Ok(output
        .lines()
        .filter_map(|line| {
            let (id, name) = line.split_once('\t')?;
            if id.is_empty()
                || id.len() > 128
                || !id.as_bytes()[0].is_ascii_alphanumeric()
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
                || name.is_empty()
                || name.len() > 256
                || name.chars().any(char::is_control)
                || !seen.insert(id)
            {
                return None;
            }
            Some(mew_protocol::ModelInfo {
                id: format!("antigravity/{id}"),
                provider: "antigravity".into(),
                model: id.into(),
                display_name: Some(name.into()),
                description: None,
                thinking_variants: Vec::new(),
                thinking_budget: None,
                context_window: None,
            })
        })
        .collect())
}

#[async_trait]
impl Provider for LocalAgyProvider {
    fn name(&self) -> &str {
        "antigravity"
    }

    async fn stream(&self, mut request: Request) -> Result<EventStream, ProviderError> {
        if request.model.is_empty() {
            request.model.clone_from(&self.model);
        }
        self.ready().await?.provider.stream(request).await
    }
}

fn private_dir(path: &Path) -> io::Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "AGY data directory must be private and owned by the current user",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    #[tokio::test]
    async fn disabling_isolation_explicitly_selects_direct_launch_without_proxy() {
        let (boundary, proxy) = local_boundary(false).await.unwrap();
        assert!(matches!(boundary, LocalBoundary::Unsandboxed(_)));
        assert!(proxy.is_none());
    }

    #[cfg(not(target_os = "macos"))]
    #[tokio::test]
    async fn unsupported_isolation_does_not_fall_back_to_direct_launch() {
        assert!(local_boundary(true).await.is_err());
    }

    #[test]
    fn profile_directory_is_private_and_rejects_symlinks_or_public_directories() {
        let root = tempfile::tempdir().unwrap();
        let private = root.path().join("profile");
        private_dir(&private).unwrap();
        assert_eq!(
            std::fs::metadata(&private).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(private_dir(&private).is_ok());
        let link = root.path().join("link");
        symlink(&private, &link).unwrap();
        assert!(private_dir(&link).is_err());
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_dir(&private).is_err());
    }

    #[test]
    fn workspace_setup_does_not_create_an_agy_profile() {
        let root = tempfile::tempdir().unwrap();
        let workspace = setup_workspaces(root.path()).unwrap();
        assert_eq!(workspace, root.path().join("antigravity/workspaces"));
        assert!(!root.path().join("antigravity/profile").exists());
        assert_eq!(
            std::fs::metadata(workspace).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn parses_real_agy_models_output_and_ignores_malformed_lines() {
        let input = b"gemini-4-neon-high\tGemini 4 Neon (High)\ngemini-4-argon-low\tGemini 4 Argon (Low)\ninvalid id\tbad\ngemini-4-neon-high\tduplicate\n";
        let models = parse_models(input).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "antigravity/gemini-4-neon-high");
        assert_eq!(
            models[0].display_name.as_deref(),
            Some("Gemini 4 Neon (High)")
        );
        assert_eq!(models[1].model, "gemini-4-argon-low");
        assert!(parse_models(&[0xff]).is_err());
    }

    #[test]
    fn login_uses_existing_agy_environment() {
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().join("unused-profile");
        let script = root.path().join("agy");
        std::fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let command = login_command(&script);
        let overrides: Vec<_> = command
            .as_std()
            .get_envs()
            .map(|(key, _)| key.to_owned())
            .collect();
        assert!(!overrides.iter().any(|key| matches!(
            key.to_str(),
            Some(
                "ANTIGRAVITY_APP_DATA_DIR"
                    | "XDG_CONFIG_HOME"
                    | "XDG_CACHE_HOME"
                    | "XDG_DATA_HOME"
                    | "HOME"
            )
        )));
        assert!(!profile.exists());
    }

    #[tokio::test]
    async fn login_command_uses_existing_environment_and_propagates_failure() {
        let root = tempfile::tempdir().unwrap();
        let script = root.path().join("agy");
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" \"$HOME\" > \"$AGY_TEST_CAPTURE\"\nexit 7\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let capture = root.path().join("observed");
        let mut command = login_command(&script);
        command.env("AGY_TEST_CAPTURE", &capture);
        let error = run_login(&mut command).await.unwrap_err();
        assert!(error.to_string().contains("7"));
        let captured = std::fs::read_to_string(capture).unwrap();
        let lines: Vec<_> = captured.lines().collect();
        assert_eq!(&lines[..2], &["acp", "--login"]);
        assert_eq!(lines[2], std::env::var("HOME").unwrap());
    }
}
