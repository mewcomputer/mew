//! Process lifecycle behind an explicitly selected execution boundary.
//!
//! A process group is a cleanup mechanism, not a filesystem or network sandbox.

#[cfg(unix)]
mod unix {
    use std::collections::VecDeque;
    use std::ffi::OsString;
    use std::io::{self, Read};
    #[cfg(target_os = "macos")]
    use std::os::fd::AsRawFd;
    #[cfg(target_os = "linux")]
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream as StdStream;
    #[cfg(target_os = "linux")]
    use std::os::unix::process::CommandExt;
    use std::os::unix::process::ExitStatusExt;
    use std::path::{Path, PathBuf};
    use std::process::ExitStatus;
    #[cfg(target_os = "linux")]
    use std::process::{Command, Stdio};
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::{Duration, Instant};
    use tokio::net::UnixStream;
    use tokio::sync::oneshot;

    const POLL: Duration = Duration::from_millis(20);
    const GRACE: Duration = Duration::from_millis(200);
    const CLEANUP_BOUND: Duration = Duration::from_secs(3);
    const CANCEL_BOUND: Duration = Duration::from_secs(4);
    const STDERR_LIMIT: usize = 8192;

    /// Complete subprocess specification. Environment is explicit, not inherited.
    #[derive(Debug)]
    pub struct LaunchSpec {
        pub program: PathBuf,
        pub args: Vec<OsString>,
        pub env: Vec<(OsString, OsString)>,
        pub cwd: PathBuf,
    }

    /// Prepares the spec for the caller-selected execution boundary.
    ///
    /// The launched process group must own the workload it confines. A local
    /// `docker exec` client does not own the process inside the container:
    /// killing that client is insufficient. A container boundary must own and
    /// stop the actual container, or independently prove workload termination.
    pub trait IsolationBoundary {
        fn wrap(&self, spec: LaunchSpec, workspace: &Path) -> io::Result<LaunchSpec>;
    }

    /// Launch the local CLI without an OS sandbox. Process-group cleanup still applies.
    pub struct UnsandboxedBoundary;

    impl IsolationBoundary for UnsandboxedBoundary {
        fn wrap(&self, mut spec: LaunchSpec, _: &Path) -> io::Result<LaunchSpec> {
            fn allowed(key: &std::ffi::OsStr) -> bool {
                let Some(key) = key.to_str() else {
                    return false;
                };
                matches!(
                    key,
                    "HOME"
                        | "PATH"
                        | "USER"
                        | "LOGNAME"
                        | "TMPDIR"
                        | "LANG"
                        | "LANGUAGE"
                        | "TERM"
                        | "HTTP_PROXY"
                        | "HTTPS_PROXY"
                        | "NO_PROXY"
                        | "http_proxy"
                        | "https_proxy"
                        | "no_proxy"
                        | "SSL_CERT_FILE"
                        | "SSL_CERT_DIR"
                        | "NODE_EXTRA_CA_CERTS"
                        | "GOOGLE_APPLICATION_CREDENTIALS"
                        | "GOOGLE_CLOUD_PROJECT"
                        | "GOOGLE_CLOUD_QUOTA_PROJECT"
                        | "XDG_CONFIG_HOME"
                        | "XDG_CACHE_HOME"
                        | "XDG_DATA_HOME"
                        | "XDG_STATE_HOME"
                        | "XDG_RUNTIME_DIR"
                ) || key.starts_with("LC_")
                    || key.starts_with("AGY_")
                    || key.starts_with("ANTIGRAVITY_")
                    || key.starts_with("GEMINI_")
            }
            let mut environment: Vec<_> = std::env::vars_os()
                .filter(|(key, _)| allowed(key))
                .collect();
            for (key, value) in spec.env {
                if allowed(&key) {
                    environment.retain(|(existing, _)| existing != &key);
                    environment.push((key, value));
                }
            }
            spec.env = environment;
            Ok(spec)
        }
    }

    #[derive(Debug)]
    pub enum ProcessOutcome {
        Exited(ExitStatus),
        Cancelled,
        TimedOut,
    }

    pub struct ManagedProcess {
        stdin: Option<UnixStream>,
        stdout: Option<UnixStream>,
        stderr: Arc<Mutex<VecDeque<u8>>>,
        cancel: mpsc::Sender<()>,
        completion: Option<oneshot::Receiver<io::Result<ProcessOutcome>>>,
    }

    impl std::fmt::Debug for ManagedProcess {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("ManagedProcess").finish_non_exhaustive()
        }
    }

    impl ManagedProcess {
        /// Start the wrapped command. Call inside a Tokio runtime with I/O enabled.
        /// The reaper retains the workspace until process-group cleanup is verified.
        pub fn spawn(
            spec: LaunchSpec,
            boundary: &impl IsolationBoundary,
            workspace: tempfile::TempDir,
            timeout: Duration,
        ) -> io::Result<Self> {
            if Instant::now().checked_add(timeout).is_none() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "process timeout is too large",
                ));
            }
            let spec = boundary.wrap(spec, workspace.path())?;
            let (stdin_parent, stdin_child) = StdStream::pair()?;
            let (stdout_parent, stdout_child) = StdStream::pair()?;
            let (mut stderr, stderr_child) = StdStream::pair()?;
            stdin_parent.set_nonblocking(true)?;
            stdout_parent.set_nonblocking(true)?;
            stderr.set_nonblocking(true)?;
            // Convert before spawning so a missing Tokio reactor cannot orphan a child.
            let stdin = UnixStream::from_std(stdin_parent)?;
            let stdout = UnixStream::from_std(stdout_parent)?;
            let pid = spawn_confined(spec, stdin_child, stdout_child, stderr_child)?;
            let mut owner = ReaperOwner {
                guard: Some(ChildGuard { pid, reaped: false }),
                workspace: Some(workspace),
            };
            let diagnostics = Arc::new(Mutex::new(VecDeque::new()));
            let reaper_diagnostics = Arc::clone(&diagnostics);
            let (cancel, cancelled) = mpsc::channel();
            let (done, completion) = oneshot::channel();
            std::thread::Builder::new()
                .name("mew-agy-reaper".into())
                .spawn(move || {
                    let result = supervise(
                        owner.guard.as_mut().expect("child exists"),
                        &mut stderr,
                        &reaper_diagnostics,
                        cancelled,
                        timeout,
                    );
                    drop(owner.guard.take());
                    let workspace = owner.workspace.take().expect("workspace exists");
                    let result = match result {
                        Ok(outcome) => {
                            drop(workspace);
                            Ok(outcome)
                        }
                        Err(error) => {
                            let path = workspace.keep();
                            Err(io::Error::new(
                                error.kind(),
                                format!("{error}; workspace retained at {}", path.display()),
                            ))
                        }
                    };
                    let _ = done.send(result);
                })?;
            Ok(Self {
                stdin: Some(stdin),
                stdout: Some(stdout),
                stderr: diagnostics,
                cancel,
                completion: Some(completion),
            })
        }
        pub fn take_stdin(&mut self) -> Option<UnixStream> {
            self.stdin.take()
        }
        pub fn take_stdout(&mut self) -> Option<UnixStream> {
            self.stdout.take()
        }
        /// Most recent stderr bytes. Treat as sensitive and redact before display or logging.
        pub fn stderr_tail(&self) -> Vec<u8> {
            self.stderr
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .copied()
                .collect()
        }
        pub async fn wait(&mut self) -> io::Result<ProcessOutcome> {
            let receiver = self
                .completion
                .as_mut()
                .ok_or_else(|| io::Error::other("process already collected"))?;
            let result = receiver
                .await
                .map_err(|_| io::Error::other("process reaper stopped"))?;
            self.completion = None;
            result
        }
        pub async fn cancel(&mut self) -> io::Result<ProcessOutcome> {
            let _ = self.cancel.send(());
            tokio::time::timeout(CANCEL_BOUND, self.wait())
                .await
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::TimedOut, "process cleanup is still running")
                })?
        }
    }
    impl Drop for ManagedProcess {
        fn drop(&mut self) {
            let _ = self.cancel.send(());
        }
    }

    // If reaper startup fails or panics, kill/reap first and retain the workspace
    // because descendant cleanup cannot be confirmed in this path.
    struct ReaperOwner {
        guard: Option<ChildGuard>,
        workspace: Option<tempfile::TempDir>,
    }
    impl Drop for ReaperOwner {
        fn drop(&mut self) {
            drop(self.guard.take());
            if let Some(workspace) = self.workspace.take() {
                let _ = workspace.keep();
            }
        }
    }
    struct ChildGuard {
        pid: libc::pid_t,
        reaped: bool,
    }
    impl ChildGuard {
        fn signal(&self, signal: libc::c_int) {
            unsafe {
                libc::kill(-self.pid, signal);
            }
        }
        fn reap(&mut self) -> io::Result<ExitStatus> {
            let mut status = 0;
            loop {
                if unsafe { libc::waitpid(self.pid, &mut status, 0) } >= 0 {
                    self.reaped = true;
                    return Ok(ExitStatus::from_raw(status));
                }
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
    }
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            if !self.reaped {
                self.signal(libc::SIGKILL);
                let _ = self.reap();
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn spawn_confined(
        spec: LaunchSpec,
        input: StdStream,
        output: StdStream,
        error: StdStream,
    ) -> io::Result<libc::pid_t> {
        let mut command = Command::new(spec.program);
        command
            .args(spec.args)
            .env_clear()
            .envs(spec.env)
            .current_dir(spec.cwd)
            .stdin(Stdio::from(OwnedFd::from(input)))
            .stdout(Stdio::from(OwnedFd::from(output)))
            .stderr(Stdio::from(OwnedFd::from(error)))
            .process_group(0);
        unsafe {
            command.pre_exec(|| {
                // Atomic CLOEXEC avoids closing std's exec-error pipe prematurely.
                let result = libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 4u32);
                if result < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let child = command.spawn()?;
        let pid = child.id() as libc::pid_t;
        // Child has no drop-based reaper; ChildGuard owns waitpid for this PID.
        drop(child);
        Ok(pid)
    }

    #[cfg(target_os = "macos")]
    fn spawn_confined(
        spec: LaunchSpec,
        input: StdStream,
        output: StdStream,
        error: StdStream,
    ) -> io::Result<libc::pid_t> {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        unsafe extern "C" {
            fn posix_spawn_file_actions_addchdir_np(
                actions: *mut libc::posix_spawn_file_actions_t,
                path: *const libc::c_char,
            ) -> libc::c_int;
        }
        fn cstring(value: &std::ffi::OsStr) -> io::Result<CString> {
            CString::new(value.as_bytes()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "NUL in launch specification")
            })
        }
        fn check(code: libc::c_int) -> io::Result<()> {
            if code == 0 {
                Ok(())
            } else {
                Err(io::Error::from_raw_os_error(code))
            }
        }
        let program = cstring(spec.program.as_os_str())?;
        let cwd = cstring(spec.cwd.as_os_str())?;
        let mut arguments = vec![program.clone()];
        for arg in &spec.args {
            arguments.push(cstring(arg)?);
        }
        let mut env = Vec::with_capacity(spec.env.len());
        for (name, value) in &spec.env {
            if name.is_empty() || name.as_bytes().contains(&b'=') {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid environment key",
                ));
            }
            let mut pair = name.as_bytes().to_vec();
            pair.push(b'=');
            pair.extend_from_slice(value.as_bytes());
            env.push(
                CString::new(pair).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "NUL in environment")
                })?,
            );
        }
        let mut argv: Vec<*mut libc::c_char> =
            arguments.iter().map(|s| s.as_ptr() as *mut _).collect();
        argv.push(std::ptr::null_mut());
        let mut envp: Vec<*mut libc::c_char> = env.iter().map(|s| s.as_ptr() as *mut _).collect();
        envp.push(std::ptr::null_mut());
        unsafe {
            let mut actions = std::mem::zeroed();
            check(libc::posix_spawn_file_actions_init(&mut actions))?;
            let mut attr = std::mem::zeroed();
            if let Err(error) = check(libc::posix_spawnattr_init(&mut attr)) {
                libc::posix_spawn_file_actions_destroy(&mut actions);
                return Err(error);
            }
            let result = (|| {
                check(posix_spawn_file_actions_addchdir_np(
                    &mut actions,
                    cwd.as_ptr(),
                ))?;
                for (source, dest) in [
                    (input.as_raw_fd(), 0),
                    (output.as_raw_fd(), 1),
                    (error.as_raw_fd(), 2),
                ] {
                    if source < 3 {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "standard descriptors must be open",
                        ));
                    }
                    check(libc::posix_spawn_file_actions_adddup2(
                        &mut actions,
                        source,
                        dest,
                    ))?;
                    check(libc::posix_spawn_file_actions_addclose(
                        &mut actions,
                        source,
                    ))?;
                }
                check(libc::posix_spawnattr_setflags(
                    &mut attr,
                    (libc::POSIX_SPAWN_CLOEXEC_DEFAULT | libc::POSIX_SPAWN_SETPGROUP) as i16,
                ))?;
                check(libc::posix_spawnattr_setpgroup(&mut attr, 0))?;
                let mut pid = 0;
                check(libc::posix_spawn(
                    &mut pid,
                    program.as_ptr(),
                    &actions,
                    &attr,
                    argv.as_ptr(),
                    envp.as_ptr(),
                ))?;
                Ok(pid)
            })();
            libc::posix_spawnattr_destroy(&mut attr);
            libc::posix_spawn_file_actions_destroy(&mut actions);
            result
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn spawn_confined(
        _: LaunchSpec,
        _: StdStream,
        _: StdStream,
        _: StdStream,
    ) -> io::Result<libc::pid_t> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "descriptor isolation unavailable",
        ))
    }

    fn exited_without_reaping(pid: libc::pid_t) -> io::Result<bool> {
        loop {
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    pid as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result == 0 {
                return Ok(info.si_signo != 0);
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
    #[cfg(target_os = "macos")]
    fn group_has_other_members(group: libc::pid_t) -> io::Result<bool> {
        unsafe extern "C" {
            fn proc_listpids(
                kind: u32,
                info: u32,
                buffer: *mut libc::c_void,
                size: libc::c_int,
            ) -> libc::c_int;
        }
        let mut pids = vec![0i32; 65_536];
        let bytes = unsafe {
            proc_listpids(
                2,
                group as u32,
                pids.as_mut_ptr().cast(),
                std::mem::size_of_val(pids.as_slice()) as libc::c_int,
            )
        };
        if bytes < 0 {
            return Err(io::Error::last_os_error());
        }
        if bytes as usize >= std::mem::size_of_val(pids.as_slice()) {
            return Err(io::Error::other("too many process group members to verify"));
        }
        Ok(pids[..bytes as usize / std::mem::size_of::<i32>()]
            .iter()
            .any(|pid| *pid > 0 && *pid != group))
    }

    #[cfg(target_os = "linux")]
    fn group_has_other_members(group: libc::pid_t) -> io::Result<bool> {
        for entry in std::fs::read_dir("/proc")? {
            let entry = entry?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(pid) = name.parse::<libc::pid_t>() else {
                continue;
            };
            if pid == group {
                continue;
            }
            let stat = match std::fs::read_to_string(entry.path().join("stat")) {
                Ok(stat) => stat,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error),
            };
            let Some((_, fields)) = stat.rsplit_once(')') else {
                return Err(io::Error::other("invalid process stat"));
            };
            let pgrp = fields
                .split_whitespace()
                .nth(2)
                .and_then(|field| field.parse::<libc::pid_t>().ok())
                .ok_or_else(|| io::Error::other("invalid process group in stat"))?;
            if pgrp == group {
                return Ok(true);
            }
        }
        Ok(false)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn group_has_other_members(_: libc::pid_t) -> io::Result<bool> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "process group verification unavailable",
        ))
    }

    fn drain_stderr(
        stderr: &mut StdStream,
        diagnostics: &Mutex<VecDeque<u8>>,
        budget: usize,
    ) -> io::Result<()> {
        let mut bytes = [0u8; 4096];
        let mut total = 0;
        while total < budget {
            match stderr.read(&mut bytes) {
                Ok(0) => break,
                Ok(count) => {
                    total += count;
                    let mut tail = diagnostics.lock().unwrap_or_else(|e| e.into_inner());
                    for byte in &bytes[..count] {
                        if tail.len() == STDERR_LIMIT {
                            tail.pop_front();
                        }
                        tail.push_back(*byte);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
    fn supervise(
        guard: &mut ChildGuard,
        stderr: &mut StdStream,
        diagnostics: &Mutex<VecDeque<u8>>,
        cancelled: mpsc::Receiver<()>,
        timeout: Duration,
    ) -> io::Result<ProcessOutcome> {
        let deadline = Instant::now() + timeout;
        let context = |operation: &str, error: io::Error| {
            io::Error::new(error.kind(), format!("{operation}: {error}"))
        };
        let reason = loop {
            drain_stderr(stderr, diagnostics, 65_536)
                .map_err(|error| context("read stderr", error))?;
            if exited_without_reaping(guard.pid)
                .map_err(|error| context("observe process exit", error))?
            {
                break None;
            }
            if Instant::now() >= deadline {
                break Some(ProcessOutcome::TimedOut);
            }
            match cancelled.recv_timeout(POLL) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Some(ProcessOutcome::Cancelled)
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        };
        if reason.is_some() {
            guard.signal(libc::SIGTERM);
            std::thread::sleep(GRACE);
        }
        // Keep the leader unreaped until the group is signalled, preventing PID reuse.
        guard.signal(libc::SIGKILL);
        let deadline = Instant::now() + CLEANUP_BOUND;
        loop {
            let leader_exited = exited_without_reaping(guard.pid)
                .map_err(|error| context("observe killed process", error))?;
            let others_remain = group_has_other_members(guard.pid)
                .map_err(|error| context("check process group", error))?;
            if leader_exited && !others_remain {
                break;
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "process group did not exit",
                ));
            }
            std::thread::sleep(POLL);
        }
        let status = guard
            .reap()
            .map_err(|error| context("reap process", error))?;
        drain_stderr(stderr, diagnostics, usize::MAX)
            .map_err(|error| context("read final stderr", error))?;
        Ok(reason.unwrap_or(ProcessOutcome::Exited(status)))
    }
}

#[cfg(unix)]
pub use unix::{
    IsolationBoundary, LaunchSpec, ManagedProcess, ProcessOutcome, UnsandboxedBoundary,
};

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    struct Boundary;
    impl IsolationBoundary for Boundary {
        fn wrap(&self, spec: LaunchSpec, _: &std::path::Path) -> io::Result<LaunchSpec> {
            Ok(spec)
        }
    }

    struct Unavailable;
    impl IsolationBoundary for Unavailable {
        fn wrap(&self, _: LaunchSpec, _: &std::path::Path) -> io::Result<LaunchSpec> {
            Err(io::Error::other("isolation unavailable"))
        }
    }

    fn command(script: &str) -> LaunchSpec {
        LaunchSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
            cwd: "/tmp".into(),
        }
    }

    #[tokio::test]
    async fn unavailable_boundary_refuses_to_spawn() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("ran");
        let script = format!("touch {}", marker.display());
        let error = ManagedProcess::spawn(
            command(&script),
            &Unavailable,
            directory,
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(error.to_string().contains("isolation unavailable"));
        assert!(!marker.exists());
    }

    #[tokio::test]
    async fn streams_stdio_and_waits_for_successful_exit() {
        let directory = tempfile::tempdir().unwrap();
        let mut process = ManagedProcess::spawn(
            command("read line; printf '%s\\n' \"$line\""),
            &Boundary,
            directory,
            Duration::from_secs(5),
        )
        .unwrap();
        let mut stdin = process.take_stdin().unwrap();
        stdin.write_all(b"hello\n").await.unwrap();
        drop(stdin);
        let mut line = String::new();
        BufReader::new(process.take_stdout().unwrap())
            .read_line(&mut line)
            .await
            .unwrap();
        assert_eq!(line, "hello\n");
        assert!(
            matches!(process.wait().await.unwrap(), ProcessOutcome::Exited(status) if status.success())
        );
    }

    async fn child_pid(process: &mut ManagedProcess) -> i32 {
        let mut line = String::new();
        tokio::time::timeout(
            Duration::from_secs(3),
            BufReader::new(process.take_stdout().unwrap()).read_line(&mut line),
        )
        .await
        .unwrap()
        .unwrap();
        line.trim().parse().unwrap()
    }

    async fn gone(pid: i32) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if unsafe { libc::kill(pid, 0) } == -1
                    && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("descendant must be gone");
    }

    #[tokio::test]
    async fn child_environment_is_explicit_and_does_not_inherit_home() {
        let directory = tempfile::tempdir().unwrap();
        let mut process = ManagedProcess::spawn(
            command("if test \"${HOME+x}\"; then echo inherited; else echo clean; fi"),
            &Boundary,
            directory,
            Duration::from_secs(5),
        )
        .unwrap();
        let mut line = String::new();
        BufReader::new(process.take_stdout().unwrap())
            .read_line(&mut line)
            .await
            .unwrap();
        assert_eq!(line, "clean\n");
        assert!(
            matches!(process.wait().await.unwrap(), ProcessOutcome::Exited(status) if status.success())
        );
    }

    #[tokio::test]
    async fn cancellation_escalates_when_group_ignores_term() {
        let directory = tempfile::tempdir().unwrap();
        let mut process = ManagedProcess::spawn(
            command("trap '' TERM; sleep 30 & echo $!; wait"),
            &Boundary,
            directory,
            Duration::from_secs(30),
        )
        .unwrap();
        let pid = child_pid(&mut process).await;
        assert!(matches!(
            process.cancel().await.unwrap(),
            ProcessOutcome::Cancelled
        ));
        gone(pid).await;
    }

    #[tokio::test]
    async fn stderr_diagnostics_are_bounded_and_available_without_logging() {
        let directory = tempfile::tempdir().unwrap();
        let mut process = ManagedProcess::spawn(
            command("printf '%020000d' 0 >&2; printf 'END-DIAGNOSTIC' >&2"),
            &Boundary,
            directory,
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(
            matches!(process.wait().await.unwrap(), ProcessOutcome::Exited(status) if status.success())
        );
        let output = process.stderr_tail();
        assert!(output.len() <= 8192);
        assert!(output.ends_with(b"END-DIAGNOSTIC"));
    }

    #[tokio::test]
    async fn cancellation_terminates_descendants_and_releases_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_owned();
        let mut process = ManagedProcess::spawn(
            command("sleep 30 & echo $!; wait"),
            &Boundary,
            directory,
            Duration::from_secs(30),
        )
        .unwrap();
        let pid = child_pid(&mut process).await;
        assert!(path.exists());
        assert!(matches!(
            process.cancel().await.unwrap(),
            ProcessOutcome::Cancelled
        ));
        gone(pid).await;
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn timeout_terminates_descendants() {
        let directory = tempfile::tempdir().unwrap();
        let mut process = ManagedProcess::spawn(
            command("sleep 30 & echo $!; wait"),
            &Boundary,
            directory,
            Duration::from_millis(400),
        )
        .unwrap();
        let pid = child_pid(&mut process).await;
        assert!(matches!(
            process.wait().await.unwrap(),
            ProcessOutcome::TimedOut
        ));
        gone(pid).await;
    }

    #[tokio::test]
    async fn drop_still_reaps_process_group_and_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_owned();
        let mut process = ManagedProcess::spawn(
            command("sleep 30 & echo $!; wait"),
            &Boundary,
            directory,
            Duration::from_secs(30),
        )
        .unwrap();
        let pid = child_pid(&mut process).await;
        drop(process);
        gone(pid).await;
        tokio::time::timeout(Duration::from_secs(3), async {
            while path.exists() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("workspace must be released after reaping");
    }

    #[test]
    fn drop_after_runtime_shutdown_still_cleans_child_and_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_owned();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (process, pid) = runtime.block_on(async {
            let mut process = ManagedProcess::spawn(
                command("sleep 30 & echo $!; wait"),
                &Boundary,
                directory,
                Duration::from_secs(30),
            )
            .unwrap();
            let pid = child_pid(&mut process).await;
            (process, pid)
        });
        drop(runtime);
        drop(process);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while path.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !path.exists(),
            "workspace must be released after runtime shutdown"
        );
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }

    #[tokio::test]
    async fn normal_leader_exit_cleans_remaining_descendants_before_workspace() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().to_owned();
        let mut process = ManagedProcess::spawn(
            command("sleep 30 & echo $!; exit 0"),
            &Boundary,
            directory,
            Duration::from_secs(5),
        )
        .unwrap();
        let pid = child_pid(&mut process).await;
        assert!(
            matches!(process.wait().await.unwrap(), ProcessOutcome::Exited(status) if status.success())
        );
        assert!(!path.exists());
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "descendant must exit before workspace removal"
        );
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }

    #[tokio::test]
    async fn failed_exec_is_reported_to_caller() {
        let directory = tempfile::tempdir().unwrap();
        let error = ManagedProcess::spawn(
            LaunchSpec {
                program: "/mew-does-not-exist".into(),
                ..command("unused")
            },
            &Boundary,
            directory,
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn unrelated_non_cloexec_fd_is_not_inherited() {
        use std::os::fd::AsRawFd;
        let directory = tempfile::tempdir().unwrap();
        let file = std::fs::File::create(directory.path().join("secret")).unwrap();
        let fd = file.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert!(flags >= 0);
        assert_eq!(
            unsafe { libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) },
            0
        );
        let script = format!("if test -e /dev/fd/{fd}; then echo inherited; else echo closed; fi");
        let mut process = ManagedProcess::spawn(
            command(&script),
            &Boundary,
            directory,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(unsafe { libc::fcntl(fd, libc::F_SETFD, flags) }, 0);
        let mut line = String::new();
        BufReader::new(process.take_stdout().unwrap())
            .read_line(&mut line)
            .await
            .unwrap();
        assert_eq!(line, "closed\n");
        assert!(
            matches!(process.wait().await.unwrap(), ProcessOutcome::Exited(status) if status.success())
        );
    }
}
