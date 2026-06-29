//! Central registry of every prompt mew sends to the LLM.
//!
//! One crate to look at when you ask "where does the system say X to the
//! model?" Submodules group the prompts by where they originate:
//!
//! - [`system`] — base system prompt assembled from context files.
//! - [`persona`] — persona body rendered through minijinja (when
//!   `template: true` is set in the persona frontmatter).
//! - [`skills`] — `<available_skills>` XML block.
//! - [`subagent`] — built-in subagent system prompts (researcher, reviewer,
//!   coder, etc.).
//! - [`classifier`] — permission-decision prompt for the future Auto mode
//!   (classifier-driven approval).
//!
//! Use [`inventory::inventory`] to enumerate every prompt this crate knows
//! about — useful for docs, tests, and tools that audit what the system says
//! to the model.
//!
//! Each module owns the prompt it produces and is the single source of truth
//! for that prompt's format. If you change the shape of `<available_skills>`,
//! edit `skills.rs`. If you change how classifier prompts read, edit
//! `classifier.rs`. There is intentionally no shared "template engine" — each
//! module uses whatever is most natural for its format (minijinja for the
//! persona template, plain string formatting for the classifier prompt).

pub mod classifier;
pub mod inventory;
pub mod persona;
pub mod skills;
pub mod subagent;
pub mod system;
pub mod template;
pub mod vfs;
