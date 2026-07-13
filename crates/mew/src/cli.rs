//! CLI argument types — extracted from main.rs.
//!
//! All clap derive types live here so main.rs stays small.

use clap::{Parser, Subcommand};

/// The top-level CLI.
#[derive(Parser)]
#[command(name = "mew")]
#[command(about = "A terminal agent harness")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run a single prompt non-interactively
    Run {
        /// Provider ID (defaults to last-used or opencode-zen)
        #[arg(long)]
        provider: Option<String>,

        /// Model ID (overrides provider default)
        #[arg(long)]
        model: Option<String>,

        /// Thinking variant (e.g. "high", "max", "low")
        #[arg(long)]
        variant: Option<String>,

        /// Dump raw request/response to stderr
        #[arg(long)]
        raw: bool,

        /// Auto-allow Mutating tools (write/edit/etc.); bash still prompts.
        /// Equivalent to `/permissions permissive`. `-D` and `-A` win if set.
        #[arg(long, short = 'P', env = "MEW_PERMISSIVE")]
        permissive: bool,

        /// Route every tool call through a small LLM classifier
        /// (allow/deny/escalate). Equivalent to `/permissions auto`.
        /// `-D` wins if both are set. Requires a classifier provider.
        #[arg(long, short = 'A', env = "MEW_AUTO")]
        auto: bool,

        /// Like `--auto`, but the classifier CANNOT escalate — escalate or
        /// failure means Deny (fail closed). Equivalent to
        /// `/permissions auto_plus`. Wins over `-A` if both are set.
        #[arg(long, env = "MEW_AUTO_PLUS")]
        auto_plus: bool,

        /// Skip all permission prompts. Every tool auto-runs and overrides
        /// deny rules, ask rules, and the secret-file guard. Equivalent to
        /// `/permissions dangerous`. Wins over `-P`, `-A`, and
        /// `--auto-plus` if set.
        #[arg(long, short = 'D', env = "MEW_DANGEROUS")]
        dangerously_skip_permissions: bool,

        /// The prompt to send
        prompt: Vec<String>,
    },
    /// Start an interactive session
    Chat {
        /// Provider ID (defaults to last-used or opencode-zen)
        #[arg(long)]
        provider: Option<String>,

        /// Model ID (overrides provider default)
        #[arg(long)]
        model: Option<String>,

        /// Thinking variant (e.g. "high", "max", "low")
        #[arg(long)]
        variant: Option<String>,

        /// Dump raw request/response to stderr
        #[arg(long)]
        raw: bool,

        /// Auto-allow Mutating tools (write/edit/etc.); bash still prompts.
        /// Equivalent to `/permissions permissive`.
        #[arg(long, short = 'P', env = "MEW_PERMISSIVE")]
        permissive: bool,

        /// Route every tool call through a small LLM classifier
        /// (allow/deny/escalate). Equivalent to `/permissions auto`.
        #[arg(long, short = 'A', env = "MEW_AUTO")]
        auto: bool,

        /// Like `--auto`, but the classifier CANNOT escalate — escalate or
        /// failure means Deny (fail closed). Equivalent to
        /// `/permissions auto_plus`.
        #[arg(long, env = "MEW_AUTO_PLUS")]
        auto_plus: bool,

        /// Skip all permission prompts. Every tool auto-runs and overrides
        /// deny rules, ask rules, and the secret-file guard. Equivalent to
        /// `/permissions dangerous`. Can be toggled at runtime via the
        /// `/permissions` slash command.
        #[arg(long, short = 'D', env = "MEW_DANGEROUS")]
        dangerously_skip_permissions: bool,

        /// Connect to a mew daemon at the given WebSocket URL
        /// (e.g. "ws://unix:/tmp/mew.sock")
        #[arg(long)]
        connect: Option<String>,

        /// Attach to an existing session by ID instead of creating a new one.
        /// Requires --connect. Use `/resume <id>` in daemon mode to switch
        /// sessions at runtime.
        #[arg(long)]
        attach: Option<String>,
    },
    /// Run as a daemon (WebSocket server). Frontends connect to run sessions.
    Daemon {
        /// Unix socket path (default: $XDG_RUNTIME_DIR/mew.sock or /tmp/mew.sock)
        #[arg(long)]
        socket: Option<String>,

        /// TCP address to listen on, e.g. 127.0.0.1:9847. Browser-based
        /// frontends connect to this. Defaults to off — pass explicitly
        /// to enable. May be combined with --socket to listen on both.
        #[arg(long)]
        port: Option<String>,

        /// Detach from the terminal and run in the background. The daemon
        /// survives logout. Writes its PID to the pidfile (default:
        /// $XDG_RUNTIME_DIR/mew.pid). Combine with --log to redirect
        /// output to a file instead of /dev/null.
        #[arg(long)]
        background: bool,

        /// Redirect logs to this file (implies --background behavior for
        /// stdio redirection). Defaults to /dev/null when --background
        /// is set without --log.
        #[arg(long)]
        log: Option<String>,

        /// Path to write the daemon PID. Defaults to
        /// $XDG_RUNTIME_DIR/mew.pid or /tmp/mew.pid.
        #[arg(long)]
        pidfile: Option<String>,

        /// Stop a running background daemon. Reads the PID from the
        /// pidfile and sends SIGTERM. Exits 0 on success.
        #[arg(long)]
        stop: bool,

        /// Use the bundled `FakeProvider` instead of a real model.
        /// Responds to any prompt with a fixed streaming text. Intended
        /// for tests, demos, and offline experimentation — do not use
        /// in production. Overrides `--provider` and `--model`.
        #[arg(long)]
        fake_provider: bool,

        /// Provider ID (defaults to last-used or opencode-zen)
        #[arg(long)]
        provider: Option<String>,

        /// Model ID
        #[arg(long)]
        model: Option<String>,

        /// Dump raw request/response
        #[arg(long)]
        raw: bool,

        /// Auto-allow Mutating tools; bash still prompts.
        #[arg(long, short = 'P', env = "MEW_PERMISSIVE")]
        permissive: bool,

        /// Route every tool call through a small LLM classifier.
        #[arg(long, short = 'A', env = "MEW_AUTO")]
        auto: bool,

        /// Like `--auto`, but fail-closed on classifier uncertainty.
        #[arg(long, env = "MEW_AUTO_PLUS")]
        auto_plus: bool,

        /// Skip all permission prompts. Every tool auto-runs.
        #[arg(long, short = 'D', env = "MEW_DANGEROUS")]
        dangerously_skip_permissions: bool,

        /// Listen on iroh (P2P) instead of Unix socket / TCP.
        /// Used for remote/mobile access. Requires the `iroh` feature.
        #[cfg(feature = "iroh")]
        #[arg(long)]
        iroh: bool,
    },
    /// Generate a pairing QR code for mobile/remote clients.
    ///
    /// Prints the daemon's iroh NodeId and enters pairing mode. The next
    /// iroh connection's peer ID is added to the allowlist automatically.
    #[cfg(feature = "iroh")]
    Pair,
    /// Manage authentication for providers
    Auth {
        #[command(subcommand)]
        command: AuthCommands,
    },
    /// View or edit configuration
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    /// Debug tools: permission simulator, VFS inspector.
    Debug {
        #[command(subcommand)]
        command: DebugCommands,
    },
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: Option<String>,
    },
    /// Manage TUI themes
    Theme {
        #[command(subcommand)]
        command: ThemeCommands,
    },
    /// Manage extensions
    Ext {
        #[command(subcommand)]
        command: ExtCommands,
    },
    /// Capture the TUI as PNG screenshots or MP4 video using the headless
    /// harness. Runs a `.tape`-style script against a FakeProvider-backed
    /// app — no network, no credentials, fully deterministic.
    TuiCapture {
        /// Path to the script file. Uses the same verbs as the harness:
        /// `type`, `key`, `submit`, `say`, `snapshot`, `screenshot`,
        /// `start_recording`, `stop_recording`, `pause <ms>`, `record <path>`.
        #[arg(long, short = 's')]
        script: Option<std::path::PathBuf>,

        /// Interactive REPL mode: read verbs from stdin one line at a time,
        /// print the frame after each. Enables agent-driven puppet mode via
        /// bash pipes.
        #[arg(long, short = 'i')]
        interactive: bool,

        /// In interactive mode, write a PNG screenshot after each verb to
        /// this directory (numbered frame_0001.png, frame_0002.png, ...).
        /// The path to the latest screenshot is printed as a line on stdout
        /// so the agent can read it and inspect the image.
        #[arg(long)]
        screenshot_dir: Option<std::path::PathBuf>,

        /// In interactive mode, encode all captured frames to this MP4 path
        /// when the session ends (quit or EOF).
        #[arg(long)]
        mp4: Option<String>,

        /// Connect to a mew daemon for real chat/turn captures.
        #[arg(long)]
        connect: Option<String>,

        /// Framerate for the output video (default: 30).
        #[arg(long, default_value = "30")]
        fps: u32,

        /// Terminal width in columns (default: 80).
        #[arg(long, default_value = "80")]
        width: u16,

        /// Terminal height in rows (default: 24).
        #[arg(long, default_value = "24")]
        height: u16,
    },
}

#[derive(Subcommand)]
pub enum ExtCommands {
    /// List installed extensions (packages + bare plugins)
    List,
    /// Enable a disabled extension
    Enable { name: String },
    /// Disable an extension (stops it from loading)
    Disable { name: String },
    /// Remove an extension package
    Remove { name: String },
    /// Diagnose extension discovery, conflicts, and health
    Doctor,
    /// Install an extension from a git URL or local path
    Install {
        /// Git URL (https://...) or local directory path
        source: String,
        /// Override the install name (defaults to repo name or dir name)
        #[arg(long)]
        name: Option<String>,
        /// Overwrite if already installed
        #[arg(long)]
        force: bool,
        /// Show what would be installed without copying any files
        #[arg(long)]
        dry_run: bool,
    },
    /// Revoke an extension's attach token
    Revoke { name: String },
    /// Re-mint all extension attach tokens
    RotateAll,
    /// Show the attach token for an extension
    Token { name: String },
}

#[derive(Subcommand)]
pub enum ThemeCommands {
    /// List available themes
    List,
    /// Print the currently active theme name
    Current,
    /// Install a theme JSON file to ~/.config/mew/themes/
    Install {
        /// Path to the JSON theme file to install
        path: std::path::PathBuf,
    },
}

#[derive(Subcommand)]
pub enum ConfigCommands {
    /// Print the current configuration to stdout
    Show,
    /// Open the config file in $EDITOR (or $VISUAL, or vi)
    Edit,
    /// Interactive TUI config editor
    Editor,
    /// Print the path to the config file
    Path,
}

#[derive(Subcommand)]
pub enum DebugCommands {
    /// Simulate a permission check for a tool call. Shows the decision the
    /// engine would make without running the agent.
    Permissions {
        /// Tool name (e.g. "bash", "read", "write").
        tool: String,
        /// Tool input as JSON (e.g. '{"command": "rm -rf /"}').
        /// Defaults to empty object `{}`.
        input: Option<String>,
        /// Sensitivity tier: readonly, mutating, or dangerous.
        /// Defaults to "dangerous" (the strictest — worst-case check).
        #[arg(long, default_value = "dangerous")]
        sensitivity: String,
    },
    /// Inspect built-in resources via the mew:// virtual filesystem.
    Vfs {
        #[command(subcommand)]
        command: VfsCommands,
    },
    /// Inspect or clear the on-disk model catalog cache.
    Cache {
        #[command(subcommand)]
        command: CacheCommands,
    },
}

#[derive(Subcommand)]
pub enum CacheCommands {
    /// Print the directory that holds cached catalog files.
    Path,
    /// Remove the cached catalog files (main + umans). The next launch will
    /// re-fetch from the network. Use this when a provider added or removed
    /// models and the picker hasn't picked it up yet.
    Clear,
}

#[derive(Subcommand)]
pub enum VfsCommands {
    /// List resources at a path (or top-level if no path given).
    Ls {
        /// Path relative to the VFS root (e.g. "personas", "subagents").
        /// Omit to list top-level directories.
        path: Option<String>,
    },
    /// Print the contents of a resource.
    Cat {
        /// Path relative to the VFS root (e.g. "personas/researcher").
        path: String,
    },
}

#[derive(Subcommand)]
pub enum AuthCommands {
    /// Log in to OpenAI via ChatGPT OAuth (browser-based PKCE flow).
    /// Uses your ChatGPT Plus/Pro subscription credits.
    Login {
        /// Provider to log in to (currently only "codex").
        provider: Option<String>,
        /// Use the device-code flow instead of opening a browser. For
        /// headless machines or when a browser can't be launched.
        #[arg(long)]
        headless: bool,
    },
    /// Show current auth status for all providers.
    Status,
    /// Log out and delete stored credentials.
    Logout {
        /// Provider to log out from.
        provider: Option<String>,
    },
}
