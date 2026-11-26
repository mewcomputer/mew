//! Local macOS sandbox for an AGY process using a restricted loopback proxy.

#[cfg(target_os = "macos")]
mod macos {
    use crate::process::{IsolationBoundary, LaunchSpec};
    use std::{
        ffi::OsString,
        fs::{self, OpenOptions},
        io::{self, Read},
        os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        path::{Path, PathBuf},
    };

    pub struct SeatbeltBoundary {
        app_data: PathBuf,
        proxy_port: u16,
        cli_home: PathBuf,
        gemini_config: Option<PathBuf>,
    }

    impl SeatbeltBoundary {
        pub fn new(proxy_port: u16) -> io::Result<Self> {
            if proxy_port == 0 {
                return Err(invalid("invalid proxy port"));
            }
            let home = std::env::var_os("HOME").ok_or_else(|| invalid("HOME is unavailable"))?;
            let home = fs::canonicalize(home)?;
            let cli_home = validated_cli_home(&home)?;
            let gemini_config = validated_gemini_config(&home)?;
            Ok(Self {
                app_data: cli_home.clone(),
                proxy_port,
                cli_home,
                gemini_config,
            })
        }

        #[cfg(test)]
        pub(super) fn for_test(app_data: PathBuf, proxy_port: u16, cli_home: PathBuf) -> Self {
            Self {
                app_data: fs::canonicalize(app_data).unwrap(),
                proxy_port,
                cli_home: fs::canonicalize(cli_home).unwrap(),
                gemini_config: None,
            }
        }

        #[cfg(test)]
        pub(super) fn for_test_with_config(
            app_data: PathBuf,
            proxy_port: u16,
            cli_home: PathBuf,
            gemini_config: PathBuf,
        ) -> Self {
            let mut boundary = Self::for_test(app_data, proxy_port, cli_home);
            boundary.gemini_config = Some(fs::canonicalize(gemini_config).unwrap());
            boundary
        }
    }

    impl IsolationBoundary for SeatbeltBoundary {
        fn wrap(&self, spec: LaunchSpec, workspace: &Path) -> io::Result<LaunchSpec> {
            let workspace = fs::canonicalize(workspace)?;
            if fs::canonicalize(&spec.cwd)? != workspace
                || workspace.starts_with(&self.app_data)
                || self.app_data.starts_with(&workspace)
            {
                return Err(invalid("invalid isolated workspace"));
            }
            let program = fs::canonicalize(&spec.program)?;
            for path in [&program, &workspace, &self.app_data, &self.cli_home] {
                path_text(path)?;
            }
            if let Some(config) = &self.gemini_config {
                path_text(config)?;
            }
            if !fs::metadata(&program)?.is_file()
                || program.starts_with(&workspace)
                || program.starts_with(&self.app_data)
            {
                return Err(invalid("invalid AGY executable"));
            }
            let agents = workspace.join(".agents");
            if fs::symlink_metadata(&agents)?.file_type().is_symlink() {
                return Err(invalid("invalid agent directory"));
            }
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(agents.join("mcp_config.json"))?;
            if !file.metadata()?.is_file() || file.metadata()?.len() > 65_536 {
                return Err(invalid("invalid MCP configuration"));
            }
            let mut text = String::new();
            file.take(65_537).read_to_string(&mut text)?;
            let config: serde_json::Value =
                serde_json::from_str(&text).map_err(|_| invalid("invalid MCP configuration"))?;
            let url = config["mcpServers"]["mew"]["serverUrl"]
                .as_str()
                .or_else(|| config["mcpServers"]["mew"]["url"].as_str())
                .ok_or_else(|| invalid("invalid MCP configuration"))?;
            let url = url::Url::parse(url).map_err(|_| invalid("invalid MCP URL"))?;
            let token = url.path().strip_prefix('/').unwrap_or_default();
            if url.scheme() != "http"
                || url.host_str() != Some("127.0.0.1")
                || url.port().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.query().is_some()
                || url.fragment().is_some()
                || !(16..=128).contains(&token.len())
                || !token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            {
                return Err(invalid("invalid MCP URL"));
            }
            let mcp_port = url.port().unwrap();
            if mcp_port == 0 {
                return Err(invalid("invalid MCP port"));
            }
            let mut ancestors = std::collections::BTreeSet::new();
            for path in [&program, &workspace, &self.app_data, &self.cli_home] {
                for ancestor in path.ancestors().skip(1) {
                    ancestors.insert(ancestor.to_path_buf());
                }
            }
            if let Some(config) = &self.gemini_config {
                for ancestor in config.ancestors().skip(1) {
                    ancestors.insert(ancestor.to_path_buf());
                }
            }
            let metadata = ancestors
                .iter()
                .map(|path| {
                    format!(
                        "(literal {})",
                        serde_json::to_string(&path.to_string_lossy()).expect("path serialization")
                    )
                })
                .collect::<Vec<_>>()
                .join(" ");
            let config_rule = self
                .gemini_config
                .as_ref()
                .map(|path| {
                    format!(
                        "(allow file-read* (subpath {}))",
                        serde_json::to_string(path.to_str().expect("validated path"))
                            .expect("path serialization")
                    )
                })
                .unwrap_or_default();
            let profile = format!(
                r#"(version 1)
(deny default)
(import "system.sb")
(allow process-exec (literal (param "PROGRAM")))
(allow file-read* (literal (param "PROGRAM")) (subpath "/System") (subpath "/usr/lib") (subpath "/usr/share/zoneinfo") (subpath "/System/Volumes/Preboot/Cryptexes/OS/usr/lib") (literal "/dev/null") (literal "/dev/urandom"))
(allow file-read* (literal "/private/etc/hosts"))
{config_rule}
(allow file-read* file-write* (subpath (param "WORKSPACE")) (subpath (param "APP_DATA")) (subpath (param "CLI_HOME")))
(allow file-read-metadata {metadata})
(allow file-read-data file-write-data (literal "/dev/null"))
(allow sysctl-read)
(allow mach-lookup (global-name "com.apple.system.logger"))
(allow network-outbound (remote ip "localhost:{mcp_port}") (remote ip "localhost:{}"))
"#,
                self.proxy_port
            );
            let home = std::env::var_os("HOME").ok_or_else(|| invalid("HOME is unavailable"))?;
            let proxy = format!("http://127.0.0.1:{}", self.proxy_port);
            let mut env: Vec<(OsString, OsString)> = vec![
                ("HOME".into(), home),
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("TMPDIR".into(), workspace.as_os_str().to_owned()),
                ("HTTPS_PROXY".into(), proxy.clone().into()),
                ("HTTP_PROXY".into(), proxy.into()),
                ("NO_PROXY".into(), "127.0.0.1,localhost".into()),
            ];
            for name in [
                "ANTIGRAVITY_APP_DATA_DIR",
                "XDG_CONFIG_HOME",
                "XDG_CACHE_HOME",
                "XDG_DATA_HOME",
            ] {
                if let Some(value) = std::env::var_os(name) {
                    env.push((name.into(), value));
                }
            }
            let mut args: Vec<OsString> = vec![
                "-p".into(),
                profile.into(),
                "-D".into(),
                format!("PROGRAM={}", program.display()).into(),
                "-D".into(),
                format!("WORKSPACE={}", workspace.display()).into(),
                "-D".into(),
                format!("APP_DATA={}", self.app_data.display()).into(),
                "-D".into(),
                format!("CLI_HOME={}", self.cli_home.display()).into(),
                program.into_os_string(),
            ];
            args.extend(spec.args);
            Ok(LaunchSpec {
                program: "/usr/bin/sandbox-exec".into(),
                args,
                env,
                cwd: workspace,
            })
        }
    }
    pub(super) fn validated_cli_home(home: &Path) -> io::Result<PathBuf> {
        let gemini = home.join(".gemini");
        let cli_home = gemini.join("antigravity-cli");
        for path in [&gemini, &cli_home] {
            let metadata = fs::symlink_metadata(path).map_err(|_| {
                invalid(
                    "AGY CLI settings directory is missing; run the official AGY CLI setup first",
                )
            })?;
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.permissions().mode() & 0o022 != 0
            {
                return Err(invalid("AGY CLI settings directory must be owned by this user and not be a symlink or writable by others"));
            }
        }
        fs::canonicalize(cli_home)
    }
    pub(super) fn validated_gemini_config(home: &Path) -> io::Result<Option<PathBuf>> {
        let path = home.join(".gemini/config");
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.permissions().mode() & 0o022 != 0
        {
            return Err(invalid("Gemini configuration must be user-owned and not be a symlink or writable by others"));
        }
        fs::canonicalize(path).map(Some)
    }
    pub(super) fn path_text(path: &Path) -> io::Result<&str> {
        path.to_str()
            .ok_or_else(|| invalid("sandbox path is not UTF-8"))
    }
    fn invalid(message: &'static str) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidInput, message)
    }
}

#[cfg(target_os = "macos")]
pub use macos::SeatbeltBoundary;

#[cfg(all(test, target_os = "macos"))]
#[path = "sandbox_tests.rs"]
mod tests;
