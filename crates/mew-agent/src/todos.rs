//! Session-lived, dependency-enforced todo list.
//!
//! The list lives in agent state (not in the message history), so it survives
//! context compaction. It's persisted to `<session>/todos.json` so it survives
//! resume too. Both the model (via the `todo_*` tools) and the user (via the
//! `/todo` command) read and shape the same list.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Done,
    Blocked,
}

impl TodoStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            TodoStatus::Pending => "pending",
            TodoStatus::InProgress => "in_progress",
            TodoStatus::Done => "done",
            TodoStatus::Blocked => "blocked",
        }
    }

    /// Parse a status from its snake_case wire form. Returns None on miss.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => TodoStatus::Pending,
            "in_progress" => TodoStatus::InProgress,
            "done" => TodoStatus::Done,
            "blocked" => TodoStatus::Blocked,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Todo {
    pub id: usize,
    pub content: String,
    pub status: TodoStatus,
    #[serde(default)]
    pub depends_on: Vec<usize>,
}

/// The full persisted list: the items plus a monotonic id counter so deletion
/// never renumbers existing items.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoList {
    #[serde(default)]
    pub next_id: usize,
    #[serde(default)]
    pub items: Vec<Todo>,
}

impl TodoList {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            items: Vec::new(),
        }
    }

    pub fn get(&self, id: usize) -> Option<&Todo> {
        self.items.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: usize) -> Option<&mut Todo> {
        self.items.iter_mut().find(|t| t.id == id)
    }

    /// Create one or more todos. Each gets the next sequential id; references
    /// to nonexistent dependencies are dropped (the caller surfaces this).
    /// Returns `(created, dropped_deps)` where `dropped_deps` lists the
    /// per-input dependency ids that didn't resolve.
    pub fn create(&mut self, items: Vec<(String, Vec<usize>)>) -> (Vec<Todo>, Vec<Vec<usize>>) {
        let mut created = Vec::new();
        let mut dropped = Vec::new();
        for (content, depends_on) in items {
            let mut valid: Vec<usize> = Vec::new();
            let mut invalid: Vec<usize> = Vec::new();
            for d in depends_on {
                if self.get(d).is_some() {
                    valid.push(d);
                } else {
                    invalid.push(d);
                }
            }
            let todo = Todo {
                id: self.next_id,
                content,
                status: TodoStatus::Pending,
                depends_on: valid,
            };
            self.next_id += 1;
            self.items.push(todo.clone());
            created.push(todo);
            dropped.push(invalid);
        }
        (created, dropped)
    }

    /// Mark a todo done. Errors if a dependency isn't done yet, enforcing the
    /// "can't finish before your deps" rule.
    pub fn complete(&mut self, id: usize) -> Result<(), String> {
        let deps: Vec<usize> = self
            .get(id)
            .map(|t| t.depends_on.clone())
            .ok_or_else(|| format!("no todo with id {}", id))?;
        let incomplete: Vec<usize> = deps
            .iter()
            .filter_map(|d| {
                if self.get(*d).map(|t| t.status) != Some(TodoStatus::Done) {
                    Some(*d)
                } else {
                    None
                }
            })
            .collect();
        if !incomplete.is_empty() {
            let verb = if incomplete.len() == 1 { "is" } else { "are" };
            return Err(format!(
                "cannot complete todo {}: depends on {} which {} not done",
                id,
                join_ids(&incomplete),
                verb,
            ));
        }
        self.get_mut(id).unwrap().status = TodoStatus::Done;
        Ok(())
    }

    /// Delete a todo. Errors if another todo depends on it, enforcing the
    /// "can't remove something with dependents" rule.
    pub fn delete(&mut self, id: usize) -> Result<Todo, String> {
        let blockers: Vec<usize> = self
            .items
            .iter()
            .filter(|t| t.id != id && t.depends_on.contains(&id))
            .map(|t| t.id)
            .collect();
        if !blockers.is_empty() {
            return Err(format!(
                "cannot delete todo {}: todo(s) {} depend on it",
                id,
                join_ids(&blockers),
            ));
        }
        let pos = self
            .items
            .iter()
            .position(|t| t.id == id)
            .ok_or_else(|| format!("no todo with id {}", id))?;
        Ok(self.items.remove(pos))
    }

    /// Update a todo's content and/or status. Moving to `Done` runs the same
    /// dependency check as `complete`.
    pub fn update(
        &mut self,
        id: usize,
        content: Option<String>,
        status: Option<TodoStatus>,
    ) -> Result<(), String> {
        if status == Some(TodoStatus::Done) {
            self.complete(id)?;
        }
        let todo = self
            .get_mut(id)
            .ok_or_else(|| format!("no todo with id {}", id))?;
        if let Some(c) = content {
            todo.content = c;
        }
        if let Some(s) = status {
            todo.status = s;
        }
        Ok(())
    }

    /// Render the list as a readable, scannable string. Used for tool output
    /// and the `/todo` command.
    pub fn render(&self) -> String {
        self.render_annotated(&std::collections::HashMap::new())
    }

    /// Render with per-todo notes (e.g. a linked subagent task) appended to
    /// the matching entries.
    pub fn render_annotated(&self, notes: &std::collections::HashMap<usize, String>) -> String {
        if self.items.is_empty() {
            return "(no todos)".to_string();
        }
        let mut out = String::new();
        for t in &self.items {
            let mark = match t.status {
                TodoStatus::Done => 'x',
                TodoStatus::InProgress => '~',
                TodoStatus::Pending => ' ',
                TodoStatus::Blocked => '!',
            };
            out.push_str(&format!("[{}] #{} {}", mark, t.id, t.content));
            if !t.depends_on.is_empty() {
                out.push_str(&format!(
                    " (depends: {})",
                    t.depends_on
                        .iter()
                        .map(|d| format!("#{}", d))
                        .collect::<Vec<_>>()
                        .join(", "),
                ));
            }
            if let Some(note) = notes.get(&t.id) {
                out.push_str(&format!(" -- {}", note));
            }
            out.push('\n');
        }
        out
    }

    /// Load from a `todos.json` file. Missing file → fresh empty list.
    pub async fn load(path: &Path) -> Result<Self, String> {
        match tokio::fs::read_to_string(path).await {
            Ok(data) => serde_json::from_str(&data).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Persist to a `todos.json` file (creates parent dirs).
    pub async fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| e.to_string())?;
        }
        let data = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        tokio::fs::write(path, data)
            .await
            .map_err(|e| e.to_string())
    }
}

fn join_ids(ids: &[usize]) -> String {
    ids.iter()
        .map(|i| format!("#{}", i))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_assigns_sequential_ids() {
        let mut list = TodoList::new();
        let (created, dropped) =
            list.create(vec![("first".into(), vec![]), ("second".into(), vec![])]);
        assert_eq!(created.len(), 2);
        assert_eq!(created[0].id, 1);
        assert_eq!(created[1].id, 2);
        assert_eq!(list.next_id, 3);
        assert!(dropped.iter().all(|d| d.is_empty()));
    }

    #[test]
    fn test_create_drops_nonexistent_deps() {
        let mut list = TodoList::new();
        list.create(vec![("base".into(), vec![])]);
        // Reference #1 (exists) and #99 (doesn't).
        let (_created, dropped) = list.create(vec![("child".into(), vec![1, 99])]);
        assert_eq!(dropped, vec![vec![99]]);
        assert_eq!(list.get(2).unwrap().depends_on, vec![1]);
    }

    #[test]
    fn test_complete_blocked_by_incomplete_dependency() {
        let mut list = TodoList::new();
        list.create(vec![("base".into(), vec![])]);
        list.create(vec![("child".into(), vec![1])]);
        // Can't complete #2 before #1.
        let err = list.complete(2).unwrap_err();
        assert!(err.contains("depends on #1"), "{}", err);
        assert_eq!(list.get(2).unwrap().status, TodoStatus::Pending);
    }

    #[test]
    fn test_complete_succeeds_after_dependency_done() {
        let mut list = TodoList::new();
        list.create(vec![("base".into(), vec![])]);
        list.create(vec![("child".into(), vec![1])]);
        list.complete(1).unwrap();
        assert_eq!(list.get(1).unwrap().status, TodoStatus::Done);
        list.complete(2).unwrap();
        assert_eq!(list.get(2).unwrap().status, TodoStatus::Done);
    }

    #[test]
    fn test_complete_unknown_id_errors() {
        let mut list = TodoList::new();
        let err = list.complete(99).unwrap_err();
        assert!(err.contains("no todo with id 99"));
    }

    #[test]
    fn test_delete_blocked_by_dependent() {
        let mut list = TodoList::new();
        list.create(vec![("base".into(), vec![])]);
        list.create(vec![("child".into(), vec![1])]);
        let err = list.delete(1).unwrap_err();
        assert!(err.contains("depend on it"), "{}", err);
        assert_eq!(list.items.len(), 2, "nothing removed on error");
    }

    #[test]
    fn test_delete_succeeds_when_no_dependents() {
        let mut list = TodoList::new();
        list.create(vec![("a".to_string(), vec![]), ("b".to_string(), vec![])]);
        let removed = list.delete(1).unwrap();
        assert_eq!(removed.id, 1);
        assert_eq!(list.items.len(), 1);
        // Ids don't shift: #2 is still #2.
        assert_eq!(list.get(2).unwrap().content, "b");
    }

    #[test]
    fn test_update_content_and_status() {
        let mut list = TodoList::new();
        list.create(vec![("orig".into(), vec![])]);
        list.update(1, Some("edited".into()), Some(TodoStatus::InProgress))
            .unwrap();
        let t = list.get(1).unwrap();
        assert_eq!(t.content, "edited");
        assert_eq!(t.status, TodoStatus::InProgress);
    }

    #[test]
    fn test_update_to_done_enforces_dependencies() {
        let mut list = TodoList::new();
        list.create(vec![("base".into(), vec![])]);
        list.create(vec![("child".into(), vec![1])]);
        let err = list.update(2, None, Some(TodoStatus::Done)).unwrap_err();
        assert!(err.contains("depends on #1"));
        assert_eq!(list.get(2).unwrap().status, TodoStatus::Pending);
    }

    #[test]
    fn test_render_marks_status() {
        let mut list = TodoList::new();
        list.create(vec![("pending".into(), vec![])]);
        list.create(vec![("in-progress".into(), vec![])]);
        list.create(vec![("done".into(), vec![])]);
        list.update(2, None, Some(TodoStatus::InProgress)).unwrap();
        list.complete(3).unwrap();
        let rendered = list.render();
        assert!(rendered.contains("[ ] #1 pending"));
        assert!(rendered.contains("[~] #2 in-progress"));
        assert!(rendered.contains("[x] #3 done"));
    }

    #[test]
    fn test_render_shows_dependencies() {
        let mut list = TodoList::new();
        list.create(vec![("base".into(), vec![])]);
        list.create(vec![("child".into(), vec![1])]);
        let rendered = list.render();
        assert!(rendered.contains("(depends: #1)"), "{}", rendered);
    }

    #[test]
    fn test_render_empty_list() {
        let list = TodoList::new();
        assert_eq!(list.render(), "(no todos)");
    }

    #[tokio::test]
    async fn test_persistence_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("todos.json");
        let mut list = TodoList::new();
        list.create(vec![("task".into(), vec![])]);
        list.complete(1).unwrap();
        list.save(&path).await.unwrap();

        let loaded = TodoList::load(&path).await.unwrap();
        assert_eq!(loaded, list);
        assert_eq!(loaded.next_id, 2);
        assert_eq!(loaded.get(1).unwrap().status, TodoStatus::Done);
    }

    #[tokio::test]
    async fn test_load_missing_file_returns_fresh() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nonexistent.json");
        let list = TodoList::load(&path).await.unwrap();
        assert!(list.items.is_empty());
        assert_eq!(list.next_id, 1);
    }
}

#[cfg(test)]
mod annotation_tests {
    use super::*;

    #[test]
    fn test_render_annotated_appends_notes_to_linked_todos() {
        let mut list = TodoList::new();
        list.create(vec![("linked".into(), vec![])]);
        list.create(vec![("unlinked".into(), vec![])]);
        let notes = std::collections::HashMap::from([(1usize, "subagent stub (12s)".to_string())]);
        let rendered = list.render_annotated(&notes);
        assert!(rendered.contains("#1 linked -- subagent stub (12s)"));
        assert!(rendered.contains("[ ] #2 unlinked\n"));
        // Plain render is unaffected.
        assert!(!list.render().contains("subagent stub"));
    }
}
