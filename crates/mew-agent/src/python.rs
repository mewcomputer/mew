use std::collections::BTreeSet;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use async_trait::async_trait;
use mew_tools::{Sensitivity, Tool, ToolCtx, ToolError};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const WORKER: &str = include_str!("python_worker.py");
const MAX_FRAME_BYTES: u64 = 16 * 1024 * 1024;

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
}

pub(crate) struct PythonWorker {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    #[cfg(unix)]
    process_group: bool,
    diagnostics: Arc<std::sync::Mutex<Vec<u8>>>,
    stderr_task: Option<tokio::task::JoinHandle<()>>,
    _scratch: tempfile::TempDir,
}

/// A live Python environment shared by the runtime instances of one session.
#[derive(Clone, Default)]
pub struct PythonSession(Arc<tokio::sync::Mutex<Option<PythonWorker>>>);

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
            Command::new(&options.executable)
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
        const DISCOVER: &str = "import json,os,site,sys,sysconfig; libdir=sysconfig.get_config_var('LIBDIR') or ''; lib=sysconfig.get_config_var('LDLIBRARY') or ''; root=os.path.dirname(os.path.dirname(sys.executable)); paths=sys.path + site.getsitepackages() + [os.path.join(libdir,lib),os.path.join(sys.base_prefix,'Python'),os.path.join(root,'pyvenv.cfg'),os.path.join(root,'lib','python%d.%d'%sys.version_info[:2],'site-packages')]; print(json.dumps({'executable':sys.executable,'base':sys.base_prefix,'paths':paths}))";
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            Command::new(python)
                .args(["-I", "-S", "-c", DISCOVER])
                .env_clear()
                .env("PATH", "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin")
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
            let mut command = Command::new("bwrap");
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
            isolated: self.code_mode.isolated,
        };
        let catalog = self.python_catalog();
        mew_provider::ToolDef {
            name: tool.name().into(),
            description: format!("Run Python in a persistent session. Use text(value) to send output to the model. Call tools as tools.name({{...}}) or tools.call('exact-name', {{...}}); results have output, error, and images fields. Use emit_image(result['images'][0]) to view an image. The available tools and their schemas are: {}", serde_json::Value::Array(catalog)),
            schema: tool.schema().clone(),
        }
    }

    pub(crate) async fn execute_python(
        &self,
        ctx: &ToolCtx,
        input: Value,
        events: &tokio::sync::mpsc::Sender<crate::AgentEvent>,
    ) -> Result<mew_hooks::ToolOutput, ToolError> {
        let source = input
            .get("source")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::InvalidInput("source must be a string".into()))?;
        let catalog = self.python_catalog();
        let mut worker = self.python_session.0.lock().await;
        if worker.is_none() {
            let options = PythonOptions {
                executable: self
                    .code_mode
                    .python
                    .clone()
                    .unwrap_or_else(|| PathBuf::from("python3")),
                isolated: self.code_mode.isolated,
            };
            let started = tokio::select! {
                _ = ctx.cancel.cancelled() => return Err(ToolError::Execution("Python was cancelled".into())),
                result = PythonWorker::start(&options) => result,
            };
            *worker = Some(
                started
                    .map_err(|e| ToolError::Execution(format!("could not start Python: {e}")))?,
            );
        }
        let process = worker.as_mut().expect("worker started");
        let cell = ulid::Ulid::new().to_string();
        if let Err(error) = process
            .send(&serde_json::json!({"type":"exec","cell":cell,"source":source,"tools":catalog}))
            .await
        {
            *worker = None;
            return Err(ToolError::Execution(format!(
                "Python worker exited and session state was lost: {error}"
            )));
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
                    return Err(ToolError::Execution(format!(
                        "Python worker exited and session state was lost: {error}"
                    )));
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
                    let images =
                        serde_json::from_value(message["images"].clone()).map_err(|error| {
                            ToolError::Execution(format!("invalid Python images: {error}"))
                        })?;
                    return Ok(mew_hooks::ToolOutput {
                        output: message["output"].as_str().unwrap_or("").to_owned(),
                        images,
                        ..Default::default()
                    });
                }
                Some("error") => {
                    return Ok(mew_hooks::ToolOutput {
                        error: format!(
                            "{}{}",
                            message["error"].as_str().unwrap_or("Python error"),
                            message["output"]
                                .as_str()
                                .filter(|s| !s.is_empty())
                                .map(|s| format!("\n{s}"))
                                .unwrap_or_default()
                        ),
                        ..Default::default()
                    })
                }
                Some("tool") => {
                    let name = message["name"].as_str().unwrap_or("");
                    if !self.tool_available(name) {
                        let response = serde_json::json!({"type":"tool_result","cell":cell,"id":message["id"],"result":{"output":"","error":"tool unavailable"}});
                        if let Err(error) = process.send(&response).await {
                            *worker = None;
                            return Err(ToolError::Execution(format!(
                                "Python worker exited and session state was lost: {error}"
                            )));
                        }
                        continue;
                    }
                    let call_id = format!("{}:{}", ctx.call_id, message["id"]);
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
                    let events = events.clone();
                    let parent = ctx.call_id.clone();
                    let name_for_event = name.to_owned();
                    let forwarder = tokio::spawn(async move {
                        while let Some(event) = nested_rx.recv().await {
                            match event {
                                crate::AgentEvent::ToolStart { .. } => {
                                    let _ = events
                                        .send(crate::AgentEvent::ToolProgress {
                                            call_id: parent.clone(),
                                            chunk: format!("[{name_for_event}]\n"),
                                        })
                                        .await;
                                }
                                crate::AgentEvent::ToolEnd { .. } => {}
                                crate::AgentEvent::PartUpdated { ref part, .. } if !matches!(part, mew_message::Part::ToolCall(call) if call.tool_name == "exit_tool") =>
                                    {}
                                crate::AgentEvent::ToolProgress { chunk, .. } => {
                                    let _ = events
                                        .send(crate::AgentEvent::ToolProgress {
                                            call_id: parent.clone(),
                                            chunk,
                                        })
                                        .await;
                                }
                                other => {
                                    let _ = events.send(other).await;
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
