//! Slash command handling for the App state.
//!
//! Built-in slash commands, dynamic plugin commands, command
//! autocomplete, and the main handle_slash dispatcher.

use super::*;

impl App {
    pub fn builtin_slash_commands() -> Vec<SlashCommand> {
        vec![
            SlashCommand {
                name: "/clear".into(),
                description: "clear chat".into(),
            },
            SlashCommand {
                name: "/compact".into(),
                description: "force context compaction".into(),
            },
            SlashCommand {
                name: "/todo".into(),
                description: "show the session todo list".into(),
            },
            SlashCommand {
                name: "/cost".into(),
                description: "show cost breakdown".into(),
            },
            SlashCommand {
                name: "/help".into(),
                description: "show available commands".into(),
            },
            SlashCommand {
                name: "/model".into(),
                description: "switch model (e.g. /model deepseek-v4-flash)".into(),
            },
            SlashCommand {
                name: "/models".into(),
                description: "alias for /model".into(),
            },
            SlashCommand {
                name: "/thinking".into(),
                description: "set thinking variant (e.g. /thinking high)".into(),
            },
            SlashCommand {
                name: "/persona".into(),
                description: "switch persona (e.g. /persona researcher)".into(),
            },
            SlashCommand {
                name: "/permissions".into(),
                description:
                    "switch permission mode (standard/permissive/auto/auto_plus/dangerous)".into(),
            },
            SlashCommand {
                name: "/quit".into(),
                description: "exit mew".into(),
            },
            SlashCommand {
                name: "/sessions".into(),
                description: "list previous sessions".into(),
            },
            SlashCommand {
                name: "/resume".into(),
                description: "resume a session (e.g. /resume <id>)".into(),
            },
            SlashCommand {
                name: "/rewind".into(),
                description: "rewind to an earlier point (e.g. /rewind 3)".into(),
            },
            SlashCommand {
                name: "/mouse".into(),
                description: "toggle mouse capture for text selection".into(),
            },
            SlashCommand {
                name: "/theme".into(),
                description: "switch theme (e.g. /theme light, /theme dark)".into(),
            },
            SlashCommand {
                name: "/web".into(),
                description: "show the web UI URL for the current session".into(),
            },
            SlashCommand {
                name: "/yield".into(),
                description: "yield control to other clients".into(),
            },
            SlashCommand {
                name: "/autotitle".into(),
                description: "toggle auto session titling (daemon mode)".into(),
            },
            SlashCommand {
                name: "/autosummary".into(),
                description: "toggle auto session summaries (daemon mode)".into(),
            },
        ]
    }

    pub fn all_slash_commands(&self) -> Vec<SlashCommand> {
        let mut cmds = Self::builtin_slash_commands();
        cmds.extend(self.dynamic_slash_commands.clone());
        cmds
    }

    pub fn add_dynamic_slash_commands(&mut self, cmds: Vec<SlashCommand>) {
        self.dynamic_slash_commands.extend(cmds);
    }

    pub fn handle_slash(&self, input: &str) -> SlashResult {
        let (cmd, arg) = match input.split_once(' ') {
            Some((c, a)) => (c, Some(a)),
            None => (input, None),
        };
        match cmd {
            "/quit" | "/q" => SlashResult::Quit,
            "/clear" => SlashResult::Clear,
            "/compact" => SlashResult::Compact,
            "/todo" => SlashResult::Todo,
            "/cost" => SlashResult::Message(self.build_cost_report()),
            "/help" => SlashResult::OpenHelp,
            "/model" | "/models" => {
                if let Some(id) = arg {
                    SlashResult::SwitchModel(id.to_string())
                } else {
                    SlashResult::OpenModelPicker
                }
            }
            "/thinking" => {
                if let Some(variant) = arg {
                    SlashResult::SetThinkingVariant(variant.trim().to_string())
                } else {
                    SlashResult::OpenThinkingVariantPicker
                }
            }
            "/theme" => {
                if let Some(name) = arg {
                    SlashResult::SetTheme(name.trim().to_string())
                } else {
                    SlashResult::OpenThemePicker
                }
            }
            "/persona" => {
                if let Some(name) = arg {
                    // Clearing the persona is idempotent and unambiguous; do
                    // it directly. Switching to the currently active persona
                    // is a no-op. Any other switch goes through the confirm
                    // modal so the user sees the model/toolset diff.
                    if name == "default" || name == "none" {
                        SlashResult::SwitchPersona(name.to_string())
                    } else if self.active_persona.as_deref() == Some(name) {
                        SlashResult::Message(format!("persona '{name}' is already active"))
                    } else {
                        SlashResult::PersonaSwitchConfirm(name.to_string())
                    }
                } else {
                    SlashResult::OpenPersonaPicker
                }
            }
            "/permissions" | "/permission" => {
                // `/permissions`        → open the picker
                // `/permissions standard|permissive|auto|auto_plus|dangerous` → switch directly
                if let Some(arg) = arg {
                    let mode = arg.trim();
                    match mew_hooks::PermissionMode::from_id(mode) {
                        Some(m) => SlashResult::SetPermissionMode(m),
                        None => SlashResult::Message(format!(
                            "unknown permission mode '{mode}'; expected 'standard', 'permissive', 'auto', 'auto_plus', or 'dangerous'"
                        )),
                    }
                } else {
                    SlashResult::PermissionModeMenu
                }
            }
            "/sessions" | "/session" => SlashResult::OpenSessionPickerFromDisk,
            "/autotitle" | "/autosummary" => {
                SlashResult::Message("this command is only available in daemon mode".into())
            }
            "/mouse" | "/m" => SlashResult::ToggleMouseCapture,
            "/resume" => {
                if let Some(id) = arg {
                    SlashResult::ResumeSession(id.to_string())
                } else {
                    SlashResult::OpenSessionPickerFromDisk
                }
            }
            "/rewind" => {
                if let Some(n_str) = arg {
                    match n_str.parse::<usize>() {
                        Ok(n) => SlashResult::Rewind(n),
                        Err(_) => SlashResult::Message(
                            "usage: /rewind <n> — n is the number of messages to keep".into(),
                        ),
                    }
                } else {
                    SlashResult::OpenRewindPicker
                }
            }
            _ => {
                // Check dynamic/plugin commands.
                if let Some(dyn_cmd) = self.dynamic_slash_commands.iter().find(|c| c.name == cmd) {
                    let args = arg.unwrap_or("").to_string();
                    SlashResult::PluginCommand {
                        name: dyn_cmd.name.clone(),
                        args,
                    }
                } else {
                    SlashResult::Continue
                }
            }
        }
    }

    pub fn filtered_slash_commands(&self) -> Vec<SlashCommand> {
        let all = self.all_slash_commands();
        if !self.input.starts_with('/') {
            return Vec::new();
        }
        let prefix = self.input.to_lowercase();
        all.into_iter()
            .filter(|c| c.name.to_lowercase().starts_with(&prefix))
            .collect()
    }
}
