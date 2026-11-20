#![cfg(unix)]

use std::{
    io::{Read, Write},
    os::fd::{AsRawFd, FromRawFd},
    os::unix::process::CommandExt,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

fn command(root: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mew"));
    command
        .current_dir(root)
        .env("MEW_CONFIG_DIR", root.join("config"))
        .env("MEW_DATA_DIR", root.join("data"))
        .env("MEW_CACHE_DIR", root.join("cache"))
        .env("MEW_SESSION_DIR", root.join("sessions"))
        .env("XDG_RUNTIME_DIR", root)
        .env("MEW_CRED_OPENCODE_ZEN", "unused")
        .env("TERM", "xterm-256color");
    command
}

struct TestProcess(Child);

impl Drop for TestProcess {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            if let Some(pid) = child_daemon(self.0.id()) {
                unsafe { libc::kill(pid, libc::SIGTERM) };
            }
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

impl std::ops::Deref for TestProcess {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}

impl std::ops::DerefMut for TestProcess {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

fn terminal(
    command: &mut Command,
) -> (
    TestProcess,
    std::fs::File,
    std::sync::Arc<std::sync::Mutex<String>>,
) {
    let (mut master, mut slave) = (0, 0);
    let mut size = libc::winsize {
        ws_row: 30,
        ws_col: 100,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        },
        0
    );
    let master = unsafe { std::fs::File::from_raw_fd(master) };
    let mut output = master.try_clone().unwrap();
    let captured = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let recorded = captured.clone();
    thread::spawn(move || {
        let mut buffer = [0; 8192];
        while let Ok(n) = output.read(&mut buffer) {
            if n == 0 {
                break;
            }
            recorded
                .lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&buffer[..n]));
        }
    });
    let slave = unsafe { std::fs::File::from_raw_fd(slave) };
    let child = command
        .process_group(0)
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave))
        .spawn()
        .unwrap();
    (TestProcess(child), master, captured)
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(100));
    }
}

fn wait_success(child: &mut TestProcess) {
    let mut status = None;
    wait_until(|| {
        status = child.try_wait().unwrap();
        status.is_some()
    });
    assert!(
        status.unwrap().success(),
        "process exited unsuccessfully: {status:?}"
    );
}

fn child_daemon(parent: u32) -> Option<i32> {
    let output = Command::new("ps")
        .args(["-A", "-o", "ppid=,pid=,command="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| {
            let mut fields = line.split_whitespace();
            let ppid = fields.next()?.parse::<u32>().ok()?;
            let pid = fields.next()?.parse::<i32>().ok()?;
            (ppid == parent && line.contains(" daemon ")).then_some(pid)
        })
}

#[test]
fn quit_stops_the_daemon_started_by_chat() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("cache")).unwrap();
    std::fs::write(root.path().join("cache/catalog.json"), "{}").unwrap();
    let marker = root.path().join("mcp-pid");
    let server = r#"
import json, os, pathlib, sys
pathlib.Path(sys.argv[1]).write_text(str(os.getpid()))
for line in sys.stdin:
    request = json.loads(line)
    if request['method'] == 'initialize':
        result = {'protocolVersion':'2025-11-25', 'capabilities':{'tools':{}}, 'serverInfo':{'name':'test','version':'1'}}
    elif request['method'] == 'tools/list':
        result = {'tools':[{'name':'ping','description':'ping','inputSchema':{'type':'object'}}]}
    else:
        continue
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}), flush=True)
"#;
    std::fs::write(root.path().join("mcp.json"), serde_json::json!({"mcpServers":{"test":{"command":"python3","args":["-u","-c",server,marker]}}}).to_string()).unwrap();
    let (mut tui, mut input, output) = terminal(command(root.path()).args([
        "chat",
        "--provider",
        "opencode-go",
        "--model",
        "hy4-flash-free",
    ]));
    let deadline = Instant::now() + Duration::from_secs(20);
    let daemon_pid = loop {
        if let Some(pid) = child_daemon(tui.id()) {
            break pid;
        }
        if Instant::now() >= deadline {
            panic!("chat did not retain its daemon as a child");
        }
        thread::sleep(Duration::from_millis(100));
    };
    wait_until(|| output.lock().unwrap().contains("connected"));
    wait_until(|| marker.exists());
    let mcp_pid: i32 = std::fs::read_to_string(marker).unwrap().parse().unwrap();
    input.write_all(b"/quit\r").unwrap();
    wait_success(&mut tui);
    let pid = daemon_pid;
    wait_until(|| unsafe { libc::kill(pid, 0) } != 0);
    wait_until(|| unsafe { libc::kill(mcp_pid, 0) } != 0);
}

#[test]
fn quit_leaves_an_attached_daemon_running() {
    let root = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut daemon = TestProcess(
        command(root.path())
            .args(["daemon", "--fake-provider", "--port", &address.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_until(|| std::net::TcpStream::connect(address).is_ok());
    let (mut tui, mut input, output) =
        terminal(command(root.path()).args(["chat", "--connect", &format!("ws://{address}")]));
    wait_until(|| output.lock().unwrap().contains("connected"));
    input.write_all(b"/quit\r").unwrap();
    wait_success(&mut tui);
    assert!(
        daemon.try_wait().unwrap().is_none(),
        "attached daemon exited"
    );
    unsafe {
        libc::kill(daemon.id() as i32, libc::SIGTERM);
    }
    wait_until(|| daemon.try_wait().unwrap().is_some());
}

#[test]
fn termination_signals_stop_owned_daemon() {
    for signal in [libc::SIGTERM, libc::SIGHUP] {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("cache")).unwrap();
        std::fs::write(root.path().join("cache/catalog.json"), "{}").unwrap();
        let (mut tui, input, output) = terminal(command(root.path()).args([
            "chat",
            "--provider",
            "opencode-go",
            "--model",
            "hy4-flash-free",
        ]));
        let mut daemon_pid = None;
        wait_until(|| {
            daemon_pid = child_daemon(tui.id());
            daemon_pid.is_some()
        });
        wait_until(|| output.lock().unwrap().contains("connected"));
        assert_eq!(unsafe { libc::kill(tui.id() as i32, signal) }, 0);
        wait_until(|| tui.try_wait().unwrap().is_some());
        let pid = daemon_pid.unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        let alive = unsafe { libc::kill(pid, 0) } == 0;
        if alive {
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
        assert!(!alive, "daemon survived TUI signal {signal}");
        let mut termios = std::mem::MaybeUninit::<libc::termios>::uninit();
        assert_eq!(
            unsafe { libc::tcgetattr(input.as_raw_fd(), termios.as_mut_ptr()) },
            0
        );
        let termios = unsafe { termios.assume_init() };
        assert_ne!(
            termios.c_lflag & libc::ICANON,
            0,
            "terminal remained in raw mode"
        );
        assert_ne!(
            termios.c_lflag & libc::ECHO,
            0,
            "terminal echo was not restored"
        );
    }
}

#[test]
fn signal_during_startup_stops_owned_daemon_and_mcp() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("cache")).unwrap();
    std::fs::write(root.path().join("cache/catalog.json"), "{}").unwrap();
    let marker = root.path().join("mcp-pid");
    let server = "import os,pathlib,sys,time; pathlib.Path(sys.argv[1]).write_text(str(os.getpid())); time.sleep(60)";
    std::fs::write(root.path().join("mcp.json"), serde_json::json!({"mcpServers":{"slow":{"command":"python3","args":["-u","-c",server,marker]}}}).to_string()).unwrap();
    let (mut tui, _input, _output) = terminal(command(root.path()).args([
        "chat",
        "--provider",
        "opencode-go",
        "--model",
        "hy4-flash-free",
    ]));
    let mut daemon_pid = None;
    wait_until(|| {
        daemon_pid = child_daemon(tui.id());
        daemon_pid.is_some()
    });
    wait_until(|| marker.exists());
    let mcp_pid: i32 = std::fs::read_to_string(marker).unwrap().parse().unwrap();
    assert_eq!(unsafe { libc::kill(-(tui.id() as i32), libc::SIGHUP) }, 0);
    wait_until(|| tui.try_wait().unwrap().is_some());
    wait_until(|| unsafe { libc::kill(daemon_pid.unwrap(), 0) } != 0);
    let deadline = Instant::now() + Duration::from_secs(5);
    while unsafe { libc::kill(mcp_pid, 0) } == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
    let alive = unsafe { libc::kill(mcp_pid, 0) } == 0;
    if alive {
        unsafe {
            libc::kill(mcp_pid, libc::SIGKILL);
        }
    }
    assert!(!alive, "MCP server survived group SIGHUP");
}

#[test]
fn owned_daemon_starts_with_long_tmpdir() {
    let root = tempfile::tempdir().unwrap();
    let long_tmp = root.path().join("long_directory_".repeat(10));
    std::fs::create_dir(&long_tmp).unwrap();
    std::fs::create_dir_all(root.path().join("cache")).unwrap();
    std::fs::write(root.path().join("cache/catalog.json"), "{}").unwrap();
    let (mut tui, mut input, output) =
        terminal(command(root.path()).env("TMPDIR", &long_tmp).args([
            "chat",
            "--provider",
            "opencode-go",
            "--model",
            "hy4-flash-free",
        ]));
    wait_until(|| {
        assert!(
            tui.try_wait().unwrap().is_none(),
            "TUI exited before connection"
        );
        output.lock().unwrap().contains("connected")
    });
    input.write_all(b"/quit\r").unwrap();
    let mut exit = None;
    wait_until(|| {
        exit = tui.try_wait().unwrap();
        exit.is_some()
    });
    assert!(exit.unwrap().success());
}

#[test]
fn termination_signals_leave_attached_daemon_running() {
    let root = tempfile::tempdir().unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let mut daemon = TestProcess(
        command(root.path())
            .args(["daemon", "--fake-provider", "--port", &address.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_until(|| std::net::TcpStream::connect(address).is_ok());
    for signal in [libc::SIGTERM, libc::SIGHUP] {
        let (mut tui, _input, output) =
            terminal(command(root.path()).args(["chat", "--connect", &format!("ws://{address}")]));
        wait_until(|| output.lock().unwrap().contains("connected"));
        assert_eq!(unsafe { libc::kill(tui.id() as i32, signal) }, 0);
        wait_success(&mut tui);
        assert!(
            daemon.try_wait().unwrap().is_none(),
            "attached daemon exited after signal {signal}"
        );
    }
    unsafe {
        libc::kill(daemon.id() as i32, libc::SIGTERM);
    }
    wait_success(&mut daemon);
}
