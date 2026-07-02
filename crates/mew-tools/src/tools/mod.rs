pub mod ask_user;
pub mod bash;
pub mod echo;
pub mod edit_hashline;
pub mod edit_str_replace;
pub mod exit_tool;
pub mod flag_important;
pub mod glob;
pub mod grep;
pub mod jobs;
pub mod progress_update;
pub mod read;
pub mod shell_session;
pub mod skill;
pub mod subagent;
pub mod subagent_start;
pub mod subagent_wait;
pub mod switch_persona;
pub mod todo;
pub mod web_fetch;
pub mod write;

use mew_hooks::FileDelta;

/// Compute line-level add/remove counts by diffing old vs new content.
/// Uses `similar` (already in-tree) for the line-level comparison.
pub fn compute_file_delta(old: Option<&str>, new: &str, path: &std::path::Path) -> FileDelta {
    use similar::TextDiff;

    let (added, removed) = match old {
        None => {
            // New file: all lines are "added".
            (new.lines().count() as u64, 0)
        }
        Some(old) => {
            let diff = TextDiff::from_lines(old, new);
            let mut added = 0u64;
            let mut removed = 0u64;
            for change in diff.iter_all_changes() {
                use similar::ChangeTag;
                match change.tag() {
                    ChangeTag::Insert => added += 1,
                    ChangeTag::Delete => removed += 1,
                    ChangeTag::Equal => {}
                }
            }
            (added, removed)
        }
    };

    FileDelta {
        path: path.display().to_string(),
        added,
        removed,
    }
}
