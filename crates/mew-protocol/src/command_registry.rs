//! Shared command registry — single source of truth for built-in slash commands.
//!
//! Both the TUI and the daemon read from this table. The TUI derives its
//! autocomplete list; the daemon uses it to reject unknown commands with a
//! visible error instead of silently ignoring them.

/// Where a command is handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandLocus {
    /// Handled client-side (in the TUI's `handle_slash` / `handle_action`).
    Client,
    /// Forwarded to the daemon for execution.
    Daemon,
    /// Handled on both sides depending on mode.
    Either,
}

/// A built-in slash command definition.
#[derive(Debug, Clone)]
pub struct CommandDef {
    /// The command name including leading `/` (e.g. `/clear`).
    pub name: &'static str,
    /// Short description shown in autocomplete.
    pub description: &'static str,
    /// Where this command is handled.
    pub locus: CommandLocus,
}

/// The full table of built-in slash commands.
pub static BUILTIN_COMMANDS: &[CommandDef] = &[
    CommandDef {
        name: "/clear",
        description: "clear chat",
        locus: CommandLocus::Either,
    },
    CommandDef {
        name: "/compact",
        description: "force context compaction",
        locus: CommandLocus::Either,
    },
    CommandDef {
        name: "/todo",
        description: "show the session todo list",
        locus: CommandLocus::Either,
    },
    CommandDef {
        name: "/cost",
        description: "show cost breakdown",
        locus: CommandLocus::Client,
    },
    CommandDef {
        name: "/help",
        description: "show available commands",
        locus: CommandLocus::Client,
    },
    CommandDef {
        name: "/model",
        description: "switch model (e.g. /model deepseek-v4-flash)",
        locus: CommandLocus::Client,
    },
    CommandDef {
        name: "/thinking",
        description: "set thinking variant (e.g. /thinking high)",
        locus: CommandLocus::Either,
    },
    CommandDef {
        name: "/persona",
        description: "switch persona (e.g. /persona researcher)",
        locus: CommandLocus::Either,
    },
    CommandDef {
        name: "/permissions",
        description: "switch permission mode (standard/permissive/auto/auto_plus/dangerous)",
        locus: CommandLocus::Either,
    },
    CommandDef {
        name: "/quit",
        description: "exit mew",
        locus: CommandLocus::Either,
    },
    CommandDef {
        name: "/sessions",
        description: "list previous sessions",
        locus: CommandLocus::Client,
    },
    CommandDef {
        name: "/resume",
        description: "resume a session (e.g. /resume <id>)",
        locus: CommandLocus::Client,
    },
    CommandDef {
        name: "/rewind",
        description: "rewind to an earlier point (e.g. /rewind 3)",
        locus: CommandLocus::Client,
    },
    CommandDef {
        name: "/mouse",
        description: "toggle mouse capture for text selection",
        locus: CommandLocus::Client,
    },
    CommandDef {
        name: "/theme",
        description: "switch theme (e.g. /theme light, /theme dark)",
        locus: CommandLocus::Client,
    },
    CommandDef {
        name: "/web",
        description: "show the web UI URL for the current session",
        locus: CommandLocus::Client,
    },
    CommandDef {
        name: "/yield",
        description: "yield control to other clients",
        locus: CommandLocus::Daemon,
    },
    CommandDef {
        name: "/autotitle",
        description: "toggle auto session titling (daemon mode)",
        locus: CommandLocus::Daemon,
    },
    CommandDef {
        name: "/autosummary",
        description: "toggle auto session summaries (daemon mode)",
        locus: CommandLocus::Daemon,
    },
    CommandDef {
        name: "/wiki",
        description: "search the wiki",
        locus: CommandLocus::Daemon,
    },
];

/// Look up a command by its name (including the leading `/`).
pub fn lookup(name: &str) -> Option<&'static CommandDef> {
    BUILTIN_COMMANDS.iter().find(|c| c.name == name)
}

/// Check if a command name is known to the registry.
pub fn is_known(name: &str) -> bool {
    lookup(name).is_some()
}
