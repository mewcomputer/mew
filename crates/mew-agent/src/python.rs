use std::collections::{BTreeSet, VecDeque};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::Duration;

use async_trait::async_trait;
use mew_tools::{Sensitivity, Tool, ToolCtx, ToolError};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const WORKER: &str = include_str!("python_worker.py");
const MAX_FRAME_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const PYTHON_IDLE_TIMEOUT: Duration = Duration::from_secs(60 * 60);

fn redact_nested_input(value: &Value, secrets: &mew_tools::SecretSet) -> Value {
    if secrets.words.is_empty() {
        return value.clone();
    }
    match value {
        Value::String(value) => {
            Value::String(mew_tools::secrets::redact_secret_words(value, secrets).0)
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| redact_nested_input(value, secrets))
                .collect(),
        ),
        Value::Object(values) => Value::Object(
            values
                .iter()
                .map(|(key, value)| {
                    (
                        mew_tools::secrets::redact_secret_words(key, secrets).0,
                        redact_nested_input(value, secrets),
                    )
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

pub(crate) struct PythonTool {
    pub isolated: bool,
}

#[async_trait]
impl Tool for PythonTool {
    fn name(&self) -> &str {
        "python"
    }
    fn description(&self) -> &str {
        "Run Python code in this session."
    }
    fn schema(&self) -> &Value {
        static SCHEMA: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
        SCHEMA.get_or_init(|| serde_json::json!({"type":"object","properties":{"source":{"type":"string"}},"required":["source"]}))
    }
    fn sensitivity(&self) -> Sensitivity {
        if self.isolated {
            Sensitivity::ReadOnly
        } else {
            Sensitivity::Dangerous
        }
    }
    async fn execute(
        &self,
        _ctx: ToolCtx,
        _input: Value,
    ) -> Result<mew_hooks::ToolOutput, ToolError> {
        Err(ToolError::Execution(
            "Python execution requires the agent".into(),
        ))
    }
}

#[derive(Clone)]
pub(crate) struct PythonOptions {
    pub executable: PathBuf,
    pub isolated: bool,
    pub cwd: PathBuf,
}

pub(crate) struct PythonWorker {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    isolated: bool,
    #[cfg(unix)]
    process_group: bool,
    diagnostics: Arc<std::sync::Mutex<Vec<u8>>>,
    stderr_task: Option<tokio::task::JoinHandle<()>>,
    _scratch: tempfile::TempDir,
}

type SharedPythonState = tokio::sync::Mutex<PythonSessionState>;

/// Retains a bounded number of idle Python workers across sessions.
#[derive(Clone)]
pub struct PythonWorkerCache {
    idle: Arc<StdMutex<IdlePythonWorkers>>,
}

struct IdlePythonWorkers {
    max: usize,
    sessions: VecDeque<Weak<SharedPythonState>>,
}

impl PythonWorkerCache {
    pub fn new(max_idle_workers: usize) -> Self {
        Self {
            idle: Arc::new(StdMutex::new(IdlePythonWorkers {
                max: max_idle_workers,
                sessions: VecDeque::new(),
            })),
        }
    }

    pub fn session(&self) -> PythonSession {
        PythonSession::new(self.clone(), PYTHON_IDLE_TIMEOUT)
    }
}

impl IdlePythonWorkers {
    fn remove(&mut self, state: &Arc<SharedPythonState>) {
        self.sessions
            .retain(|entry| entry.strong_count() > 0 && entry.as_ptr() != Arc::as_ptr(state));
    }

    fn enforce(&mut self) -> Vec<PythonWorker> {
        if self.sessions.len() <= self.max {
            return Vec::new();
        }
        let states: Vec<_> = self.sessions.iter().filter_map(Weak::upgrade).collect();
        // Active sessions hold their own lock, so only unlocked idle workers
        // participate in the limit. Locking is always nonblocking here.
        let mut idle: Vec<_> = states
            .iter()
            .enumerate()
            .filter_map(|(index, state)| {
                state
                    .try_lock()
                    .ok()
                    .filter(|state| state.worker.is_some())
                    .map(|guard| (index, guard))
            })
            .collect();
        let excess = idle.len().saturating_sub(self.max);
        let mut evicted = Vec::with_capacity(excess);
        let mut workers = Vec::with_capacity(excess);
        for (index, state) in idle.iter_mut().take(excess) {
            if let Some(reaper) = state.reaper.take() {
                reaper.abort();
            }
            if let Some(worker) = state.worker.take() {
                workers.push(worker);
            }
            state.expired = Some(PythonExpiration::Capacity);
            evicted.push(Arc::as_ptr(&states[*index]));
        }
        drop(idle);
        self.sessions
            .retain(|entry| entry.strong_count() > 0 && !evicted.contains(&entry.as_ptr()));
        workers
    }
}

/// A live Python environment shared by the runtime instances of one session.
#[derive(Clone)]
pub struct PythonSession {
    state: Arc<SharedPythonState>,
    cache: PythonWorkerCache,
    idle_timeout: Duration,
    policy_revision: Arc<std::sync::atomic::AtomicU64>,
}

enum PythonExpiration {
    Idle,
    Capacity,
    Isolation,
}

struct PythonSessionState {
    worker: Option<PythonWorker>,
    last_used: tokio::time::Instant,
    expired: Option<PythonExpiration>,
    reaper: Option<tokio::task::AbortHandle>,
}

impl Drop for PythonSessionState {
    fn drop(&mut self) {
        if let Some(reaper) = self.reaper.take() {
            reaper.abort();
        }
    }
}

impl Default for PythonSession {
    fn default() -> Self {
        PythonWorkerCache::new(mew_config::CodeModeConfig::default().max_idle_workers).session()
    }
}

impl PythonSession {
    fn new(cache: PythonWorkerCache, idle_timeout: Duration) -> Self {
        Self {
            state: Arc::new(tokio::sync::Mutex::new(PythonSessionState {
                worker: None,
                last_used: tokio::time::Instant::now(),
                expired: None,
                reaper: None,
            })),
            cache,
            idle_timeout,
            policy_revision: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_idle_timeout(idle_timeout: Duration) -> Self {
        Self::new(PythonWorkerCache::new(3), idle_timeout)
    }

    #[cfg(test)]
    pub(crate) async fn idle_reaper(&self) -> Option<tokio::task::AbortHandle> {
        self.state.lock().await.reaper.clone()
    }

    pub(crate) async fn lock(&self) -> PythonSessionGuard<'_> {
        let state = self.state.lock().await;
        self.cache.idle.lock().unwrap().remove(&self.state);
        PythonSessionGuard {
            state: Some(state),
            session: self,
        }
    }

    pub(crate) fn update_policy(&self, available: bool, isolated: bool) {
        use std::sync::atomic::Ordering;

        let revision = self.policy_revision.fetch_add(1, Ordering::AcqRel) + 1;
        if let Ok(mut state) = self.state.try_lock() {
            if self.policy_revision.load(Ordering::Acquire) == revision {
                self.reset_for_policy(&mut state, available, isolated);
            }
            return;
        }
        let session = self.clone();
        let reset = move || {
            let mut state = session.state.blocking_lock();
            if session.policy_revision.load(Ordering::Acquire) == revision {
                session.reset_for_policy(&mut state, available, isolated);
            }
        };
        std::thread::spawn(reset);
    }

    fn reset_for_policy(&self, state: &mut PythonSessionState, available: bool, isolated: bool) {
        if state
            .worker
            .as_ref()
            .is_some_and(|worker| !available || worker.isolated != isolated)
        {
            let worker = state.worker.take();
            state.expired = Some(PythonExpiration::Isolation);
            if let Some(reaper) = state.reaper.take() {
                reaper.abort();
            }
            self.cache.idle.lock().unwrap().remove(&self.state);
            drop(worker);
        }
    }
}

pub(crate) struct PythonSessionGuard<'a> {
    state: Option<tokio::sync::MutexGuard<'a, PythonSessionState>>,
    session: &'a PythonSession,
}

impl std::ops::Deref for PythonSessionGuard<'_> {
    type Target = Option<PythonWorker>;

    fn deref(&self) -> &Self::Target {
        &self.state.as_ref().expect("session guard").worker
    }
}

impl std::ops::DerefMut for PythonSessionGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.state.as_mut().expect("session guard").worker
    }
}

impl Drop for PythonSessionGuard<'_> {
    fn drop(&mut self) {
        let mut state = self.state.take().expect("session guard");
        state.last_used = tokio::time::Instant::now();
        let mut cache = self.session.cache.idle.lock().unwrap();
        cache.remove(&self.session.state);
        if state.worker.is_some() {
            cache
                .sessions
                .push_back(Arc::downgrade(&self.session.state));
        }
        if state.worker.is_none() {
            if let Some(reaper) = state.reaper.take() {
                reaper.abort();
            }
        } else if state.reaper.is_none() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                let deadline = state.last_used + self.session.idle_timeout;
                let timeout = self.session.idle_timeout;
                let weak_state = Arc::downgrade(&self.session.state);
                let cache = Arc::downgrade(&self.session.cache.idle);
                let reaper = runtime.spawn(async move {
                    let mut deadline = deadline;
                    loop {
                        tokio::time::sleep_until(deadline).await;
                        let Some(state_arc) = weak_state.upgrade() else {
                            return;
                        };
                        let mut state = state_arc.lock().await;
                        if state.worker.is_none() {
                            state.reaper = None;
                            return;
                        }
                        deadline = state.last_used + timeout;
                        if tokio::time::Instant::now() >= deadline {
                            let worker = state.worker.take();
                            state.expired = Some(PythonExpiration::Idle);
                            state.reaper = None;
                            if let Some(cache) = cache.upgrade() {
                                cache.lock().unwrap().remove(&state_arc);
                            }
                            drop(state);
                            drop(worker);
                            return;
                        }
                        if let Some(cache) = cache.upgrade() {
                            let mut cache = cache.lock().unwrap();
                            drop(state);
                            let workers = cache.enforce();
                            drop(cache);
                            drop(workers);
                        }
                    }
                });
                state.reaper = Some(reaper.abort_handle());
            }
        }
        drop(state);
        let workers = cache.enforce();
        drop(cache);
        drop(workers);
    }
}

impl PythonWorker {
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn scratch_path(&self) -> &Path {
        self._scratch.path()
    }

    pub(crate) async fn start(options: &PythonOptions) -> io::Result<Self> {
        let scratch = tempfile::tempdir()?;
        let mut command = if options.isolated {
            isolated_command(&options.executable, scratch.path()).await?
        } else {
            let mut command = Command::new(&options.executable);
            command.current_dir(&options.cwd);
            command
        };
        #[cfg(unix)]
        if !options.isolated {
            command.process_group(0);
        }
        command
            .args(["-I", "-u", "-c", WORKER])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        let input = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("Python stdin unavailable"))?;
        let output = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("Python stdout unavailable"))?;
        let mut stderr = child
            .stderr
            .take()
            .ok_or_else(|| io::Error::other("Python stderr unavailable"))?;
        let diagnostics = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = diagnostics.clone();
        let stderr_task = tokio::spawn(async move {
            let mut buffer = [0; 1024];
            while let Ok(count) = stderr.read(&mut buffer).await {
                if count == 0 {
                    break;
                }
                let mut log = captured.lock().unwrap();
                let remaining = 8192usize.saturating_sub(log.len());
                log.extend_from_slice(&buffer[..count.min(remaining)]);
            }
        });
        let mut worker = Self {
            child,
            input,
            output: BufReader::new(output),
            isolated: options.isolated,
            #[cfg(unix)]
            process_group: !options.isolated,
            diagnostics,
            stderr_task: Some(stderr_task),
            _scratch: scratch,
        };
        let ready = tokio::time::timeout(std::time::Duration::from_secs(10), worker.receive())
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Python startup timed out"))??;
        if ready["type"] != "ready" {
            return Err(io::Error::other("invalid Python startup message"));
        }
        Ok(worker)
    }

    pub(crate) async fn send(&mut self, value: &Value) -> io::Result<()> {
        let mut data = serde_json::to_vec(value)?;
        data.push(b'\n');
        self.input.write_all(&data).await
    }

    pub(crate) async fn receive(&mut self) -> io::Result<Value> {
        let mut data = Vec::new();
        let count = (&mut self.output)
            .take(MAX_FRAME_BYTES + 1)
            .read_until(b'\n', &mut data)
            .await?;
        if count == 0 {
            if let Some(task) = self.stderr_task.take() {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(1), task).await;
            }
            let diagnostic = self.diagnostics.lock().unwrap();
            let detail = String::from_utf8_lossy(&diagnostic);
            let message = if detail.trim().is_empty() {
                "Python worker exited".to_string()
            } else {
                format!("Python worker exited: {}", detail.trim())
            };
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, message));
        }
        if count as u64 > MAX_FRAME_BYTES || data.last() != Some(&b'\n') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Python worker message too large or incomplete",
            ));
        }
        serde_json::from_slice(&data).map_err(io::Error::other)
    }
}

impl Drop for PythonWorker {
    fn drop(&mut self) {
        #[cfg(unix)]
        if self.process_group {
            if let Some(pid) = self.child.id().and_then(|id| i32::try_from(id).ok()) {
                let _ = nix::sys::signal::killpg(
                    nix::unistd::Pid::from_raw(pid),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
        }
        let _ = self.child.start_kill();
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn executable_on_path(executable: &Path) -> io::Result<PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    if executable.components().count() != 1 {
        return Ok(executable.to_path_buf());
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(executable);
        if std::fs::metadata(&candidate)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        {
            return if candidate.is_absolute() {
                Ok(candidate)
            } else {
                Ok(std::env::current_dir()?.join(candidate))
            };
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("executable not found: {}", executable.display()),
    ))
}

async fn isolated_command(python: &Path, scratch: &Path) -> io::Result<Command> {
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (python, scratch);
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Python isolation is not available on this platform",
        ));
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let python = executable_on_path(python)?;
        const DISCOVER: &str = "import json,os,site,sys,sysconfig; libdir=sysconfig.get_config_var('LIBDIR') or ''; lib=sysconfig.get_config_var('LDLIBRARY') or ''; root=os.path.dirname(os.path.dirname(sys.executable)); paths=sys.path + site.getsitepackages() + [os.path.join(libdir,lib),os.path.join(sys.base_prefix,'Python'),os.path.join(root,'pyvenv.cfg'),os.path.join(root,'lib','python%d.%d'%sys.version_info[:2],'site-packages')]; print(json.dumps({'executable':sys.executable,'base':sys.base_prefix,'paths':paths}))";
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            Command::new(&python)
                .args(["-I", "-S", "-c", DISCOVER])
                .kill_on_drop(true)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output(),
        )
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Python discovery timed out"))??;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "Python discovery failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let info: Value = serde_json::from_slice(&output.stdout).map_err(io::Error::other)?;
        let executable = info["executable"]
            .as_str()
            .ok_or_else(|| io::Error::other("Python executable missing"))?;
        let selected_executable = PathBuf::from(executable);
        let mut paths = BTreeSet::new();
        for path in info["paths"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if let Ok(path) = std::fs::canonicalize(path) {
                if path.is_absolute() {
                    paths.insert(path);
                }
            }
        }
        let executable = std::fs::canonicalize(executable)?;
        #[cfg(target_os = "macos")]
        {
            let base = info["base"]
                .as_str()
                .ok_or_else(|| io::Error::other("Python base missing"))?;
            let framework =
                std::fs::canonicalize(base)?.join("Resources/Python.app/Contents/MacOS/Python");
            let actual = if framework.is_file() {
                framework
            } else {
                executable.clone()
            };
            let mut allowed = paths;
            for path in ["/System", "/usr/lib", "/usr/share/zoneinfo"] {
                if Path::new(path).exists() {
                    allowed.insert(PathBuf::from(path));
                }
            }
            allowed.insert(actual.clone());
            allowed.insert(std::fs::canonicalize(scratch)?);
            let mut metadata = BTreeSet::new();
            for path in allowed.iter().chain(std::iter::once(&selected_executable)) {
                for parent in path.ancestors().skip(1) {
                    metadata.insert(parent.to_path_buf());
                }
            }
            metadata.insert(selected_executable.clone());
            let quote =
                |path: &Path| serde_json::to_string(&path.to_string_lossy()).expect("path JSON");
            let reads = allowed
                .iter()
                .map(|p| format!("(subpath {})", quote(p)))
                .collect::<Vec<_>>()
                .join(" ");
            let ancestors = metadata
                .iter()
                .map(|p| format!("(literal {})", quote(p)))
                .collect::<Vec<_>>()
                .join(" ");
            let profile = format!("(version 1)(deny default)(import \"system.sb\")(allow process-exec (literal {}))(allow file-read* {} (literal \"/dev/null\"))(allow file-read-metadata {})(allow file-write* (subpath {}) (literal \"/dev/null\"))(allow sysctl-read)",quote(&actual), reads, ancestors, quote(&std::fs::canonicalize(scratch)?));
            let mut command = Command::new("/usr/bin/sandbox-exec");
            command
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", scratch)
                .env("TMPDIR", scratch);
            command.args(["-p", &profile]).arg(&actual);
            // Framework builds launch a helper binary; point it back to the selected venv.
            if actual != executable {
                command.env("__PYVENV_LAUNCHER__", selected_executable);
            }
            Ok(command)
        }
        #[cfg(target_os = "linux")]
        {
            let mut allowed = paths;
            allowed.insert(executable.clone());
            allowed.insert(selected_executable.clone());
            for path in [
                "/lib",
                "/lib64",
                "/usr/lib",
                "/usr/lib64",
                "/usr/share/zoneinfo",
            ] {
                if Path::new(path).exists() {
                    allowed.insert(PathBuf::from(path));
                }
            }
            let mut parents = BTreeSet::new();
            for path in &allowed {
                for parent in path.ancestors().skip(1) {
                    if parent != Path::new("/") {
                        parents.insert(parent.to_path_buf());
                    }
                }
            }
            let mut command = Command::new(executable_on_path(Path::new("bwrap"))?);
            command.env_clear().env("PATH", "/usr/bin:/bin");
            command.args([
                "--unshare-all",
                "--unshare-user",
                "--disable-userns",
                "--new-session",
                "--die-with-parent",
                "--clearenv",
            ]);
            for parent in parents {
                command.arg("--dir").arg(parent);
            }
            for path in allowed {
                command.arg("--ro-bind").arg(&path).arg(&path);
            }
            command
                .args(["--proc", "/proc", "--dev", "/dev", "--bind"])
                .arg(scratch)
                .arg("/mew-work");
            command.args([
                "--chdir",
                "/mew-work",
                "--setenv",
                "HOME",
                "/mew-work",
                "--setenv",
                "TMPDIR",
                "/mew-work",
            ]);
            command.arg(selected_executable);
            Ok(command)
        }
    }
}

impl crate::agent::Agent {
    fn python_error(&self, message: String, ctx: &ToolCtx) -> ToolError {
        let (message, count) = mew_tools::secrets::redact_secret_words(&message, &ctx.secrets);
        ToolError::Execution(mew_tools::secrets::annotate_redaction(message, count))
    }

    fn python_catalog(&self) -> Vec<Value> {
        let mut tools: Vec<_> = self
            .tools
            .values()
            .filter(|tool| self.tool_available(tool.name()))
            .collect();
        tools.sort_by(|a, b| a.name().cmp(b.name()));
        let mut counts = std::collections::HashMap::new();
        for tool in &tools {
            *counts
                .entry(python_identifier(tool.name()))
                .or_insert(0usize) += 1;
        }
        tools.into_iter().map(|tool| {
            let alias = python_identifier(tool.name());
            let name = if counts[&alias] == 1 { alias } else { tool.name().to_owned() };
            serde_json::json!({"name":name,"tool_name":tool.name(),"description":tool.description(),"schema":tool.schema()})
        }).collect()
    }

    pub(crate) fn python_tool_definition(&self) -> mew_provider::ToolDef {
        let tool = PythonTool {
            isolated: self.python_isolated(),
        };
        let catalog = self.python_catalog();
        mew_provider::ToolDef {
            name: tool.name().into(),
            description: format!("Run Python in a persistent session. Use text(value) to send output to the model. Call tools as tools.name({{...}}) or tools.call('exact-name', {{...}}); results have output, error, and images fields. Use emit_image(result['images'][0]) to view an image. The available tools and their schemas are: {}", serde_json::Value::Array(catalog)),
            schema: tool.schema().clone(),
        }
    }

    #[cfg(test)]
    pub(crate) async fn execute_python(
        &self,
        ctx: &ToolCtx,
        input: Value,
        events: &tokio::sync::mpsc::Sender<crate::AgentEvent>,
    ) -> Result<mew_hooks::ToolOutput, ToolError> {
        let worker = self.python_session.lock().await;
        self.execute_python_with_guard(ctx, input, events, worker, &mut Vec::new())
            .await
    }

    pub(crate) async fn execute_python_with_guard(
        &self,
        ctx: &ToolCtx,
        input: Value,
        events: &tokio::sync::mpsc::Sender<crate::AgentEvent>,
        mut worker: PythonSessionGuard<'_>,
        nested_calls: &mut Vec<mew_message::NestedToolCall>,
    ) -> Result<mew_hooks::ToolOutput, ToolError> {
        if ctx.cancel.is_cancelled() {
            return Err(ToolError::Execution("Python was cancelled".into()));
        }
        let source = input
            .get("source")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("source must be a string".into()))?;
        let catalog = self.python_catalog();
        let isolated = self.python_isolated();
        if worker
            .as_ref()
            .is_some_and(|process| process.isolated != isolated)
        {
            *worker = None;
            worker.state.as_mut().expect("session guard").expired =
                Some(PythonExpiration::Isolation);
        }
        if let Some(reason) = worker.state.as_mut().expect("session guard").expired.take() {
            let reason = match reason {
                PythonExpiration::Idle => "after being idle",
                PythonExpiration::Capacity => "because the idle worker capacity was exceeded",
                PythonExpiration::Isolation => "because Python access or isolation changed",
            };
            return Err(ToolError::Execution(format!(
                "Python stopped {reason}; its session state, including variables and temporary scratch files, was lost. Run the code again in a fresh session."
            )));
        }
        if worker.is_none() {
            let options = PythonOptions {
                executable: self
                    .code_mode
                    .python
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("python3")),
                isolated,
                cwd: self.cwd.clone(),
            };
            let started = tokio::select! {
                _ = ctx.cancel.cancelled() => return Err(ToolError::Execution("Python was cancelled".into())),
                result = PythonWorker::start(&options) => result,
            };
            *worker = Some(
                started
                    .map_err(|e| self.python_error(format!("could not start Python: {e}"), ctx))?,
            );
        }
        let process = worker.as_mut().expect("worker started");
        let cell = ulid::Ulid::new().to_string();
        let request =
            serde_json::json!({"type":"exec","cell":cell,"source":source,"tools":catalog});
        let sent = tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => {
                *worker = None;
                return Err(ToolError::Execution("Python was cancelled; session state was lost".into()));
            }
            result = process.send(&request) => result,
        };
        if let Err(error) = sent {
            *worker = None;
            return Err(self.python_error(
                format!("Python worker exited and session state was lost: {error}"),
                ctx,
            ));
        }
        loop {
            let incoming = tokio::select! {
                _ = ctx.cancel.cancelled() => {
                    *worker = None;
                    return Err(ToolError::Execution("Python was cancelled; session state was lost".into()));
                }
                result = tokio::time::timeout(std::time::Duration::from_secs(600), process.receive()) => result,
            };
            let message = match incoming {
                Ok(Ok(message)) => message,
                Ok(Err(error)) => {
                    *worker = None;
                    return Err(self.python_error(
                        format!("Python worker exited and session state was lost: {error}"),
                        ctx,
                    ));
                }
                Err(_) => {
                    *worker = None;
                    return Err(ToolError::Execution(
                        "Python timed out; session state was lost".into(),
                    ));
                }
            };
            if message["cell"].as_str() != Some(&cell) {
                *worker = None;
                return Err(ToolError::Execution(
                    "invalid or stale Python worker message; session state was lost".into(),
                ));
            }
            match message["type"].as_str() {
                Some("result") => {
                    let images = match serde_json::from_value(message["images"].clone()) {
                        Ok(images) => images,
                        Err(error) => {
                            *worker = None;
                            return Err(self.python_error(
                                format!("invalid Python images; session state was lost: {error}"),
                                ctx,
                            ));
                        }
                    };
                    let (output, count) = mew_tools::secrets::redact_secret_words(
                        message["output"].as_str().unwrap_or(""),
                        &ctx.secrets,
                    );
                    return Ok(mew_hooks::ToolOutput {
                        output: mew_tools::secrets::annotate_redaction(output, count),
                        images,
                        ..Default::default()
                    });
                }
                Some("error") => {
                    if message["reset"] == true {
                        *worker = None;
                    }
                    let error = format!(
                        "{}{}",
                        message["error"].as_str().unwrap_or("Python error"),
                        message["output"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(|s| format!("\n{s}"))
                            .unwrap_or_default()
                    );
                    let (error, count) =
                        mew_tools::secrets::redact_secret_words(&error, &ctx.secrets);
                    return Ok(mew_hooks::ToolOutput {
                        error: mew_tools::secrets::annotate_redaction(error, count),
                        ..Default::default()
                    });
                }
                Some("tool") => {
                    let name = message["name"].as_str().unwrap_or("");
                    let call_id = format!("{}:{}", ctx.call_id, message["id"]);
                    let input = redact_nested_input(&message["input"], &ctx.secrets);
                    nested_calls.push(mew_message::NestedToolCall {
                        parent_call_id: ctx.call_id.clone(),
                        call_id: call_id.clone(),
                        tool_name: name.to_owned(),
                        input: input.clone(),
                        status: mew_message::NestedToolStatus::Running,
                        error: None,
                        output_bytes: None,
                    });
                    let _ = events
                        .send(crate::AgentEvent::NestedToolStart {
                            parent_call_id: ctx.call_id.clone(),
                            call_id: call_id.clone(),
                            tool_name: name.to_owned(),
                            input,
                        })
                        .await;
                    if !self.tool_available(name) {
                        let call = nested_calls.last_mut().expect("nested call added");
                        call.status = mew_message::NestedToolStatus::Error;
                        call.error = Some("tool unavailable".into());
                        let _ = events
                            .send(crate::AgentEvent::NestedToolEnd {
                                parent_call_id: ctx.call_id.clone(),
                                call_id,
                                success: false,
                                error: Some("tool unavailable".into()),
                                output_bytes: None,
                            })
                            .await;
                        let response = serde_json::json!({"type":"tool_result","cell":cell,"id":message["id"],"result":{"output":"","error":"tool unavailable"}});
                        if let Err(error) = process.send(&response).await {
                            *worker = None;
                            return Err(self.python_error(
                                format!("Python worker exited and session state was lost: {error}"),
                                ctx,
                            ));
                        }
                        continue;
                    }
                    let part_id = ulid::Ulid::new();
                    let message_id = ulid::Ulid::new();
                    let tc = mew_message::ToolCallPart {
                        base: mew_message::PartBase {
                            id: part_id,
                            message_id,
                            session_id: self.session_id,
                        },
                        tool_name: name.to_owned(),
                        call_id: call_id.clone(),
                        state: mew_message::ToolState::Pending(mew_message::ToolStatePending {
                            input: message["input"].clone(),
                            time: mew_message::ToolTime {
                                start: chrono::Utc::now().timestamp_millis(),
                                end: None,
                            },
                        }),
                        raw_input: String::new(),
                    };
                    let mut nested = Some(mew_message::Message {
                        id: message_id,
                        session_id: self.session_id,
                        role: mew_message::Role::Assistant,
                        parts: vec![mew_message::Part::ToolCall(tc.clone())],
                        time: mew_message::Time {
                            created: chrono::Utc::now().timestamp_millis(),
                            completed: None,
                        },
                        assistant: None,
                    });
                    let (nested_tx, mut nested_rx) = tokio::sync::mpsc::channel(32);
                    let forwarded_events = events.clone();
                    let parent = ctx.call_id.clone();
                    let forwarder = tokio::spawn(async move {
                        while let Some(event) = nested_rx.recv().await {
                            match event {
                                crate::AgentEvent::ToolStart { .. }
                                | crate::AgentEvent::ToolEnd { .. } => {}
                                crate::AgentEvent::PartUpdated { ref part, .. } if !matches!(part, mew_message::Part::ToolCall(call) if call.tool_name == "exit_tool" || (call.tool_name == "progress_update" && matches!(call.state, mew_message::ToolState::Completed(_)))) =>
                                    {}
                                crate::AgentEvent::ToolProgress { chunk, .. } => {
                                    let _ = forwarded_events
                                        .send(crate::AgentEvent::ToolProgress {
                                            call_id: parent.clone(),
                                            chunk,
                                        })
                                        .await;
                                }
                                other => {
                                    let _ = forwarded_events.send(other).await;
                                }
                            }
                        }
                    });
                    Box::pin(self.execute_tool_calls(&[tc], &mut nested, &nested_tx, true)).await;
                    drop(nested_tx);
                    let _ = forwarder.await;
                    let state = nested
                        .as_ref()
                        .and_then(|message| message.parts.first())
                        .and_then(|part| match part {
                            mew_message::Part::ToolCall(call) => Some(&call.state),
                            _ => None,
                        });
                    let call = nested_calls.last_mut().expect("nested call added");
                    call.status = if matches!(state, Some(mew_message::ToolState::Completed(_))) {
                        mew_message::NestedToolStatus::Completed
                    } else {
                        mew_message::NestedToolStatus::Error
                    };
                    call.error = match state {
                        Some(mew_message::ToolState::Error(error)) => {
                            let (message, count) =
                                mew_tools::secrets::redact_secret_words(&error.error, &ctx.secrets);
                            Some(mew_tools::secrets::annotate_redaction(message, count))
                        }
                        Some(mew_message::ToolState::Completed(_)) => None,
                        _ => Some("tool did not finish".into()),
                    };
                    call.output_bytes = match state {
                        Some(mew_message::ToolState::Completed(result)) => {
                            Some(result.output.len())
                        }
                        _ => None,
                    };
                    let _ = events
                        .send(crate::AgentEvent::NestedToolEnd {
                            parent_call_id: ctx.call_id.clone(),
                            call_id: call_id.clone(),
                            success: matches!(state, Some(mew_message::ToolState::Completed(_))),
                            output_bytes: call.output_bytes,
                            error: call.error.clone(),
                        })
                        .await;
                    if let Some(mew_message::ToolState::Completed(state)) = state {
                        if name == "exit_tool" {
                            *self.python_exit.lock().await = true;
                            let answer = state.output.clone();
                            *worker = None;
                            return Ok(mew_hooks::ToolOutput {
                                output: answer.clone(),
                                metadata: Some(serde_json::json!({"python_exit":answer})),
                                ..Default::default()
                            });
                        }
                    }
                    let result = match state {
                        Some(mew_message::ToolState::Completed(state)) => {
                            serde_json::json!({"output":state.output,"error":"","metadata":state.metadata,"diff":state.diff,"images":state.images})
                        }
                        Some(mew_message::ToolState::Error(state)) => {
                            serde_json::json!({"output":"","error":state.error})
                        }
                        _ => serde_json::json!({"output":"","error":"tool did not finish"}),
                    };
                    let response = serde_json::json!({"type":"tool_result","cell":cell,"id":message["id"],"result":result});
                    if let Err(error) = process.send(&response).await {
                        *worker = None;
                        return Err(ToolError::Execution(format!(
                            "Python worker exited and session state was lost: {error}"
                        )));
                    }
                }
                _ => {
                    *worker = None;
                    return Err(ToolError::Execution(
                        "invalid Python worker message; session state was lost".into(),
                    ));
                }
            }
        }
    }
}

fn python_identifier(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(all(test, unix, any(target_os = "macos", target_os = "linux")))]
mod path_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn isolated_worker_resolves_python_on_daemon_path() {
        const MARKER: &str = "MEW_TEST_PYTHON_PATH_CHILD";
        if std::env::var_os(MARKER).is_some() {
            let mut worker = PythonWorker::start(&PythonOptions {
                executable: "mew-path-test-python".into(),
                isolated: true,
                cwd: std::env::current_dir().unwrap(),
            })
            .await
            .unwrap();
            worker.send(&serde_json::json!({"type":"exec","cell":"path","source":"import os\nassert 'MEW_TEST_DISCOVERY_SECRET' not in os.environ\ntext(42)","tools":[]})).await.unwrap();
            assert_eq!(worker.receive().await.unwrap()["output"], "42\n");
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let python = executable_on_path(Path::new("python3")).unwrap();
        std::os::unix::fs::symlink(python, dir.path().join("mew-python-helper")).unwrap();
        let shim = dir.path().join("mew-path-test-python");
        std::fs::write(&shim, "#!/bin/sh\nexec mew-python-helper \"$@\"\n").unwrap();
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        let paths = vec![dir.path().to_path_buf()];
        #[cfg(target_os = "linux")]
        {
            let bwrap = executable_on_path(Path::new("bwrap")).unwrap();
            std::os::unix::fs::symlink(bwrap, dir.path().join("bwrap")).unwrap();
        }
        #[cfg(target_os = "macos")]
        let paths: Vec<_> = paths
            .into_iter()
            .chain(std::env::split_paths(
                &std::env::var_os("PATH").unwrap_or_default(),
            ))
            .collect();
        let output = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "python::path_tests::isolated_worker_resolves_python_on_daemon_path",
                "--nocapture",
            ])
            .env(MARKER, "1")
            .env("MEW_TEST_DISCOVERY_SECRET", "sentinel")
            .env("PATH", std::env::join_paths(paths).unwrap())
            .kill_on_drop(true)
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
