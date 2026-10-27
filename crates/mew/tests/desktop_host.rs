use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mew"));
    command
        .current_dir(root)
        .env("MEW_CONFIG_DIR", root.join("config"))
        .env("MEW_DATA_DIR", root.join("data"))
        .env("MEW_CACHE_DIR", root.join("cache"))
        .env("MEW_SESSION_DIR", root.join("sessions"))
        .env("XDG_RUNTIME_DIR", root)
        .env("RUST_LOG", "error")
        .env_remove("MEW_DESKTOP_DAEMON_URL")
        .env_remove("MEW_DESKTOP_DAEMON_BINARY")
        .env_remove("MEW_DESKTOP_REMOTE");
    command
}

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("config/themes")).unwrap();
    std::fs::create_dir_all(root.path().join("cache")).unwrap();
    // A valid fresh catalog keeps this lifecycle test independent of the network.
    std::fs::write(root.path().join("cache/catalog.json"), "{}").unwrap();
    std::fs::write(
        root.path().join("config/themes/desktop-test.json"),
        r##"{"name":"desktop-test","mode":"light","tokens":{"background":"#123456"}}"##,
    )
    .unwrap();
    root
}

fn unused_address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

struct Process {
    child: Child,
    input: Option<ChildStdin>,
    output: Receiver<Value>,
    stderr: std::path::PathBuf,
}

impl Process {
    fn start(root: &Path, address: SocketAddr, attached: bool) -> Self {
        let stderr = root.join("host-stderr.log");
        let mut command = command(root);
        command
            .arg("desktop-host")
            .env("MEW_DESKTOP_DAEMON_PORT", address.port().to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(std::fs::File::create(&stderr).unwrap());
        if attached {
            command.env("MEW_DESKTOP_DAEMON_URL", format!("ws://{address}"));
        }
        let mut child = command.spawn().unwrap();
        let input = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let (sender, output) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let value: Value = serde_json::from_str(&line.unwrap()).unwrap();
                if sender.send(value).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            output,
            stderr,
        }
    }

    fn read(&self) -> Value {
        self.output
            .recv_timeout(Duration::from_secs(15))
            .unwrap_or_else(|error| {
                panic!(
                    "desktop host did not return JSON: {error}; stderr: {}",
                    std::fs::read_to_string(&self.stderr).unwrap()
                )
            })
    }

    fn request(&mut self, value: Value) -> Value {
        writeln!(self.input.as_mut().unwrap(), "{value}").unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
        self.read()
    }

    fn eof(&mut self) {
        drop(self.input.take());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "{}",
                    std::fs::read_to_string(&self.stderr).unwrap()
                );
                return;
            }
            assert!(
                Instant::now() < deadline,
                "desktop host did not stop after EOF"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        drop(self.input.take());
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Daemon(Child);

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn get(address: SocketAddr) -> String {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

#[test]
fn host_serves_embedded_assets_preserves_themes_and_stops_owned_resources_on_eof() {
    let root = fixture();
    let state_path = root.path().join("config/state.toml");
    std::fs::write(
        &state_path,
        "theme = \"nord\"\nlast_model = \"preserved\"\n",
    )
    .unwrap();
    let daemon = unused_address();
    let mut process = Process::start(root.path(), daemon, false);
    let ready = process.read();
    assert_eq!(ready["type"], "ready");
    let bootstrap = &ready["bootstrap"];
    assert_eq!(bootstrap["daemonUrl"], format!("ws://{daemon}"));
    assert_eq!(bootstrap["themeMode"], "system");
    assert_eq!(bootstrap["lightTheme"], "codex-light");
    assert_eq!(bootstrap["darkTheme"], "codex-dark");
    let themes = bootstrap["themes"].as_array().unwrap();
    let custom = themes
        .iter()
        .find(|theme| theme["id"] == "desktop-test")
        .unwrap();
    assert_eq!(custom["mode"], "light");
    assert!(custom["css"]
        .as_str()
        .unwrap()
        .contains("--background: #123456;"));
    assert!(themes.iter().any(|theme| theme["id"] == "codex-dark"));
    let assets: SocketAddr = ready["appUrl"]
        .as_str()
        .unwrap()
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .parse()
        .unwrap();
    assert!(assets.ip().is_loopback());
    let response = get(assets);
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(response.contains("<!doctype html>"));
    assert_eq!(
        process.request(json!({"id":1,"request":{"type":"bootstrap"}})),
        json!({"id":1,"result":bootstrap})
    );
    assert_eq!(process.request(json!({"id":"theme","request":{"type":"set_theme","themeMode":"light","lightTheme":"desktop-test","darkTheme":"nord"}})), json!({"id":"theme","result":{}}));
    let loaded = process.request(json!({"id":2,"request":{"type":"bootstrap"}}));
    assert_eq!(loaded["result"]["themeMode"], "light");
    assert_eq!(loaded["result"]["lightTheme"], "desktop-test");
    let state = mew_config::load_state_from(&state_path).unwrap();
    assert_eq!(state.last_model, "preserved");
    assert_eq!(state.theme, "nord");
    assert_eq!(state.desktop_light_theme, "desktop-test");
    process.eof();
    assert!(TcpStream::connect(daemon).is_err());
    assert!(TcpStream::connect(assets).is_err());
    assert!(std::fs::read_to_string(&process.stderr).unwrap().is_empty());
    let mut restarted = Process::start(root.path(), daemon, false);
    let saved = restarted.read();
    assert_eq!(saved["bootstrap"]["themeMode"], "light");
    assert_eq!(saved["bootstrap"]["lightTheme"], "desktop-test");
    assert_eq!(saved["bootstrap"]["darkTheme"], "nord");
    restarted.eof();
}

#[test]
fn malformed_requests_do_not_change_theme_state_and_do_not_break_later_requests() {
    let root = fixture();
    let mut process = Process::start(root.path(), unused_address(), false);
    let initial = process.read()["bootstrap"].clone();
    for request in [
        json!({"type":"set_theme","themeMode":"automatic","lightTheme":"codex-light","darkTheme":"nord"}),
        json!({"type":"set_theme","themeMode":"dark","lightTheme":"missing-theme","darkTheme":"nord"}),
        json!({"type":"set_theme","themeMode":"dark","lightTheme":"codex-light","darkTheme":"nord","extra":true}),
        json!({"type":"bootstrap","extra":true}),
        json!({"type":"unsupported"}),
    ] {
        let reply = process.request(json!({"id":7,"request":request}));
        assert_eq!(reply["id"], 7);
        assert!(reply["error"].is_string(), "{reply}");
        assert!(reply.get("result").is_none());
    }
    writeln!(process.input.as_mut().unwrap(), "not json").unwrap();
    let reply = process.read();
    assert_eq!(reply["id"], Value::Null);
    assert!(reply["error"].is_string());
    let mut boundary = json!({"id":8,"request":{"type":"bootstrap"}}).to_string();
    boundary.extend(std::iter::repeat_n(' ', 16_384 - boundary.len()));
    writeln!(process.input.as_mut().unwrap(), "{boundary}").unwrap();
    assert_eq!(process.read(), json!({"id":8,"result":initial}));
    writeln!(process.input.as_mut().unwrap(), "{boundary} ").unwrap();
    assert!(process.read()["error"].as_str().unwrap().contains("16 KiB"));
    for envelope in [
        json!({"id":{},"request":{"type":"bootstrap"}}),
        json!({"request":{"type":"bootstrap"}}),
        json!({"id":7,"request":{"type":"bootstrap"},"extra":true}),
        json!({"id":7}),
    ] {
        assert!(process.request(envelope)["error"].is_string());
    }
    assert_eq!(
        process.request(json!({"id":9,"request":{"type":"bootstrap"}}))["result"],
        initial
    );
    assert!(!root.path().join("config/state.toml").exists());
    process.eof();
    assert!(std::fs::read_to_string(&process.stderr).unwrap().is_empty());
}

#[test]
fn attached_real_daemon_survives_the_host_eof() {
    let root = fixture();
    let address = unused_address();
    let mut daemon = Daemon(
        command(root.path())
            .args(["daemon", "--fake-provider", "--port", &address.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while TcpStream::connect(address).is_err() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    for explicit_url in [true, false] {
        let mut host = Process::start(root.path(), address, explicit_url);
        assert_eq!(
            host.read()["bootstrap"]["daemonUrl"],
            format!("ws://{address}")
        );
        host.eof();
        assert!(daemon.0.try_wait().unwrap().is_none());
        assert!(TcpStream::connect(address).is_ok());
    }
}

#[test]
fn startup_failure_emits_no_ready_record_or_listening_assets() {
    let root = fixture();
    let address = unused_address();
    let output = command(root.path())
        .arg("desktop-host")
        .env("MEW_DESKTOP_DAEMON_PORT", address.port().to_string())
        .env(
            "MEW_DESKTOP_DAEMON_BINARY",
            root.path().join("missing-daemon"),
        )
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("configured daemon"), "{stderr}");
    assert!(stderr.contains("missing-daemon"), "{stderr}");
    assert!(TcpStream::connect(address).is_err());
}

#[test]
fn failed_theme_persistence_keeps_the_live_bootstrap_unchanged() {
    let root = fixture();
    std::fs::create_dir(root.path().join("config/state.toml")).unwrap();
    let mut process = Process::start(root.path(), unused_address(), false);
    let initial = process.read()["bootstrap"].clone();
    let reply = process.request(json!({"id":1,"request":{"type":"set_theme","themeMode":"light","lightTheme":"desktop-test","darkTheme":"nord"}}));
    assert_eq!(reply["id"], 1);
    assert!(reply["error"].is_string());
    assert_eq!(
        process.request(json!({"id":2,"request":{"type":"bootstrap"}}))["result"],
        initial
    );
    assert!(root.path().join("config/state.toml").is_dir());
    process.eof();
}
