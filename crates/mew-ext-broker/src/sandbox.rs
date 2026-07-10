//! macOS Seatbelt sandbox profile generation for extension processes.
//!
//! On macOS, extensions are sandboxed via `sandbox-exec` (available on all
//! macOS installs). The profile is a Seatbelt S-expression passed inline
//! via `sandbox-exec -p <profile> -D KEY=VALUE ... <command>`.
//! On other platforms, sandbox enforcement is a no-op with a warning.

use std::path::Path;

use crate::manifest::ExtensionSandbox;

/// Sandbox profile text + parameter bindings for `sandbox-exec`.
/// Passed to `SpawnSpec` as a plain `(String, Vec<(String, String)>)` so
/// `mew-hooks-runtime` (which owns `SpawnSpec`) doesn't depend on
/// `mew-ext-broker`.
pub struct SandboxConfig {
    /// The complete Seatbelt profile text (passed to `sandbox-exec -p`).
    pub profile_text: String,
    /// Parameter bindings (passed to `sandbox-exec -D KEY=VALUE`).
    pub params: Vec<(String, String)>,
}

/// Build a sandbox profile for an extension.
///
/// Default-deny: the process can read/write its package dir and
/// storage dir, plus any explicitly widened paths. Network is denied
/// unless `sandbox.net = true`.
///
/// Seatbelt syntax note: the action (allow/deny) must WRAP the predicate.
/// Correct: `(allow file-read* file-write* (subpath (param "PACKAGE_DIR")))`
/// Wrong: `(subpath (param "PACKAGE_DIR")) (allow file-read* file-write*)`
pub fn build_sandbox_profile(
    package_dir: &Path,
    storage_dir: &Path,
    sandbox: &ExtensionSandbox,
) -> SandboxConfig {
    let mut rules = Vec::new();

    // Package dir: read/write.
    rules.push(
        "(allow file-read* file-write* file-read-metadata (subpath (param \"PACKAGE_DIR\")))"
            .to_string(),
    );
    // Storage dir: read/write.
    rules.push(
        "(allow file-read* file-write* file-read-metadata (subpath (param \"STORAGE_DIR\")))"
            .to_string(),
    );

    // Widened read paths.
    for path in &sandbox.fs_read {
        rules.push(format!(
            "(allow file-read* file-read-metadata (literal \"{}\"))",
            escape_path(path)
        ));
    }
    // Widened write paths.
    for path in &sandbox.fs_write {
        rules.push(format!(
            "(allow file-read* file-write* file-read-metadata (literal \"{}\"))",
            escape_path(path)
        ));
    }

    // Network: denied by default, allowed only if sandbox.net = true.
    if !sandbox.net {
        rules.push("(deny network*)".to_string());
    }

    // Required system services for a functioning process.
    rules.push("(allow sysctl-read)".to_string());
    rules.push("(allow mach-lookup)".to_string());
    rules.push("(allow process-info-pidinfo)".to_string());
    rules.push("(allow signal (target self))".to_string());
    // Allow process execution (needed for the extension itself to run).
    rules.push("(allow process-exec process-fork)".to_string());
    // Allow read/write on inherited pipe FDs (stdin/stdout/stderr).
    // file-read-data/file-write-data cover the read()/write() syscalls
    // on already-open FDs WITHOUT allowing open() on arbitrary paths.
    // Path-based file-read*/file-write* rules above still control which
    // paths can be opened. This is the key distinction: data I/O vs open.
    rules.push("(allow file-read-data file-write-data)".to_string());
    // Allow /dev access (e.g., /dev/null for stdin redirection).
    rules.push(r#"(allow file-read* file-write* (subpath "/dev"))"#.to_string());

    let profile_text = format!("(version 1)\n(deny default)\n{}\n", rules.join("\n"));

    // Guard against ARG_MAX overflow: sandbox-exec -p passes the profile
    // text as a CLI argument. If it's too long, the spawn will fail with
    // E2BIG. 100KB is a conservative limit (macOS ARG_MAX is ~1MB total).
    const MAX_PROFILE_LEN: usize = 100_000;
    if profile_text.len() > MAX_PROFILE_LEN {
        tracing::warn!(
            "sandbox profile is {} bytes (limit {}); extension may fail to spawn",
            profile_text.len(),
            MAX_PROFILE_LEN
        );
    }

    SandboxConfig {
        profile_text,
        params: vec![
            (
                "PACKAGE_DIR".to_string(),
                package_dir.to_string_lossy().into(),
            ),
            (
                "STORAGE_DIR".to_string(),
                storage_dir.to_string_lossy().into(),
            ),
        ],
    }
}

/// Whether OS sandbox enforcement is available on this platform.
pub fn sandbox_available() -> bool {
    cfg!(target_os = "macos")
}

/// Escape a path for use in a Seatbelt `(literal "...")` expression.
/// Escapes backslash, double-quote, and newline to prevent profile injection.
/// Without newline escaping, a crafted path containing `\n(allow file-read*)`
/// could inject arbitrary rules into the sandbox profile.
fn escape_path(p: &str) -> String {
    p.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_sandbox_profile_default() {
        let pkg = std::path::Path::new("/tmp/pkg");
        let storage = std::path::Path::new("/tmp/storage");
        let sandbox = ExtensionSandbox::default();

        let cfg = build_sandbox_profile(pkg, storage, &sandbox);

        // Profile denies network by default.
        assert!(cfg.profile_text.contains("(deny network*)"));
        // Profile allows package dir.
        assert!(cfg.profile_text.contains("PACKAGE_DIR"));
        assert!(cfg.profile_text.contains("STORAGE_DIR"));
        // Params contain the paths.
        assert_eq!(cfg.params.len(), 2);
        assert_eq!(cfg.params[0].0, "PACKAGE_DIR");
        assert_eq!(cfg.params[1].0, "STORAGE_DIR");
    }

    #[test]
    fn test_sandbox_profile_no_blanket_file_access() {
        // The profile must NOT contain a blanket file-read*/file-write* rule
        // (which would allow opening arbitrary files). file-read-data and
        // file-write-data without a path filter are OK — they allow
        // read()/write() syscalls on already-open FDs (pipes) but do NOT
        // allow open() on new paths.
        let pkg = std::path::Path::new("/tmp/pkg");
        let storage = std::path::Path::new("/tmp/storage");
        let sandbox = ExtensionSandbox::default();

        let cfg = build_sandbox_profile(pkg, storage, &sandbox);

        for line in cfg.profile_text.lines() {
            // file-read* and file-write* (with glob) must have a path filter.
            // file-read-data and file-write-data (specific syscalls) are OK
            // without a path filter — they operate on open FDs only.
            if (line.contains("file-read*") || line.contains("file-write*"))
                && !line.contains("file-read-data")
                && !line.contains("file-write-data")
            {
                assert!(
                    line.contains("subpath") || line.contains("literal"),
                    "blanket file-access rule found (no path filter): {}",
                    line
                );
            }
        }
    }

    #[test]
    fn test_build_sandbox_profile_with_net() {
        let pkg = std::path::Path::new("/tmp/pkg");
        let storage = std::path::Path::new("/tmp/storage");
        let sandbox = ExtensionSandbox {
            net: true,
            ..Default::default()
        };

        let cfg = build_sandbox_profile(pkg, storage, &sandbox);

        // No network deny rule when net = true.
        assert!(!cfg.profile_text.contains("(deny network*)"));
    }

    #[test]
    fn test_build_sandbox_profile_with_fs_widenings() {
        let pkg = std::path::Path::new("/tmp/pkg");
        let storage = std::path::Path::new("/tmp/storage");
        let sandbox = ExtensionSandbox {
            fs_read: vec!["/etc/hosts".into()],
            fs_write: vec!["/tmp/output".into()],
            ..Default::default()
        };

        let cfg = build_sandbox_profile(pkg, storage, &sandbox);

        // Profile has literal allow rules for widened paths.
        assert!(cfg.profile_text.contains(r#"(literal "/etc/hosts")"#));
        assert!(cfg.profile_text.contains(r#"(literal "/tmp/output")"#));
    }

    #[test]
    fn test_escape_path() {
        assert_eq!(escape_path("/simple/path"), "/simple/path");
        assert_eq!(escape_path(r#"C:\Users"#), r#"C:\\Users"#);
        assert_eq!(escape_path(r#"/path/"inject""#), r#"/path/\"inject\""#);
        // Newline injection prevention.
        assert_eq!(
            escape_path("/ok\n(allow file-read*)"),
            r#"/ok\n(allow file-read*)"#
        );
        assert_eq!(escape_path("/tab\there"), r#"/tab\there"#);
    }
}
