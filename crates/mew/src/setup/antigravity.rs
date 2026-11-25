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

pub(crate) const EFFORT_SUFFIXES: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
const AGY_EFFORT_PARAM: &str = "agy_effort";

pub(crate) struct ModelCache {
    state: std::sync::Mutex<ModelCacheState>,
    generation: tokio::sync::watch::Sender<u64>,
    isolated: bool,
}

#[derive(Default)]
struct ModelCacheState {
    models: Vec<mew_protocol::ModelInfo>,
    refreshing: bool,
}

impl ModelCache {
    pub(crate) fn new(isolated: bool) -> Self {
        Self {
            state: Default::default(),
            generation: tokio::sync::watch::channel(0).0,
            isolated,
        }
    }
    pub(crate) fn refresh(
        self: &std::sync::Arc<Self>,
    ) -> Option<tokio::sync::watch::Receiver<u64>> {
        let executable = super::providers::find_agy_executable()?;
        let runtime = tokio::runtime::Handle::try_current().ok()?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let receiver = self.generation.subscribe();
        if state.refreshing {
            return Some(receiver);
        }
        state.refreshing = true;
        drop(state);
        let cache = self.clone();
        runtime.spawn(async move {
            let result = discover_models(executable, cache.isolated).await;
            let mut state = cache.state.lock().unwrap_or_else(|e| e.into_inner());
            match result {
                Ok(models) => state.models = models,
                Err(_) => {
                    state.models.clear();
                    tracing::warn!(
                        "Antigravity model discovery failed; check its sign-in and sandbox setup"
                    );
                }
            }
            state.refreshing = false;
            cache
                .generation
                .send_modify(|generation| *generation = generation.wrapping_add(1));
        });
        Some(receiver)
    }

    pub(crate) fn models(&self) -> Vec<mew_protocol::ModelInfo> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .models
            .clone()
    }

    pub(crate) fn thinking_variants(&self, model_id: &str) -> Vec<String> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .models
            .iter()
            .find(|model| model.model == model_id)
            .map(|model| {
                model
                    .thinking_variants
                    .iter()
                    .map(|variant| variant.name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .models
            .is_empty()
    }
}

/// Discovery has a 25 second deadline followed by bounded subprocess cleanup.
pub(crate) async fn wait_for_model_refresh(mut pending: tokio::sync::watch::Receiver<u64>) {
    let _ = tokio::time::timeout(Duration::from_secs(35), pending.changed()).await;
}

pub(super) struct LocalAgyProvider {
    executable: PathBuf,
    model: String,
    default_effort: Option<String>,
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
            default_effort: None,
            isolated,
            ready: OnceCell::new(),
        }
    }

    pub(super) fn with_default_effort(mut self, effort: Option<String>) -> Self {
        self.default_effort = effort;
        self
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
    let executable = super::providers::find_agy_executable().ok_or_else(|| {
        anyhow::anyhow!(
            "agy executable not found on PATH, in ~/.local/bin, or standard macOS bin directories"
        )
    })?;
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

fn split_effort_model<'a>(id: &'a str, name: &'a str) -> Option<(&'a str, &'static str, &'a str)> {
    let (base_id, effort) = EFFORT_SUFFIXES.iter().find_map(|&effort| {
        let base_id = id.strip_suffix(effort)?.strip_suffix('-')?;
        (!base_id.is_empty()).then_some((base_id, effort))
    })?;
    let base_name = name
        .strip_suffix(')')
        .and_then(|without_paren| without_paren.rsplit_once(" ("))
        .filter(|(prefix, suffix)| !prefix.is_empty() && suffix.eq_ignore_ascii_case(effort))
        .map_or(name, |(prefix, _)| prefix);
    Some((base_id, effort, base_name))
}

fn effort_rank(effort: &str) -> usize {
    EFFORT_SUFFIXES
        .iter()
        .position(|&candidate| candidate == effort)
        .unwrap_or(usize::MAX)
}

fn parse_models(output: &[u8]) -> io::Result<Vec<mew_protocol::ModelInfo>> {
    let output =
        std::str::from_utf8(output).map_err(|_| io::Error::other("invalid AGY model list"))?;
    let mut seen = std::collections::HashSet::new();
    let entries: Vec<(&str, &str)> = output
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
            Some((id, name))
        })
        .collect();

    let mut groups: std::collections::HashMap<&str, (Vec<&'static str>, &str)> =
        std::collections::HashMap::new();
    for &(id, name) in &entries {
        if let Some((base_id, effort, base_name)) = split_effort_model(id, name) {
            let entry = groups
                .entry(base_id)
                .or_insert_with(|| (Vec::new(), base_name));
            if !entry.0.contains(&effort) {
                entry.0.push(effort);
            }
        }
    }

    let mut emitted_bases = std::collections::HashSet::new();
    let mut models = Vec::with_capacity(entries.len());
    for (id, name) in entries {
        if let Some((base_id, _, _)) = split_effort_model(id, name) {
            if let Some((efforts, base_name)) = groups.get(base_id) {
                if efforts.len() >= 2 {
                    if !emitted_bases.insert(base_id) {
                        continue;
                    }
                    let mut sorted = efforts.clone();
                    sorted.sort_by_key(|effort| effort_rank(effort));
                    models.push(mew_protocol::ModelInfo {
                        id: format!("antigravity/{base_id}"),
                        provider: "antigravity".into(),
                        model: base_id.into(),
                        display_name: Some((*base_name).into()),
                        description: None,
                        thinking_variants: sorted
                            .into_iter()
                            .map(|effort| mew_protocol::ThinkingVariantInfo {
                                name: effort.into(),
                            })
                            .collect(),
                        thinking_budget: None,
                        context_window: None,
                    });
                    continue;
                }
            }
        }
        models.push(mew_protocol::ModelInfo {
            id: format!("antigravity/{id}"),
            provider: "antigravity".into(),
            model: id.into(),
            display_name: Some(name.into()),
            description: None,
            thinking_variants: Vec::new(),
            thinking_budget: None,
            context_window: None,
        });
    }
    Ok(models)
}

pub(crate) fn resolve_agy_reasoning(
    variants: &[String],
    variant_name: Option<&str>,
) -> Option<(mew_provider::ReasoningConfig, String)> {
    if variants.is_empty() {
        return None;
    }
    let resolved = match variant_name {
        Some(name) if name == "off" || name == "none" => return None,
        Some(name) => variants
            .iter()
            .find(|candidate| candidate.eq_ignore_ascii_case(name))?
            .clone(),
        None => variants
            .iter()
            .find(|candidate| *candidate == "high")
            .or_else(|| variants.iter().find(|candidate| *candidate == "medium"))
            .or_else(|| variants.first())?
            .clone(),
    };
    let mut params = serde_json::Map::new();
    params.insert(
        AGY_EFFORT_PARAM.into(),
        serde_json::Value::String(resolved.clone()),
    );
    Some((mew_provider::ReasoningConfig { params }, resolved))
}

fn strip_effort_suffix(model: &str) -> &str {
    EFFORT_SUFFIXES
        .iter()
        .find_map(|effort| model.strip_suffix(effort)?.strip_suffix('-'))
        .filter(|base| !base.is_empty())
        .unwrap_or(model)
}

fn resolve_model_for_request(
    model: &str,
    reasoning: Option<mew_provider::ReasoningConfig>,
    default_effort: Option<&str>,
) -> String {
    let effort = reasoning
        .as_ref()
        .and_then(|config| config.params.get(AGY_EFFORT_PARAM))
        .and_then(|value| value.as_str())
        .or(default_effort);
    match effort {
        Some(effort) => {
            let base = strip_effort_suffix(model);
            format!("{base}-{effort}")
        }
        None => model.to_owned(),
    }
}

#[async_trait]
impl Provider for LocalAgyProvider {
    fn name(&self) -> &str {
        "antigravity"
    }

    async fn replay_budget(
        &self,
        request: &Request,
    ) -> Result<Option<(usize, usize)>, ProviderError> {
        self.ready().await?.provider.replay_budget(request).await
    }
    async fn can_accept_guidance(&self, request: &Request) -> Result<bool, ProviderError> {
        self.ready()
            .await?
            .provider
            .can_accept_guidance(request)
            .await
    }

    async fn abandon_pending(&self, session_id: mew_message::SessionId) {
        if let Some(ready) = self.ready.get() {
            ready.provider.abandon_pending(session_id).await;
        }
    }

    async fn stream(&self, mut request: Request) -> Result<EventStream, ProviderError> {
        if request.model.is_empty() {
            request.model.clone_from(&self.model);
        }
        let reasoning = request.reasoning.take();
        request.model =
            resolve_model_for_request(&request.model, reasoning, self.default_effort.as_deref());
        self.ready().await?.provider.stream(request).await
    }

    async fn stream_auxiliary(&self, mut request: Request) -> Result<EventStream, ProviderError> {
        if request.model.is_empty() {
            request.model.clone_from(&self.model);
        }
        let reasoning = request.reasoning.take();
        request.model =
            resolve_model_for_request(&request.model, reasoning, self.default_effort.as_deref());
        self.ready().await?.provider.stream_auxiliary(request).await
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

    #[tokio::test(start_paused = true)]
    async fn model_refresh_waits_for_discovery_cleanup_after_request_timeout() {
        let (sender, receiver) = tokio::sync::watch::channel(0u64);
        let waiting = tokio::spawn(wait_for_model_refresh(receiver));
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(27)).await;
        assert!(
            !waiting.is_finished(),
            "refresh expired during process cleanup"
        );
        sender.send(1).unwrap();
        waiting.await.unwrap();
    }

    #[tokio::test]
    async fn agy_provider_can_answer_permission_classifier_requests() {
        struct Fixture(PathBuf);
        impl IsolationBoundary for Fixture {
            fn wrap(&self, mut spec: LaunchSpec, _: &Path) -> io::Result<LaunchSpec> {
                spec.program = "/usr/bin/python3".into();
                spec.args = vec!["-u".into(), self.0.as_os_str().into()];
                Ok(spec)
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("classifier.py");
        std::fs::write(&script, r#"import sys,json
for line in sys.stdin:
 msg=json.loads(line)
 mid=msg.get('id')
 method=msg.get('method')
 if method=='initialize':
  print(json.dumps({'jsonrpc':'2.0','id':mid,'result':{'protocolVersion':1}}),flush=True)
 elif method=='authenticate':
  print(json.dumps({'jsonrpc':'2.0','id':mid,'result':{}}),flush=True)
 elif method=='session/new':
  print(json.dumps({'jsonrpc':'2.0','id':mid,'result':{'sessionId':'s1'}}),flush=True)
 elif method=='session/set_model':
  print(json.dumps({'jsonrpc':'2.0','id':mid,'result':{}}),flush=True)
 elif method=='session/prompt':
  print(json.dumps({'jsonrpc':'2.0','method':'session/update','params':{'sessionId':'s1','update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'allow'}}}}),flush=True)
  print(json.dumps({'jsonrpc':'2.0','id':mid,'result':{'stopReason':'end_turn'}}),flush=True)
  break
"#).unwrap();
        let provider = std::sync::Arc::new(AgyProvider::new(
            Config {
                executable: "/usr/bin/python3".into(),
                workspace_parent: directory.path().to_owned(),
                environment: vec![],
                lifetime: Duration::from_secs(10),
            },
            Fixture(script),
        ));
        let mut agent = mew_agent::Agent::new(
            std::sync::Arc::new(mew_provider_fake::FakeProvider::new(vec![])),
            std::sync::Arc::new(mew_hooks::NopDispatcher),
            None,
            vec![std::sync::Arc::new(mew_tools::tools::write::Write)],
            None,
        );
        agent.set_classifier_provider(provider, Some("model".into()));
        let decision = agent
            .classify_permission(&mew_hooks::ToolCall {
                tool_name: "write".into(),
                call_id: "test".into(),
                input: serde_json::json!({"path":"report.txt","content":"hi"}),
            })
            .await;
        assert_eq!(
            decision,
            Some(mew_prompts::classifier::ClassifierDecision::Allow)
        );
    }

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
    fn groups_effort_variants_into_single_model_with_thinking_variants() {
        let input = b"\
gemini-4-neon-high\tGemini 4 Neon (High)\n\
gemini-4-neon-xhigh\tGemini 4 Neon (XHigh)\n\
gemini-4-neon-medium\tGemini 4 Neon (Medium)\n\
gemini-4-neon-low\tGemini 4 Neon (Low)\n\
gemini-pro-orchestrator\tOrchestrator (Carbon)\n\
gpt-oss-120b-medium\tGPT-OSS 120B (Medium)\n";
        let models = parse_models(input).unwrap();
        assert_eq!(models.len(), 3);

        assert_eq!(models[0].id, "antigravity/gemini-4-neon");
        assert_eq!(models[0].model, "gemini-4-neon");
        assert_eq!(models[0].display_name.as_deref(), Some("Gemini 4 Neon"));
        let variants: Vec<&str> = models[0]
            .thinking_variants
            .iter()
            .map(|v| v.name.as_str())
            .collect();
        assert_eq!(variants, vec!["low", "medium", "high", "xhigh"]);

        assert_eq!(models[1].id, "antigravity/gemini-pro-orchestrator");
        assert!(models[1].thinking_variants.is_empty());

        assert_eq!(models[2].id, "antigravity/gpt-oss-120b-medium");
        assert!(models[2].thinking_variants.is_empty());
    }

    #[test]
    fn resolves_agy_reasoning_and_request_model() {
        let variants = vec!["low".into(), "medium".into(), "high".into(), "xhigh".into()];
        let (default_cfg, default_name) = resolve_agy_reasoning(&variants, None).unwrap();
        assert_eq!(default_name, "high");
        assert_eq!(
            resolve_model_for_request("gemini-4-neon", Some(default_cfg), Some("low")),
            "gemini-4-neon-high"
        );

        let (explicit_cfg, explicit_name) =
            resolve_agy_reasoning(&variants, Some("XHigh")).unwrap();
        assert_eq!(explicit_name, "xhigh");
        assert_eq!(
            resolve_model_for_request("gemini-4-neon", Some(explicit_cfg), Some("low")),
            "gemini-4-neon-xhigh"
        );

        assert!(resolve_agy_reasoning(&variants, Some("off")).is_none());
        assert!(resolve_agy_reasoning(&variants, Some("unknown")).is_none());
        assert_eq!(
            resolve_model_for_request("gemini-4-neon", None, Some("low")),
            "gemini-4-neon-low"
        );
        assert_eq!(
            resolve_model_for_request("gemini-3.8-flash-low", None, Some("low")),
            "gemini-3.8-flash-low"
        );
        assert_eq!(
            resolve_model_for_request("gemini-4-neon-high", None, Some("low")),
            "gemini-4-neon-low"
        );
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
