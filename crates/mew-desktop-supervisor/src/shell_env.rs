//! Capture the user's interactive login shell environment when the desktop host
//! spawns an owned daemon without an inherited shell environment (e.g. Finder launch).

use std::collections::{HashMap, HashSet};
#[cfg(unix)]
use std::io::Read;
use std::path::Path;
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::time::{Duration, Instant};

const ENV_START_MARKER: &[u8] = b"__MEW_LOGIN_ENV_START__\0";
const ENV_END_MARKER: &[u8] = b"__MEW_LOGIN_ENV_END__\0";
#[cfg(unix)]
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);
#[cfg(unix)]
const POLL_INTERVAL: Duration = Duration::from_millis(20);
#[cfg(unix)]
const SKIP_ENV_VAR: &str = "MEW_SKIP_LOGIN_SHELL_ENV";
#[cfg(unix)]
const ENV_PROBE_SCRIPT: &str =
    "printf '__MEW_LOGIN_ENV_START__\\0'; env -0; printf '__MEW_LOGIN_ENV_END__\\0'";

/// Resolve an environment map combining the current process environment with the
/// user's login shell environment. Returns `None` on non-Unix, when disabled via
/// `MEW_SKIP_LOGIN_SHELL_ENV`, or if probing the shell fails/times out.
pub(crate) fn resolve_daemon_env() -> Option<HashMap<String, String>> {
    #[cfg(not(unix))]
    {
        None
    }

    #[cfg(unix)]
    {
        if skip_login_shell_probe() {
            return None;
        }
        let shell = resolve_user_shell();
        let shell_env = capture_login_shell_env(&shell, PROBE_TIMEOUT)?;
        Some(merge_login_env(std::env::vars().collect(), shell_env))
    }
}

#[cfg(unix)]
fn skip_login_shell_probe() -> bool {
    std::env::var(SKIP_ENV_VAR).is_ok_and(|value| {
        let trimmed = value.trim();
        trimmed == "1"
            || trimmed.eq_ignore_ascii_case("true")
            || trimmed.eq_ignore_ascii_case("yes")
    })
}

#[cfg(unix)]
fn resolve_user_shell() -> String {
    match std::env::var("SHELL") {
        Ok(shell) if !shell.trim().is_empty() => shell,
        _ => {
            #[cfg(target_os = "macos")]
            {
                "/bin/zsh".to_string()
            }
            #[cfg(not(target_os = "macos"))]
            {
                "/bin/sh".to_string()
            }
        }
    }
}

fn shell_probe_args(shell: &str) -> Option<Vec<&'static str>> {
    #[cfg(unix)]
    let script = ENV_PROBE_SCRIPT;
    #[cfg(not(unix))]
    let script = "printf '__MEW_LOGIN_ENV_START__\\0'; env -0; printf '__MEW_LOGIN_ENV_END__\\0'";

    let name = Path::new(shell).file_name()?.to_str()?;
    match name {
        "zsh" | "bash" | "fish" | "ksh" | "mksh" => Some(vec!["-l", "-i", "-c", script]),
        "sh" => Some(vec!["-l", "-c", script]),
        _ => None,
    }
}

#[cfg(unix)]
fn capture_login_shell_env(shell: &str, timeout: Duration) -> Option<HashMap<String, String>> {
    let args = shell_probe_args(shell)?;
    let mut child = Command::new(shell)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(POLL_INTERVAL);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };

    let bytes = reader.join().ok()?;
    let status = status?;
    if !status.success() {
        return None;
    }

    parse_marked_env(&bytes)
}

fn parse_marked_env(bytes: &[u8]) -> Option<HashMap<String, String>> {
    let start_pos = bytes
        .windows(ENV_START_MARKER.len())
        .rposition(|window| window == ENV_START_MARKER)?;
    let payload_start = start_pos + ENV_START_MARKER.len();
    let rest = &bytes[payload_start..];
    let end_rel = rest
        .windows(ENV_END_MARKER.len())
        .position(|window| window == ENV_END_MARKER)?;
    let payload = &rest[..end_rel];

    let mut map = HashMap::new();
    for entry in payload.split(|byte| *byte == b'\0') {
        if entry.is_empty() {
            continue;
        }
        let Some(eq_pos) = entry.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let Ok(key) = std::str::from_utf8(&entry[..eq_pos]) else {
            continue;
        };
        if key.is_empty() {
            continue;
        }
        let value = String::from_utf8_lossy(&entry[eq_pos + 1..]).into_owned();
        map.insert(key.to_string(), value);
    }

    (!map.is_empty()).then_some(map)
}

fn is_ephemeral_shell_var(key: &str) -> bool {
    matches!(key, "PWD" | "OLDPWD" | "SHLVL" | "_" | "TERM")
}

fn merge_login_env(
    current: HashMap<String, String>,
    shell: HashMap<String, String>,
) -> HashMap<String, String> {
    let mut merged = HashMap::new();
    for (key, value) in &shell {
        if !is_ephemeral_shell_var(key) {
            merged.insert(key.clone(), value.clone());
        }
    }

    if let Some(merged_path) = merge_path(
        shell.get("PATH").map(String::as_str),
        current.get("PATH").map(String::as_str),
    ) {
        merged.insert("PATH".to_string(), merged_path);
    }

    for (key, value) in current {
        if key == "PATH" {
            continue;
        }
        if key.starts_with("MEW_") || !merged.contains_key(&key) {
            merged.insert(key, value);
        }
    }

    merged
}

fn merge_path(shell_path: Option<&str>, current_path: Option<&str>) -> Option<String> {
    let mut seen = HashSet::new();
    let mut parts = Vec::new();
    for raw in [shell_path, current_path].into_iter().flatten() {
        for segment in raw.split(':').filter(|segment| !segment.is_empty()) {
            if seen.insert(segment) {
                parts.push(segment);
            }
        }
    }
    (!parts.is_empty()).then(|| parts.join(":"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_marked_env_ignores_noisy_prefix_and_suffix_and_handles_newlines_in_values() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"Welcome to zsh!\n__MEW_LOGIN_ENV_START__\0stale=1\0");
        bytes.extend_from_slice(ENV_START_MARKER);
        bytes.extend_from_slice(b"HOME=/Users/natalie\0MULTI=line1\nline2\0EMPTY=\0");
        bytes.extend_from_slice(ENV_END_MARKER);
        bytes.extend_from_slice(b"goodbye from trap\n");

        let parsed = parse_marked_env(&bytes).expect("marked env should parse");
        assert_eq!(
            parsed.get("HOME").map(String::as_str),
            Some("/Users/natalie")
        );
        assert_eq!(
            parsed.get("MULTI").map(String::as_str),
            Some("line1\nline2")
        );
        assert_eq!(parsed.get("EMPTY").map(String::as_str), Some(""));
        assert!(!parsed.contains_key("stale"));
    }

    #[test]
    fn parse_marked_env_returns_none_when_markers_missing() {
        assert!(parse_marked_env(b"HOME=/Users/natalie\0").is_none());
        assert!(parse_marked_env(b"__MEW_LOGIN_ENV_START__\0HOME=/Users/natalie\0").is_none());
        assert!(parse_marked_env(b"__MEW_LOGIN_ENV_START__\0__MEW_LOGIN_ENV_END__\0").is_none());
    }

    #[test]
    fn merge_login_env_preserves_mew_overrides_merges_path_and_drops_ephemeral_vars() {
        let current = HashMap::from([
            ("PATH".to_string(), "/usr/bin:/custom/bin:/bin".to_string()),
            ("MEW_CONFIG_DIR".to_string(), "/tmp/test-config".to_string()),
            (
                "MEW_SESSION_DIR".to_string(),
                "/tmp/test-sessions".to_string(),
            ),
            ("TMPDIR".to_string(), "/var/folders/tmp".to_string()),
            ("TERM".to_string(), "xterm-256color".to_string()),
            ("SHLVL".to_string(), "1".to_string()),
        ]);
        let shell = HashMap::from([
            (
                "PATH".to_string(),
                "/Users/natalie/.local/share/mise/shims:/opt/homebrew/bin:/usr/bin".to_string(),
            ),
            (
                "MEW_CONFIG_DIR".to_string(),
                "/Users/natalie/.config/mew".to_string(),
            ),
            ("MISE_SHELL".to_string(), "zsh".to_string()),
            ("PWD".to_string(), "/Users/natalie".to_string()),
            ("OLDPWD".to_string(), "/".to_string()),
            ("SHLVL".to_string(), "9".to_string()),
            ("_".to_string(), "/usr/bin/env".to_string()),
            ("TERM".to_string(), "dumb".to_string()),
        ]);

        let merged = merge_login_env(current, shell);
        assert_eq!(
            merged.get("PATH").map(String::as_str),
            Some("/Users/natalie/.local/share/mise/shims:/opt/homebrew/bin:/usr/bin:/custom/bin:/bin")
        );
        assert_eq!(
            merged.get("MEW_CONFIG_DIR").map(String::as_str),
            Some("/tmp/test-config")
        );
        assert_eq!(
            merged.get("MEW_SESSION_DIR").map(String::as_str),
            Some("/tmp/test-sessions")
        );
        assert_eq!(merged.get("MISE_SHELL").map(String::as_str), Some("zsh"));
        assert_eq!(
            merged.get("TMPDIR").map(String::as_str),
            Some("/var/folders/tmp")
        );
        assert_eq!(
            merged.get("TERM").map(String::as_str),
            Some("xterm-256color")
        );
        assert_eq!(merged.get("SHLVL").map(String::as_str), Some("1"));
        assert!(!merged.contains_key("PWD"));
        assert!(!merged.contains_key("OLDPWD"));
        assert!(!merged.contains_key("_"));
    }

    #[test]
    fn shell_probe_args_selects_login_interactive_for_known_shells_and_skips_unknown() {
        assert_eq!(
            shell_probe_args("/bin/zsh").as_deref(),
            Some(["-l", "-i", "-c", ENV_PROBE_SCRIPT].as_slice())
        );
        assert_eq!(
            shell_probe_args("/opt/homebrew/bin/fish").as_deref(),
            Some(["-l", "-i", "-c", ENV_PROBE_SCRIPT].as_slice())
        );
        assert_eq!(
            shell_probe_args("/bin/sh").as_deref(),
            Some(["-l", "-c", ENV_PROBE_SCRIPT].as_slice())
        );
        assert!(shell_probe_args("/usr/local/bin/nu").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn capture_login_shell_env_times_out_cleanly_on_hung_shell() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("sh");
        std::fs::write(&script_path, "#!/bin/sh\nexec sleep 10\n").unwrap();
        let mut perms = std::fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&script_path, perms).unwrap();

        let started = Instant::now();
        let captured =
            capture_login_shell_env(script_path.to_str().unwrap(), Duration::from_millis(120));
        assert!(captured.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "timeout took too long: {:?}",
            started.elapsed()
        );
    }
}
