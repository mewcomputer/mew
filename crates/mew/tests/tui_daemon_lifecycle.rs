#![cfg(unix)]

use std::{
    io::{Read, Write},
    os::fd::FromRawFd,
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
            if let Ok(contents) = std::fs::read_to_string(root.path().join("mew.pid")) {
                if let Ok(pid) = contents.trim().parse::<i32>() {
                    unsafe {
                        libc::kill(pid, libc::SIGTERM);
                    }
                }
            }
            let _ = tui.kill();
            let _ = tui.wait();
            panic!("chat did not retain its daemon as a child");
        }
        thread::sleep(Duration::from_millis(100));
    };
    wait_until(|| output.lock().unwrap().contains("connected"));
    wait_until(|| marker.exists());
    let mcp_pid: i32 = std::fs::read_to_string(marker).unwrap().parse().unwrap();
    input.write_all(b"/quit\r").unwrap();
    wait_until(|| tui.try_wait().unwrap().is_some());
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
    wait_until(|| tui.try_wait().unwrap().is_some());
    assert!(
        daemon.try_wait().unwrap().is_none(),
        "attached daemon exited"
    );
    unsafe {
        libc::kill(daemon.id() as i32, libc::SIGTERM);
    }
    wait_until(|| daemon.try_wait().unwrap().is_some());
}
