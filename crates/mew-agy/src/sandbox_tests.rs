#![cfg(target_os = "macos")]

use crate::{
    process::{IsolationBoundary, LaunchSpec, ManagedProcess, ProcessOutcome},
    sandbox::SeatbeltBoundary,
};

#[test]
fn sandbox_preserves_existing_agy_identity_environment() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    let cli_home = root.path().join("cli-home");
    fs::create_dir(&profile).unwrap();
    fs::create_dir(&cli_home).unwrap();
    let workspace = tempfile::tempdir().unwrap();
    config(workspace.path(), 1234);
    let boundary = SeatbeltBoundary::for_test(profile, 1235, cli_home);
    let spec = LaunchSpec {
        program: "/usr/bin/true".into(),
        args: vec![],
        env: vec![],
        cwd: workspace.path().to_owned(),
    };
    let wrapped = boundary.wrap(spec, workspace.path()).unwrap();
    for name in [
        "HOME",
        "ANTIGRAVITY_APP_DATA_DIR",
        "XDG_CONFIG_HOME",
        "XDG_CACHE_HOME",
        "XDG_DATA_HOME",
    ] {
        let actual = wrapped
            .env
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone());
        assert_eq!(
            actual,
            std::env::var_os(name),
            "{name} must retain the locally configured identity path"
        );
    }
}
use std::{ffi::OsString, fs, net::TcpListener, path::Path, process::Command, time::Duration};
use tokio::io::AsyncReadExt;

fn canary(dir: &Path) -> std::path::PathBuf {
    let source = dir.join("canary.c");
    let binary = dir.join("canary");
    fs::write(&source, r#"
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
#include <arpa/inet.h>
#include <netdb.h>
#include <fcntl.h>
static int connect_port(int p) {
  int s = socket(AF_INET, SOCK_STREAM, 0);
  struct sockaddr_in a = {.sin_family=AF_INET, .sin_port=htons(p)};
  inet_pton(AF_INET, "127.0.0.1", &a.sin_addr);
  int ok = connect(s,(struct sockaddr *)&a,sizeof(a)) == 0;
  close(s); return ok;
}
static int bind_local(const char *host) {
  int s = socket(AF_INET, SOCK_STREAM, 0);
  struct sockaddr_in a = {.sin_family=AF_INET, .sin_port=htons(0)};
  inet_pton(AF_INET, host, &a.sin_addr);
  int ok = bind(s,(struct sockaddr *)&a,sizeof(a)) == 0 && listen(s, 1) == 0;
  close(s); return ok;
}
int main(int argc, char **argv) {
  int r = open(argv[1], O_RDONLY); if(r >= 0) close(r);
  int w = open(argv[1], O_WRONLY | O_TRUNC); if(w >= 0) close(w);
  int a = open(argv[2], O_WRONLY | O_CREAT, 0600); if(a >= 0) close(a);
  struct addrinfo *resolved = NULL;
  int dns = getaddrinfo("localhost", NULL, NULL, &resolved) == 0;
  if (resolved) freeaddrinfo(resolved);
  printf("%d %d %d %d %d %d %d %d\n", r >= 0, w >= 0, a >= 0, connect_port(atoi(argv[3])), connect_port(atoi(argv[4])), dns, bind_local("127.0.0.1"), bind_local("0.0.0.0"));
  if (argc > 5) {
    int cr = open(argv[5], O_RDONLY); if(cr >= 0) close(cr);
    int cw = open(argv[5], O_WRONLY); if(cw >= 0) close(cw);
    printf("%d %d\n", cr >= 0, cw >= 0);
  }
  fflush(stdout);
  execl("/bin/sh", "sh", "-c", "exit 91", NULL);
  return 0;
}

"#).unwrap();
    let output = Command::new("/usr/bin/cc")
        .args([source.as_os_str(), "-o".as_ref(), binary.as_os_str()])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    binary
}

fn config(workspace: &Path, port: u16) {
    fs::create_dir(workspace.join(".agents")).unwrap();
    fs::write(workspace.join(".agents/mcp_config.json"), format!(r#"{{"mcpServers":{{"mew":{{"serverUrl":"http://127.0.0.1:{port}/0123456789abcdef0123456789abcdef"}}}}}}"#)).unwrap();
}

#[tokio::test]
async fn sandbox_allows_gemini_configuration_read_only() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    let cli_home = root.path().join("cli-home");
    let config_dir = root.path().join("gemini-config");
    fs::create_dir(&profile).unwrap();
    fs::create_dir(&cli_home).unwrap();
    fs::create_dir(&config_dir).unwrap();
    let settings = config_dir.join("settings");
    fs::write(&settings, "configuration").unwrap();
    let outside = root.path().join("outside-secret");
    fs::write(&outside, "original").unwrap();
    let binary = canary(root.path());
    let allowed = TcpListener::bind("127.0.0.1:0").unwrap();
    let blocked = TcpListener::bind("127.0.0.1:0").unwrap();
    let allowed_port = allowed.local_addr().unwrap().port();
    let workspace = tempfile::tempdir().unwrap();
    config(workspace.path(), allowed_port);
    let boundary =
        SeatbeltBoundary::for_test_with_config(profile.clone(), allowed_port, cli_home, config_dir);
    let spec = LaunchSpec {
        program: binary,
        args: vec![
            outside.into_os_string(),
            profile.join("written").into_os_string(),
            allowed_port.to_string().into(),
            blocked.local_addr().unwrap().port().to_string().into(),
            settings.as_os_str().to_owned(),
        ],
        env: vec![],
        cwd: workspace.path().to_owned(),
    };
    let mut child =
        ManagedProcess::spawn(spec, &boundary, workspace, Duration::from_secs(5)).unwrap();
    let mut output = String::new();
    child
        .take_stdout()
        .unwrap()
        .read_to_string(&mut output)
        .await
        .unwrap();
    assert!(
        matches!(child.wait().await.unwrap(), ProcessOutcome::Exited(status) if status.success())
    );
    assert_eq!(output.lines().last(), Some("1 0"));
    assert_eq!(fs::read_to_string(settings).unwrap(), "configuration");
}

#[tokio::test]
async fn sandbox_confines_filesystem_and_network_to_the_declared_endpoints() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    fs::create_dir(&profile).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&profile, fs::Permissions::from_mode(0o700)).unwrap();
    let outside = root.path().join("outside-secret");
    fs::write(&outside, "original").unwrap();
    let binary = canary(root.path());
    let allowed = TcpListener::bind("127.0.0.1:0").unwrap();
    let blocked = TcpListener::bind("127.0.0.1:0").unwrap();
    let allowed_port = allowed.local_addr().unwrap().port();
    let blocked_port = blocked.local_addr().unwrap().port();
    let workspace = tempfile::tempdir().unwrap();
    config(workspace.path(), allowed_port);
    let cli_home = root.path().join("cli-home");
    fs::create_dir(&cli_home).unwrap();
    let boundary = SeatbeltBoundary::for_test(profile.clone(), allowed_port, cli_home);
    let spec = LaunchSpec {
        program: binary,
        args: vec![
            outside.as_os_str().to_owned(),
            profile.join("written").into_os_string(),
            OsString::from(allowed_port.to_string()),
            OsString::from(blocked_port.to_string()),
        ],
        env: vec![],
        cwd: workspace.path().to_owned(),
    };
    let mut child =
        ManagedProcess::spawn(spec, &boundary, workspace, Duration::from_secs(5)).unwrap();
    let mut output = String::new();
    child
        .take_stdout()
        .unwrap()
        .read_to_string(&mut output)
        .await
        .unwrap();
    let outcome = child.wait().await.unwrap();
    assert!(
        matches!(outcome, ProcessOutcome::Exited(status) if status.success()),
        "{outcome:?}: {}",
        String::from_utf8_lossy(&child.stderr_tail())
    );
    assert_eq!(output.trim(), "0 0 1 1 0 1 0 0");
    assert_eq!(fs::read_to_string(outside).unwrap(), "original");
}

#[tokio::test]
async fn malformed_mcp_endpoint_fails_before_starting_process() {
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    fs::create_dir(&profile).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&profile, fs::Permissions::from_mode(0o700)).unwrap();
    let workspace = tempfile::tempdir().unwrap();
    config(workspace.path(), 1234);
    fs::write(
        workspace.path().join(".agents/mcp_config.json"),
        r#"{"mcpServers":{"mew":{"serverUrl":"http://192.0.2.1:1234/token"}}}"#,
    )
    .unwrap();
    let cli_home = root.path().join("cli-home");
    fs::create_dir(&cli_home).unwrap();
    let boundary = SeatbeltBoundary::for_test(profile, 1235, cli_home);
    let spec = LaunchSpec {
        program: "/usr/bin/true".into(),
        args: vec![],
        env: vec![],
        cwd: workspace.path().to_owned(),
    };
    assert!(ManagedProcess::spawn(spec, &boundary, workspace, Duration::from_secs(5)).is_err());
}

#[test]
fn rejects_invalid_proxy_port() {
    assert!(SeatbeltBoundary::new(0).is_err());
}

#[tokio::test]
async fn symlinked_mcp_configuration_is_not_read() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let profile = root.path().join("profile");
    let cli_home = root.path().join("cli-home");
    fs::create_dir(&profile).unwrap();
    fs::create_dir(&cli_home).unwrap();
    let workspace = tempfile::tempdir().unwrap();
    config(workspace.path(), 1234);
    let config = workspace.path().join(".agents/mcp_config.json");
    let replacement = root.path().join("config");
    fs::rename(&config, &replacement).unwrap();
    symlink(replacement, config).unwrap();
    let boundary = SeatbeltBoundary::for_test(profile, 1235, cli_home);
    let spec = LaunchSpec {
        program: "/usr/bin/true".into(),
        args: vec![],
        env: vec![],
        cwd: workspace.path().to_owned(),
    };
    assert!(ManagedProcess::spawn(spec, &boundary, workspace, Duration::from_secs(5)).is_err());
}

#[test]
fn rejects_paths_that_seatbelt_cannot_represent_exactly() {
    use std::os::unix::ffi::OsStrExt;
    let invalid = Path::new(std::ffi::OsStr::from_bytes(b"workspace-\xff"));
    assert!(super::macos::path_text(invalid).is_err());
}

#[test]
fn cli_home_must_exist_and_rejects_symlinks() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let root = tempfile::tempdir().unwrap();
    let gemini = root.path().join(".gemini");
    fs::create_dir(&gemini).unwrap();
    fs::set_permissions(&gemini, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(super::macos::validated_cli_home(root.path()).is_err());
    let real = root.path().join("real");
    fs::create_dir(&real).unwrap();
    symlink(&real, gemini.join("antigravity-cli")).unwrap();
    assert!(super::macos::validated_cli_home(root.path()).is_err());
    fs::remove_file(gemini.join("antigravity-cli")).unwrap();
    fs::create_dir(gemini.join("antigravity-cli")).unwrap();
    fs::set_permissions(
        gemini.join("antigravity-cli"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    assert_eq!(
        super::macos::validated_cli_home(root.path()).unwrap(),
        fs::canonicalize(gemini.join("antigravity-cli")).unwrap()
    );
}

#[test]
fn gemini_configuration_rejects_unsafe_directories() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let root = tempfile::tempdir().unwrap();
    let gemini = root.path().join(".gemini");
    fs::create_dir(&gemini).unwrap();
    assert!(super::macos::validated_gemini_config(root.path())
        .unwrap()
        .is_none());
    let real = root.path().join("real");
    fs::create_dir(&real).unwrap();
    symlink(&real, gemini.join("config")).unwrap();
    assert!(super::macos::validated_gemini_config(root.path()).is_err());
    fs::remove_file(gemini.join("config")).unwrap();
    fs::create_dir(gemini.join("config")).unwrap();
    fs::set_permissions(gemini.join("config"), fs::Permissions::from_mode(0o777)).unwrap();
    assert!(super::macos::validated_gemini_config(root.path()).is_err());
    fs::set_permissions(gemini.join("config"), fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(
        super::macos::validated_gemini_config(root.path()).unwrap(),
        Some(fs::canonicalize(gemini.join("config")).unwrap())
    );
}

#[tokio::test]
#[ignore = "requires the official agy executable on PATH"]
async fn official_cli_starts_in_local_sandbox() {
    let root = tempfile::tempdir().unwrap();
    let app_data = root.path().join("profile");
    let cli_home = root.path().join("cli-home");
    fs::create_dir(&app_data).unwrap();
    fs::create_dir(&cli_home).unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let mcp = TcpListener::bind("127.0.0.1:0").unwrap();
    config(workspace.path(), mcp.local_addr().unwrap().port());
    let proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    let binary = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("agy"))
        .find(|path| path.is_file())
        .expect("install agy on PATH before running this test");
    let boundary =
        SeatbeltBoundary::for_test(app_data, proxy.local_addr().unwrap().port(), cli_home);
    let spec = LaunchSpec {
        program: binary,
        args: vec!["--help".into()],
        env: vec![],
        cwd: workspace.path().to_owned(),
    };
    let mut child =
        ManagedProcess::spawn(spec, &boundary, workspace, Duration::from_secs(10)).unwrap();
    let mut output = String::new();
    child
        .take_stdout()
        .unwrap()
        .read_to_string(&mut output)
        .await
        .unwrap();
    let outcome = child.wait().await.unwrap();
    assert!(
        matches!(outcome, ProcessOutcome::Exited(status) if status.success()),
        "{outcome:?}: {}",
        String::from_utf8_lossy(&child.stderr_tail())
    );
    assert!(
        output.contains("--output-format")
            || String::from_utf8_lossy(&child.stderr_tail()).contains("--output-format")
    );
}
