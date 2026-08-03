use super::*;

pub(super) fn transcript_part_block_count(
    part: Option<&TranscriptPart>,
    cached: &CachedMarkdown,
    reasoning_expanded: bool,
) -> usize {
    match part {
        None | Some(TranscriptPart::Text(_)) => cached.render_blocks.len().max(1),
        Some(TranscriptPart::Reasoning(_)) if reasoning_expanded => {
            cached.render_blocks.len().max(1)
        }
        Some(_) => 1,
    }
}

impl DesktopShell {
    pub(super) fn refresh_review(&mut self, cx: &mut Context<Self>) {
        if !self.model.session_is_ready() {
            self.clear_review();
            return;
        }
        let Some(selected_session) = self.model.ui.selected_session.as_deref() else {
            self.clear_review();
            return;
        };
        let Some(conversation) = self
            .model
            .ui
            .conversations
            .iter()
            .find(|conversation| conversation.session_id == selected_session)
        else {
            self.clear_review();
            return;
        };
        let Some(root) = conversation.cwd.clone() else {
            self.clear_review();
            return;
        };
        let paths = self.model.ui.review.files.clone();
        let signature = format!(
            "{selected_session}|{root}|{}|{}|{paths:?}",
            self.model.ui.review.added, self.model.ui.review.removed
        );
        if self.review_signature.as_deref() == Some(signature.as_str()) {
            return;
        }

        self.review_signature = Some(signature.clone());
        self.review_diffs.clear();
        self.review_lines.clear();
        self.review_selected_file = None;
        self.review_line_list.reset(0);
        self.review_error = None;
        self.review_loading = !paths.is_empty();
        if paths.is_empty() {
            return;
        }

        let (sender, receiver) = oneshot::channel();
        let root_path = PathBuf::from(root);
        let worker = std::thread::Builder::new()
            .name("mew-review-loader".into())
            .spawn(move || {
                let result = mew_diff::load_worktree_diffs(&root_path, &paths);
                let _ = sender.send(result);
            });
        if let Err(error) = worker {
            self.review_loading = false;
            self.review_error = Some(format!("could not start diff loader: {error}"));
            return;
        }

        cx.spawn(async move |this, cx| {
            let Ok(result) = receiver.await else {
                return;
            };
            this.update(cx, |shell, cx| {
                if shell.review_signature.as_deref() != Some(signature.as_str()) {
                    return;
                }
                shell.review_loading = false;
                match result {
                    Ok(diffs) => {
                        shell.review_diffs = diffs;
                        if shell.review_selected_file.is_none() {
                            if let Some(diff) = shell.review_diffs.first() {
                                shell.review_selected_file = Some(0);
                                shell.review_lines = diff.lines().cloned().collect();
                                shell.review_line_list.reset(shell.review_lines.len());
                            }
                        }
                    }
                    Err(error) => shell.review_error = Some(error.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn clear_review(&mut self) {
        self.review_diffs.clear();
        self.review_lines.clear();
        self.review_selected_file = None;
        self.review_line_list.reset(0);
        self.review_signature = None;
        self.review_loading = false;
        self.review_error = None;
    }

    pub(super) fn select_review_file(&mut self, index: usize, cx: &mut Context<Self>) {
        self.review_selected_file = Some(index);
        self.review_lines = self
            .review_diffs
            .get(index)
            .map(|diff| diff.lines().cloned().collect())
            .unwrap_or_default();
        self.review_line_list.reset(self.review_lines.len());
        cx.notify();
    }

    pub(super) fn sync_markdown_cache(&mut self) {
        let mut changed = self.markdown_cache.len() != self.model.ui.transcript.len();
        let mut changed_message_start = None;
        let mut changed_message_end = None;
        self.markdown_cache.truncate(self.model.ui.transcript.len());
        for (index, item) in self.model.ui.transcript.iter().enumerate() {
            let part_count = if item.parts.is_empty() {
                1
            } else {
                item.parts.len()
            };
            let part_count_changed = self
                .markdown_cache
                .get(index)
                .is_none_or(|cached| cached.len() != part_count);
            if part_count_changed {
                changed = true;
                changed_message_start =
                    Some(changed_message_start.map_or(index, |start: usize| start.min(index)));
                changed_message_end =
                    Some(changed_message_end.map_or(index, |end: usize| end.max(index)));
            }
            if self.markdown_cache.len() <= index {
                self.markdown_cache.resize_with(index + 1, Vec::new);
            }
            let cached_parts = &mut self.markdown_cache[index];
            cached_parts.truncate(part_count);
            cached_parts.resize_with(part_count, || CachedMarkdown {
                source: String::new(),
                source_identity: 0,
                source_len: 0,
                render_blocks: Vec::new(),
                streaming: None,
            });
            for (part_index, cached_part) in cached_parts.iter_mut().enumerate().take(part_count) {
                let part = item.parts.get(part_index);
                let Some((source, source_identity)) = (match part {
                    None => Some((
                        Cow::Borrowed(item.text.as_str()),
                        item.text.as_ptr() as usize,
                    )),
                    Some(TranscriptPart::Text(text) | TranscriptPart::Reasoning(text)) => {
                        Some((Cow::Borrowed(text.as_str()), text.as_ptr() as usize))
                    }
                    Some(_) => None,
                }) else {
                    if cached_part.source_identity != 0
                        || cached_part.source_len != 0
                        || !cached_part.source.is_empty()
                        || !cached_part.render_blocks.is_empty()
                    {
                        changed = true;
                        changed_message_start = Some(
                            changed_message_start.map_or(index, |start: usize| start.min(index)),
                        );
                        changed_message_end =
                            Some(changed_message_end.map_or(index, |end: usize| end.max(index)));
                    }
                    cached_part.source.clear();
                    cached_part.source_identity = 0;
                    cached_part.source_len = 0;
                    cached_part.render_blocks.clear();
                    continue;
                };
                let source_len = source.len();
                let source_unchanged =
                    cached_markdown_source_unchanged(cached_part, source.as_ref(), source_identity);
                if source_unchanged {
                    continue;
                }
                changed = true;
                changed_message_start =
                    Some(changed_message_start.map_or(index, |start: usize| start.min(index)));
                changed_message_end =
                    Some(changed_message_end.map_or(index, |end: usize| end.max(index)));
                let (render_blocks, streaming) = match part {
                    None | Some(TranscriptPart::Text(_)) | Some(TranscriptPart::Reasoning(_)) => {
                        let append_only = cached_part.streaming.is_some()
                            && source.as_ref().starts_with(cached_part.source.as_str());
                        let mut streaming = if append_only {
                            cached_part
                                .streaming
                                .take()
                                .expect("streaming cache exists for append-only markdown")
                        } else {
                            StreamingMarkdown::new()
                        };
                        let document = if append_only {
                            streaming.append(&source.as_ref()[cached_part.source.len()..])
                        } else {
                            let document = parse_document(source.as_ref());
                            streaming.append(source.as_ref());
                            document
                        };
                        let mut blocks = virtualize_document(&document);
                        highlight_code_blocks(&mut blocks);
                        (blocks, Some(streaming))
                    }
                    Some(_) => (Vec::new(), None),
                };
                *cached_part = CachedMarkdown {
                    source: source.into_owned(),
                    source_identity,
                    source_len,
                    render_blocks,
                    streaming,
                };
            }
        }
        if !changed {
            return;
        }
        if self.markdown_cache.len() != self.model.ui.transcript.len() {
            self.rebuild_transcript_rows_from_cache();
        } else if let (Some(message_start), Some(message_end)) =
            (changed_message_start, changed_message_end)
        {
            self.rebuild_transcript_rows_in_range(message_start, message_end);
        } else {
            self.rebuild_transcript_rows_from_cache();
        }
        self.sync_transcript_list();
        if let (Some(message_start), Some(message_end)) =
            (changed_message_start, changed_message_end)
        {
            let list_count = self.transcript_list.item_count();
            if let Some(row_start) = self
                .transcript_rows
                .iter()
                .position(|row| row.message_index >= message_start)
            {
                let row_end = self
                    .transcript_rows
                    .iter()
                    .position(|row| row.message_index > message_end)
                    .unwrap_or(self.transcript_rows.len())
                    .min(list_count);
                if row_start < row_end {
                    self.transcript_list.remeasure_items(row_start..row_end);
                }
            }
        }
    }

    pub(super) fn rebuild_transcript_rows_from_cache(&mut self) {
        let old_rows = std::mem::take(&mut self.transcript_rows);
        let mut new_rows = Vec::new();
        for (message_index, item) in self.model.ui.transcript.iter().enumerate() {
            new_rows.extend(self.transcript_rows_for_message(message_index, item));
        }
        self.transcript_rows = new_rows;
        self.transcript_rows_append_only = self.transcript_rows.len() >= old_rows.len()
            && self.transcript_rows.starts_with(&old_rows);
    }

    fn transcript_rows_for_message(
        &self,
        message_index: usize,
        item: &TranscriptItem,
    ) -> Vec<TranscriptRenderRow> {
        let part_count = if item.parts.is_empty() {
            1
        } else {
            item.parts.len()
        };
        let mut rows = Vec::new();
        for part_index in 0..part_count {
            let Some(cached) = self
                .markdown_cache
                .get(message_index)
                .and_then(|parts| parts.get(part_index))
            else {
                continue;
            };
            let block_count = transcript_part_block_count(
                item.parts.get(part_index),
                cached,
                matches!(
                    item.parts.get(part_index),
                    Some(TranscriptPart::Reasoning(_))
                ) && self
                    .expanded_chat_parts
                    .contains(&format!("chat-part-{message_index}-{part_index}")),
            );
            rows.extend((0..block_count).map(|block_index| TranscriptRenderRow {
                message_index,
                part_index,
                block_index,
            }));
        }
        rows
    }

    fn rebuild_transcript_rows_in_range(&mut self, message_start: usize, message_end: usize) {
        let Some(last_message) = self.model.ui.transcript.len().checked_sub(1) else {
            self.transcript_rows.clear();
            self.transcript_rows_append_only = false;
            return;
        };
        let message_start = message_start.min(last_message);
        let message_end = message_end.min(last_message).max(message_start);
        let old_rows = std::mem::take(&mut self.transcript_rows);
        let row_start = old_rows
            .iter()
            .position(|row| row.message_index >= message_start)
            .unwrap_or(old_rows.len());
        let row_end = old_rows
            .iter()
            .position(|row| row.message_index > message_end)
            .unwrap_or(old_rows.len());
        let replacement = (message_start..=message_end)
            .filter_map(|message_index| {
                self.model
                    .ui
                    .transcript
                    .get(message_index)
                    .map(|item| (message_index, item))
            })
            .flat_map(|(message_index, item)| self.transcript_rows_for_message(message_index, item))
            .collect::<Vec<_>>();
        let mut new_rows =
            Vec::with_capacity(old_rows.len() - (row_end - row_start) + replacement.len());
        new_rows.extend_from_slice(&old_rows[..row_start]);
        new_rows.extend(replacement);
        new_rows.extend_from_slice(&old_rows[row_end..]);
        self.transcript_rows_append_only =
            new_rows.len() >= old_rows.len() && new_rows.starts_with(&old_rows);
        self.transcript_rows = new_rows;
    }

    pub(super) fn send_pending_prompt(&mut self) {
        if !self.model.session_is_ready() {
            return;
        }
        if let Some(text) = self.pending_prompt.take() {
            let attachments = std::mem::take(&mut self.pending_attachments);
            self.send_command(ClientMessage::Prompt { text, attachments });
        }
    }

    pub(super) fn add_attachment(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if !path.is_file() {
            self.attachment_error = Some("Only files can be attached.".into());
            cx.notify();
            return;
        }
        match std::fs::metadata(&path) {
            Ok(metadata) if metadata.len() > MAX_ATTACHMENT_BYTES => {
                self.attachment_error = Some(format!(
                    "File is too large. Attachments are limited to {} MB.",
                    MAX_ATTACHMENT_BYTES / (1024 * 1024)
                ));
                cx.notify();
                return;
            }
            Err(error) => {
                self.attachment_error = Some(format!("Could not read file: {error}"));
                cx.notify();
                return;
            }
            _ => {}
        }
        let path_string = path.to_string_lossy().into_owned();
        if self
            .attachments
            .iter()
            .any(|attachment| attachment.path == path_string)
        {
            return;
        }
        self.attachments.push(Attachment {
            path: path_string,
            mime: attachment_mime(&path).map(str::to_owned),
        });
        self.attachment_error = None;
        cx.notify();
    }

    pub(super) fn pick_attachments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths_receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach files".into()),
        });
        let shell = cx.entity();
        window
            .spawn(cx, async move |cx| {
                let paths = match paths_receiver.await {
                    Ok(Ok(Some(paths))) => paths,
                    _ => return Ok::<(), anyhow::Error>(()),
                };
                for path in paths {
                    shell.update(cx, |shell, cx| shell.add_attachment(path, cx));
                }
                Ok(())
            })
            .detach();
    }

    pub(super) fn remove_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.attachments.len() {
            self.attachments.remove(index);
            cx.notify();
        }
    }

    /// Requests a directory listing for the attached session's workspace.
    /// `path` is workspace-relative; `None` lists the workspace root.
    pub(super) fn request_dir_listing(&mut self, path: Option<String>) {
        if !self.model.session_is_ready() {
            return;
        }
        let Some(session_id) = self.model.attached_session.clone() else {
            return;
        };
        let has_cwd = self.model.ui.conversations.iter().any(|conversation| {
            conversation.session_id == session_id && conversation.cwd.is_some()
        });
        if !has_cwd {
            return;
        }
        if !self
            .file_tree_pending
            .insert(path.clone().unwrap_or_default())
        {
            return;
        }
        self.send_command(ClientMessage::ListDir { session_id, path });
    }

    pub(super) fn toggle_file_tree_dir(&mut self, path: String, cx: &mut Context<Self>) {
        if !self.file_tree_expanded.insert(path.clone()) {
            self.file_tree_expanded.remove(&path);
        } else if !self.file_tree_entries.contains_key(&path) {
            self.request_dir_listing(Some(path));
        }
        cx.notify();
    }

    pub(super) fn open_workspace_path(&mut self, path: String, cx: &mut Context<Self>) {
        if self.model.session_is_ready() {
            if let Some(session_id) = self.model.attached_session.clone() {
                self.send_command(ClientMessage::OpenPath { session_id, path });
            }
        }
        cx.notify();
    }

    /// Keeps the daemon's workspace watch aligned with the Local view: the
    /// watch is enabled only while the Local panel is visible on the
    /// attached session, mirroring the web client's right-rail lifecycle.
    pub(super) fn sync_workspace_watch(&mut self) {
        let desired =
            if self.auxiliary_view == AuxiliaryView::Local && self.model.session_is_ready() {
                self.model.attached_session.clone().filter(|session_id| {
                    self.model.ui.conversations.iter().any(|conversation| {
                        conversation.session_id == *session_id && conversation.cwd.is_some()
                    })
                })
            } else {
                None
            };
        if self.watching_workspace_session == desired {
            return;
        }
        if let Some(previous) = self.watching_workspace_session.take() {
            self.send_command(ClientMessage::WatchWorkspace {
                session_id: previous,
                enabled: false,
            });
        }
        self.file_tree_entries.clear();
        self.file_tree_pending.clear();
        self.file_tree_expanded.clear();
        if let Some(session_id) = desired.clone() {
            self.send_command(ClientMessage::WatchWorkspace {
                session_id,
                enabled: true,
            });
            self.watching_workspace_session = desired;
            self.request_dir_listing(None);
        }
    }

    /// Handles `ClientEvent::FileTreeChanged`. A response to one of our
    /// pending `ListDir` requests stores the listing; anything else is an
    /// `FsChanged` push, which triggers a re-request of the visible paths
    /// like the web store's `onFsChanged`.
    pub(super) fn apply_file_tree_update(&mut self, state: &ClientState) {
        if state.dir_listing_session_id.as_deref() != self.watching_workspace_session.as_deref() {
            return;
        }
        if let Some(path) = state.dir_listing_path.clone() {
            if self.file_tree_pending.remove(&path) {
                self.file_tree_entries
                    .insert(path, state.dir_listing.clone());
                return;
            }
        }
        if self.watching_workspace_session.is_some() {
            self.request_dir_listing(None);
            let expanded: Vec<String> = self.file_tree_expanded.iter().cloned().collect();
            for path in expanded {
                self.request_dir_listing(Some(path));
            }
        }
    }

    pub(super) fn toggle_connection_picker(&mut self, cx: &mut Context<Self>) {
        self.connection_picker_open = !self.connection_picker_open;
        cx.notify();
    }
}

/// Writes clipboard image bytes to a temp file so the existing attachment
/// pipeline (size cap, MIME detection, chips) can pick it up.
pub(super) fn save_clipboard_image(image: &gpui::Image) -> Result<PathBuf, String> {
    let directory = std::env::temp_dir().join("mew-paste");
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not prepare paste directory: {error}"))?;
    let timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default();
    let path = directory.join(clipboard_image_file_name(
        image.format.extension(),
        timestamp_ms,
    ));
    std::fs::write(&path, &image.bytes)
        .map_err(|error| format!("Could not save pasted image: {error}"))?;
    Ok(path)
}
