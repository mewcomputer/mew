#[cfg(test)]
mod shell_tests {
    use super::super::browser::replace_browser_event_value;
    use super::super::composer::composer_input_height;
    use super::super::lifecycle::transcript_is_scrolled_away;
    use super::super::session_data::transcript_part_block_count;
    use super::super::*;

    fn navigation_conversation(session_id: &str) -> ConversationItem {
        ConversationItem {
            session_id: session_id.into(),
            title: format!("Chat {session_id}"),
            cwd: Some("/workspace/mew/subdirectory".into()),
            workspace_path: Some("/workspace/mew".into()),
            last_message_at: None,
            state: mew_protocol::SessionState::Idle,
            last_turn_failed: false,
            failure_detail: None,
            unread_completion: false,
            needs_attention: false,
            archived: false,
            pinned: false,
            group_id: None,
        }
    }

    #[test]
    fn numeric_shortcuts_do_not_select_hidden_conversations() {
        for key in ["1", "2", "3", "4", "5", "6", "7", "8", "9"] {
            assert_eq!(shell_command_for_key(key, true), None);
        }
    }

    #[gpui::test]
    fn new_chat_inherits_the_selected_workspace(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        let (sender, mut commands) = tokio::sync::mpsc::unbounded_channel();
        shell.update(cx, |shell, cx| {
            shell.command_tx = Some(sender);
            shell.model.ui.conversations = vec![navigation_conversation("a")];
            shell.model.ui.selected_session = Some("a".into());
            shell.new_conversation(cx);
            shell.quitting = true;
        });
        let command = commands.try_recv().unwrap();
        assert!(
            matches!(command, ClientMessage::NewSession { cwd: Some(path), .. }
            if path == "/workspace/mew")
        );
    }

    #[gpui::test]
    fn closing_a_chat_preserves_its_draft_for_sidebar_navigation(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        shell.update(cx, |shell, _| {
            shell.model.ui.conversations = vec![navigation_conversation("a")];
            shell.switch_draft_owner(Some("a"));
            shell.model.ui.selected_session = Some("a".into());
            shell.model.ui.composer = "keep this follow-up".into();
            shell.attachments = vec![Attachment {
                path: "a.txt".into(),
                mime: None,
            }];
        });
        cx.update(|window, cx| {
            let focus = shell.read(cx).composer_focus_handle.clone();
            window.focus(&focus, cx);
        });
        cx.run_until_parked();
        cx.dispatch_action(CloseConversation);
        shell.update(cx, |shell, cx| {
            assert!(shell.model.ui.selected_session.is_none());
            assert!(shell.model.ui.composer.is_empty());
            assert!(shell.attachments.is_empty());
            shell.attach_session("a".into(), cx);
            assert_eq!(shell.model.ui.composer, "keep this follow-up");
            assert_eq!(shell.attachments[0].path, "a.txt");
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn transcript_keeps_horizontal_gutters_in_split_and_folded_layouts(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = false;
            shell.layout.workbench_collapsed = false;
            shell.layout.terminal_collapsed = true;
            shell.sidebar_animation_id = 0;
            shell.workbench_animation_id = 0;
            shell.model.ui.transcript = vec![TranscriptItem {
                role: TranscriptRole::Assistant,
                text: "This reply needs space beside the pane divider.".into(),
                parts: Vec::new(),
            }];
            shell.sync_markdown_cache();
            cx.notify();
        });
        for width in [1240., 900.] {
            cx.simulate_resize(gpui::size(px(width), px(760.)));
            cx.refresh().unwrap();
            cx.run_until_parked();
            let conversation = cx.debug_bounds("conversation-pane").unwrap();
            shell.update(cx, |shell, _| {
                let viewport = shell.transcript_list.viewport_bounds();
                assert_eq!(
                    viewport.origin.x,
                    conversation.origin.x + px(CHAT_TRANSCRIPT_PADDING)
                );
                assert_eq!(
                    viewport.right(),
                    conversation.right() - px(CHAT_TRANSCRIPT_PADDING)
                );
            });
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn active_chat_uses_one_bounded_column_for_messages_and_composer(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.transcript = vec![
                TranscriptItem {
                    role: TranscriptRole::User,
                    text: "Keep this longer user message readable inside the conversation pane, including when the browser is open and there is less room for the chat.".into(),
                    parts: Vec::new(),
                },
                TranscriptItem {
                    role: TranscriptRole::Assistant,
                    text: "The reply and composer share a reading column while the pane stays resizable.".into(),
                    parts: Vec::new(),
                },
            ];
            shell.sync_markdown_cache();
            cx.notify();
        });
        for width in [420., 480., 1240.] {
            cx.simulate_resize(gpui::size(px(width), px(900.)));
            cx.refresh().unwrap();
            cx.run_until_parked();
            let composer = cx.debug_bounds("composer").unwrap();
            let reply = cx.debug_bounds("transcript-content-1").unwrap();
            let user = cx.debug_bounds("transcript-content-0").unwrap();
            assert_eq!(reply.left(), composer.left());
            assert_eq!(reply.right(), composer.right());
            assert!(reply.size.width <= px(CHAT_CONTENT_MAX_WIDTH));
            assert!(
                user.left() >= composer.left(),
                "user message clipped at {width}"
            );
            assert!(user.right() <= composer.right());
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn active_chat_separates_turns_more_than_parts(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.transcript = vec![
                TranscriptItem {
                    role: TranscriptRole::User,
                    text: "check the layout".into(),
                    parts: Vec::new(),
                },
                TranscriptItem {
                    role: TranscriptRole::Assistant,
                    text: String::new(),
                    parts: vec![
                        TranscriptPart::Text("checking the view".into()),
                        TranscriptPart::Text("the layout is ready".into()),
                    ],
                },
                TranscriptItem {
                    role: TranscriptRole::User,
                    text: "keep going".into(),
                    parts: Vec::new(),
                },
            ];
            shell.sync_markdown_cache();
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(680.), px(900.)));
        cx.run_until_parked();
        let user = cx.debug_bounds("transcript-content-0").unwrap();
        let first = cx.debug_bounds("transcript-content-1").unwrap();
        let second = cx.debug_bounds("transcript-content-2").unwrap();
        let followup = cx.debug_bounds("transcript-content-3").unwrap();
        let part_gap = second.top() - first.bottom();
        assert!(part_gap >= px(8.));
        assert!(first.top() - user.bottom() >= px(24.));
        assert!(followup.top() - second.bottom() >= px(24.));
        assert!(part_gap < first.top() - user.bottom());
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn narrow_composer_keeps_long_setting_labels_and_send_inside_its_bounds(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.model =
                Some("a-provider/model-with-a-long-name-and-extra-high-effort".into());
            shell.model.ui.current_persona = Some("a-persona-with-a-long-display-name".into());
            let mut conversation = navigation_conversation("layout");
            conversation.cwd = Some(
                "/workspace/a-project-with-a-very-long-name/a-deeply-nested-working-directory"
                    .into(),
            );
            shell.model.ui.selected_session = Some("layout".into());
            shell.model.ui.conversations = vec![conversation];
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(420.), px(760.)));
        cx.run_until_parked();
        let composer = cx.debug_bounds("composer").unwrap();
        let mut controls = Vec::new();
        for selector in [
            "permission-picker-trigger",
            "persona-picker-trigger",
            "model-picker-trigger",
            "submit-prompt",
        ] {
            let bounds = cx.debug_bounds(selector).unwrap();
            assert!(
                bounds.left() >= composer.left(),
                "{selector} crosses left edge"
            );
            assert!(
                bounds.right() <= composer.right(),
                "{selector} crosses right edge"
            );
            assert!(bounds.top() >= composer.top() && bounds.bottom() <= composer.bottom());
            assert!(bounds.size.width > px(0.));
            controls.push(bounds);
        }
        for (index, a) in controls.iter().enumerate() {
            for b in &controls[index + 1..] {
                assert!(!a.intersects(b), "composer controls overlap");
            }
        }
        assert_eq!(controls.last().unwrap().size.width, px(28.));
        let path = cx.debug_bounds("checkout-path").unwrap();
        let status = cx.debug_bounds("session-status").unwrap();
        assert!(path.right() <= status.left());
        assert!(path.left() >= composer.left());
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn content_copy_code_button_copies_the_complete_fence(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        let code = (0..90)
            .map(|index| format!("let value_{index} = \"🙂中\";"))
            .collect::<Vec<_>>()
            .join("\n");
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.transcript = vec![TranscriptItem {
                role: TranscriptRole::Assistant,
                text: format!("```rust\n{code}\n```\n\n```text\nseparate fence\n```"),
                parts: Vec::new(),
            }];
            shell.sync_markdown_cache();
            assert!(shell.markdown_cache[0][0].render_blocks.len() > 2);
            shell.transcript_list.scroll_to(gpui::ListOffset::default());
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(680.), px(900.)));
        cx.run_until_parked();
        let button = cx
            .debug_bounds("copy-code-0-0")
            .expect("code has a copy button");
        cx.simulate_click(button.center(), gpui::Modifiers::none());
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some(code.as_str())
        );
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn content_copy_tool_buttons_copy_full_input_output_and_diff(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        let output = (0..70)
            .map(|index| format!("line {index} 🙂中"))
            .collect::<Vec<_>>()
            .join("\n");
        let diff = (0..70)
            .map(|index| format!("+line {index} é"))
            .collect::<Vec<_>>()
            .join("\n");
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.transcript = vec![TranscriptItem {
                role: TranscriptRole::Assistant,
                text: String::new(),
                parts: vec![TranscriptPart::ToolCall {
                    tool_name: "shell".into(),
                    call_id: "copy-call".into(),
                    status: ToolStatus::Completed,
                    input: r#"{"command":"printf '🙂中'"}"#.into(),
                    output: Some(output.clone()),
                    error: None,
                    diff: Some(diff.clone()),
                }],
            }];
            shell.expanded_chat_parts.insert("chat-part-0-0".into());
            shell.sync_markdown_cache();
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(680.), px(1200.)));
        cx.run_until_parked();
        for (selector, expected) in [
            (
                "copy-tool-input-0-0",
                "{\n  \"command\": \"printf '🙂中'\"\n}",
            ),
            ("copy-tool-output-0-0", output.as_str()),
            ("copy-tool-diff-0-0", diff.as_str()),
        ] {
            let button = cx
                .debug_bounds(selector)
                .expect("each tool section has a copy button");
            cx.simulate_click(button.center(), gpui::Modifiers::none());
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().as_deref(),
                Some(expected),
                "{selector}"
            );
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn content_copy_transcript_selection_takes_focus_from_the_composer(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        let text = "reply 🙂中";
        cx.update(|window, cx| {
            let focus = shell.update(cx, |shell, cx| {
                shell.layout.sidebar_collapsed = true;
                shell.layout.workbench_collapsed = true;
                shell.model.ui.composer = "keep this draft".into();
                shell.composer_selection = 0..4;
                shell.model.ui.transcript = vec![TranscriptItem {
                    role: TranscriptRole::Assistant,
                    text: text.into(),
                    parts: Vec::new(),
                }];
                shell.sync_markdown_cache();
                cx.notify();
                shell.composer_focus_handle.clone()
            });
            window.focus(&focus, cx);
        });
        cx.simulate_resize(gpui::size(px(680.), px(900.)));
        cx.run_until_parked();
        let bounds = cx.debug_bounds("markdown-inline-container-0-0").unwrap();
        let start = point(bounds.left() + px(0.1), bounds.top() + px(10.));
        let end = point(bounds.right() - px(1.), start.y);
        cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.run_until_parked();
        cx.simulate_mouse_move(end, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.simulate_mouse_up(end, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.run_until_parked();
        cx.dispatch_action(TextCopy);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some(text)
        );
        shell.read_with(cx, |shell, _| {
            assert_eq!(shell.model.ui.composer, "keep this draft")
        });
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn content_copy_code_and_tool_output_support_unicode_selection(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.transcript = vec![TranscriptItem {
                role: TranscriptRole::Assistant,
                text: String::new(),
                parts: vec![
                    TranscriptPart::Text("```text\ncode 🙂中\n```".into()),
                    TranscriptPart::ToolCall {
                        tool_name: "shell".into(),
                        call_id: "select-call".into(),
                        status: ToolStatus::Completed,
                        input: r#"{"command":"echo é"}"#.into(),
                        output: Some("first 🙂\nsecond 中\nlast é".into()),
                        error: None,
                        diff: Some("+added 🙂\n-removed 中".into()),
                    },
                ],
            }];
            shell.expanded_chat_parts.insert("chat-part-0-1".into());
            shell.sync_markdown_cache();
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(680.), px(1000.)));
        cx.run_until_parked();
        for (first, last, expected) in [
            (
                "markdown-inline-container-0-0",
                "markdown-inline-container-0-0",
                "code 🙂中",
            ),
            (
                "tool-input-0-1-line-0",
                "tool-input-0-1-line-2",
                "{\n  \"command\": \"echo é\"\n}",
            ),
            (
                "tool-output-0-1-line-0",
                "tool-output-0-1-line-2",
                "first 🙂\nsecond 中\nlast é",
            ),
            (
                "tool-diff-0-1-line-0",
                "tool-diff-0-1-line-1",
                "+added 🙂\n-removed 中",
            ),
        ] {
            let first = cx.debug_bounds(first).unwrap();
            let last = cx.debug_bounds(last).unwrap();
            let start = point(first.left() + px(0.1), first.top() + px(8.));
            let end = point(last.right() - px(1.), last.top() + px(8.));
            cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::none());
            cx.run_until_parked();
            cx.simulate_mouse_move(end, gpui::MouseButton::Left, gpui::Modifiers::none());
            cx.simulate_mouse_up(end, gpui::MouseButton::Left, gpui::Modifiers::none());
            cx.run_until_parked();
            cx.dispatch_action(TextCopy);
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().as_deref(),
                Some(expected)
            );
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn content_copy_selection_preserves_code_chunk_boundaries(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        let code = (0..24)
            .map(|index| format!("line_{index}: {}", "中".repeat(20)))
            .collect::<Vec<_>>()
            .join("\n");
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.transcript = vec![TranscriptItem {
                role: TranscriptRole::Assistant,
                text: format!("```text\n{code}\n```"),
                parts: Vec::new(),
            }];
            shell.sync_markdown_cache();
            assert_eq!(shell.markdown_cache[0][0].render_blocks.len(), 2);
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(900.), px(1400.)));
        cx.run_until_parked();
        let first = cx.debug_bounds("markdown-inline-container-0-0").unwrap();
        let last = cx.debug_bounds("markdown-inline-container-0-1").unwrap();
        let start = point(first.left() + px(0.1), first.top() + px(8.));
        let end = point(last.right() - px(1.), last.bottom() - px(8.));
        cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.run_until_parked();
        cx.simulate_mouse_move(end, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.simulate_mouse_up(end, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.run_until_parked();
        cx.dispatch_action(TextCopy);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some(code.as_str())
        );
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn content_copy_selection_keeps_code_chunks_that_scroll_out_of_view(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        let code = (0..160)
            .map(|index| format!("line_{index}: {}", "中".repeat(20)))
            .collect::<Vec<_>>()
            .join("\n");
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.transcript = vec![TranscriptItem {
                role: TranscriptRole::Assistant,
                text: format!("```text\n{code}\n```"),
                parts: Vec::new(),
            }];
            shell.sync_markdown_cache();
            shell.transcript_list.scroll_to(gpui::ListOffset::default());
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(900.), px(700.)));
        cx.run_until_parked();
        let first = cx.debug_bounds("markdown-inline-container-0-0").unwrap();
        let start = point(first.left() + px(0.1), first.top() + px(8.));
        cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.run_until_parked();
        shell.update(cx, |shell, cx| {
            shell.transcript_list.scroll_to(gpui::ListOffset {
                item_ix: shell.transcript_rows.len() - 1,
                offset_in_item: px(0.),
            });
            cx.notify();
        });
        cx.run_until_parked();
        let end = shell.read_with(cx, |shell, _| {
            let registry = shell.transcript_text_registry.borrow();
            assert!(registry.iter().all(|entry| entry.block_index > 0));
            let last = registry
                .iter()
                .max_by_key(|entry| entry.block_index)
                .unwrap();
            last.layout.position_for_index(last.text.len()).unwrap() + point(px(2.), px(8.))
        });
        cx.simulate_mouse_move(end, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.simulate_mouse_up(end, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.run_until_parked();
        cx.dispatch_action(TextCopy);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some(code.as_str())
        );
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn changing_chat_theme_refreshes_cached_code_colors(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        shell.update(cx, |shell, cx| {
            shell.model.ui.transcript = vec![TranscriptItem {
                role: TranscriptRole::Assistant,
                text: "```rust\nlet message = \"hello\";\n```".into(),
                parts: Vec::new(),
            }];
            shell.sync_markdown_cache();
            let source_pointer = shell.markdown_cache[0][0].source.as_ptr();
            for mode in [DesktopThemeMode::Light, DesktopThemeMode::Dark] {
                shell.theme_mode = mode;
                shell.reload_theme(WindowAppearance::Dark, cx);
                let cached = &shell.markdown_cache[0][0];
                assert_eq!(
                    cached.source.as_ptr(),
                    source_pointer,
                    "changing colors keeps the parsed source"
                );
                let row = &cached.render_blocks[0];
                let MarkdownBlock::Code { text, .. } = &row.block else {
                    panic!("expected code")
                };
                let color = row
                    .syntax_highlights
                    .iter()
                    .find(|highlight| text[highlight.range.clone()].contains("hello"))
                    .expect("string is highlighted")
                    .color;
                let Color::Rgb(r, g, b) = shell.theme.resolve("syntax.string") else {
                    panic!("RGB token")
                };
                assert_eq!(
                    color,
                    Some([r, g, b]),
                    "cached code follows the selected syntax palette"
                );
            }
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn completed_tool_rows_are_quieter_and_still_expand(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.transcript = [ToolStatus::Completed, ToolStatus::Running]
                .into_iter()
                .enumerate()
                .map(|(index, status)| TranscriptItem {
                    role: TranscriptRole::Assistant,
                    text: String::new(),
                    parts: vec![TranscriptPart::ToolCall {
                        tool_name: "shell".into(),
                        call_id: format!("call-{index}"),
                        status,
                        input: r#"{"command":"cargo test -p mew-desktop"}"#.into(),
                        output: Some("all tests passed".into()),
                        error: None,
                        diff: Some("-old line\n+new line".into()),
                    }],
                })
                .collect();
            shell.sync_markdown_cache();
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(680.), px(900.)));
        cx.run_until_parked();
        let completed = cx.debug_bounds("transcript-content-0").unwrap();
        let running = cx.debug_bounds("transcript-content-1").unwrap();
        assert!(completed.size.height < running.size.height);
        let header = cx.debug_bounds("chat-part-0-0-header").unwrap();
        cx.simulate_click(header.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert!(shell.expanded_chat_parts.contains("chat-part-0-0"));
            shell.quitting = true;
        });
        assert!(
            cx.debug_bounds("transcript-content-0").unwrap().size.height > completed.size.height
        );
        let output = cx
            .debug_bounds("tool-output-0-0-line-0")
            .expect("expanded tool output is rendered");
        assert!(output.size.height >= px(18.));
        assert!(cx.debug_bounds("tool-input-0-0-line-0").is_some());
        assert!(cx.debug_bounds("tool-diff-0-0-line-0").is_some());
    }

    #[gpui::test]
    fn completed_tool_groups_span_messages_and_reveal_each_call(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.transcript = vec![
                TranscriptItem {
                    role: TranscriptRole::Assistant,
                    text: String::new(),
                    parts: vec![
                        TranscriptPart::Text("checking the implementation".into()),
                        activity_tool("read_file", "read", ToolStatus::Completed),
                    ],
                },
                TranscriptItem {
                    role: TranscriptRole::Assistant,
                    text: String::new(),
                    parts: vec![
                        activity_tool("shell", "test", ToolStatus::Completed),
                        TranscriptPart::Text("the checks passed".into()),
                    ],
                },
            ];
            shell.sync_markdown_cache();
            assert_eq!(
                shell.transcript_rows.len(),
                3,
                "one activity summary between prose"
            );
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(680.), px(1000.)));
        cx.run_until_parked();
        assert!(cx.debug_bounds("chat-part-0-1-header").is_none());
        let group = cx.debug_bounds("tool-group-read-header").unwrap();
        cx.simulate_click(group.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        shell.update(cx, |shell, _| assert_eq!(shell.transcript_rows.len(), 5));
        let first_call = cx.debug_bounds("transcript-content-2").unwrap();
        let second_call = cx.debug_bounds("transcript-content-3").unwrap();
        assert!(
            second_call.top() - first_call.bottom() <= px(8.),
            "grouped calls stay together across provider messages"
        );
        let call = cx.debug_bounds("chat-part-1-0-header").unwrap();
        cx.simulate_click(call.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(cx.debug_bounds("tool-input-1-0-line-0").is_some());
        assert!(cx.debug_bounds("tool-output-1-0-line-0").is_some());
        assert!(cx.debug_bounds("tool-diff-1-0-line-0").is_some());
        let group = cx.debug_bounds("tool-group-read-header").unwrap();
        cx.simulate_click(group.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(cx.debug_bounds("chat-part-1-0-header").is_none());
        shell.update(cx, |shell, _| {
            assert_eq!(shell.transcript_rows.len(), 3);
            shell.quitting = true;
        });
    }

    fn activity_tool(name: &str, id: &str, status: ToolStatus) -> TranscriptPart {
        TranscriptPart::ToolCall {
            tool_name: name.into(),
            call_id: id.into(),
            status,
            input: r#"{"command":"cargo test","path":"src/main.rs"}"#.into(),
            output: Some(format!("output from {id}")),
            error: (status == ToolStatus::Error).then(|| "command failed".into()),
            diff: Some("-before\n+after".into()),
        }
    }

    #[gpui::test]
    fn tool_groups_keep_prose_unfinished_calls_and_turn_boundaries_visible(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        shell.update(cx, |shell, _| {
            let completed = |id| activity_tool("shell", id, ToolStatus::Completed);
            shell.model.ui.transcript = vec![
                TranscriptItem {
                    role: TranscriptRole::Assistant,
                    text: String::new(),
                    parts: vec![
                        completed("a"),
                        completed("b"),
                        TranscriptPart::Text("a checkpoint".into()),
                        completed("c"),
                        activity_tool("shell", "pending", ToolStatus::Pending),
                        activity_tool("shell", "running", ToolStatus::Running),
                        activity_tool("shell", "failed", ToolStatus::Error),
                        completed("d"),
                        completed("e"),
                    ],
                },
                TranscriptItem {
                    role: TranscriptRole::User,
                    text: "continue".into(),
                    parts: Vec::new(),
                },
                TranscriptItem {
                    role: TranscriptRole::Assistant,
                    text: String::new(),
                    parts: vec![completed("f")],
                },
            ];
            shell.sync_markdown_cache();
            assert_eq!(shell.transcript_rows.len(), 9);
            let parts: Vec<_> = shell
                .transcript_rows
                .iter()
                .map(|row| (row.message_index, row.part_index))
                .collect();
            assert_eq!(
                parts,
                vec![
                    (0, 0),
                    (0, 2),
                    (0, 3),
                    (0, 4),
                    (0, 5),
                    (0, 6),
                    (0, 7),
                    (1, 0),
                    (2, 0)
                ]
            );
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn tool_completion_updates_groups_without_a_markdown_change(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        shell.update(cx, |shell, _| {
            shell.model.ui.transcript = vec![TranscriptItem {
                role: TranscriptRole::Assistant,
                text: String::new(),
                parts: vec![
                    activity_tool("shell", "a", ToolStatus::Completed),
                    activity_tool("shell", "b", ToolStatus::Running),
                ],
            }];
            shell.sync_markdown_cache();
            assert_eq!(shell.transcript_rows.len(), 2);
            let TranscriptPart::ToolCall { status, .. } =
                &mut shell.model.ui.transcript[0].parts[1]
            else {
                unreachable!()
            };
            *status = ToolStatus::Completed;
            shell.pending_transcript_scroll_anchor =
                Some(mew_config::DesktopTranscriptScrollAnchor {
                    message_index: 0,
                    part_index: 1,
                    block_index: 0,
                    offset: 200.,
                });
            shell.sync_markdown_cache();
            assert_eq!(
                shell.transcript_rows.len(),
                1,
                "completion folds into the adjacent activity group"
            );
            assert_eq!(
                shell.transcript_list.logical_scroll_top().offset_in_item,
                px(0.),
                "a tall call's offset does not skip its replacement summary"
            );
            shell.expanded_chat_parts.insert("tool-group-a".into());
            shell.rebuild_transcript_rows_from_cache();
            assert_eq!(shell.transcript_rows.len(), 3);
            shell.model.ui.transcript[0].parts.push(activity_tool(
                "read_file",
                "c",
                ToolStatus::Completed,
            ));
            shell.sync_markdown_cache();
            assert_eq!(
                shell.transcript_rows.len(),
                4,
                "new calls retain the expanded group"
            );
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn prepending_tools_preserves_the_open_group_and_restores_its_anchor(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        shell.update(cx, |shell, cx| {
            let message = |id| TranscriptItem {
                role: TranscriptRole::Assistant,
                text: String::new(),
                parts: vec![activity_tool("shell", id, ToolStatus::Completed)],
            };
            shell.model.ui.transcript = vec![message("a"), message("b")];
            shell.expanded_chat_parts.insert("tool-group-a".into());
            shell.sync_markdown_cache();
            shell.pending_transcript_scroll_anchor =
                Some(mew_config::DesktopTranscriptScrollAnchor {
                    message_index: 0,
                    part_index: 0,
                    block_index: 1,
                    offset: 0.,
                });
            shell.prepare_for_history_prepend(1);
            shell.model.ui.transcript.insert(0, message("older"));
            shell.sync_markdown_cache();
            assert_eq!(
                shell.transcript_rows.len(),
                4,
                "the existing group remains expanded"
            );
            assert!(
                shell.pending_transcript_scroll_anchor.is_none(),
                "an anchor in the merged group is restored"
            );
            cx.notify();
        });
        cx.run_until_parked();
        let header = cx.debug_bounds("tool-group-older-header").unwrap();
        cx.simulate_click(header.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert_eq!(
                shell.transcript_rows.len(),
                1,
                "the merged group can still collapse"
            );
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn activity_groups_fit_a_narrow_pane_without_wrapping_the_count(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.transcript = vec![TranscriptItem {
                role: TranscriptRole::Assistant,
                text: String::new(),
                parts: [
                    "read",
                    "shell",
                    "edit",
                    "glob",
                    "browser_open",
                    "integration.unknown",
                ]
                .into_iter()
                .map(|name| activity_tool(name, name, ToolStatus::Completed))
                .collect(),
            }];
            shell.sync_markdown_cache();
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(420.), px(760.)));
        cx.run_until_parked();
        let header = cx.debug_bounds("tool-group-read-header").unwrap();
        let content = cx.debug_bounds("transcript-content-0").unwrap();
        assert_eq!(
            header.size.height,
            px(20.),
            "the activity count stays on the summary line"
        );
        assert!(header.left() >= content.left() && header.right() <= content.right());
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn forming_a_group_keeps_manually_opened_call_details(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        shell.update(cx, |shell, _| {
            shell.model.ui.transcript = vec![TranscriptItem {
                role: TranscriptRole::Assistant,
                text: String::new(),
                parts: vec![
                    activity_tool("shell", "a", ToolStatus::Completed),
                    activity_tool("shell", "b", ToolStatus::Running),
                ],
            }];
            shell.sync_markdown_cache();
            shell.expanded_chat_parts.insert("chat-part-0-0".into());
            let TranscriptPart::ToolCall { status, .. } =
                &mut shell.model.ui.transcript[0].parts[1]
            else {
                unreachable!()
            };
            *status = ToolStatus::Completed;
            shell.sync_markdown_cache();
            assert_eq!(
                shell.transcript_rows.len(),
                3,
                "grouping preserves the call the reader opened"
            );
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn completing_offscreen_tools_keeps_the_readers_position(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.transcript = vec![
                TranscriptItem {
                    role: TranscriptRole::Assistant,
                    text: "older context to keep reading ".repeat(300),
                    parts: Vec::new(),
                },
                TranscriptItem {
                    role: TranscriptRole::Assistant,
                    text: String::new(),
                    parts: vec![
                        activity_tool("shell", "first", ToolStatus::Completed),
                        activity_tool("shell", "second", ToolStatus::Running),
                    ],
                },
            ];
            shell.sync_markdown_cache();
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(680.), px(760.)));
        cx.run_until_parked();
        shell.update(cx, |shell, cx| {
            shell
                .transcript_list
                .set_follow_mode(gpui::FollowMode::Normal);
            shell.transcript_list.scroll_to(gpui::ListOffset {
                item_ix: 0,
                offset_in_item: px(30.),
            });
            shell.transcript_scrolled_away = true;
            cx.notify();
        });
        cx.run_until_parked();
        let before = cx.debug_bounds("markdown-inline-container-0-0").unwrap();
        shell.update(cx, |shell, cx| {
            let TranscriptPart::ToolCall { status, .. } =
                &mut shell.model.ui.transcript[1].parts[1]
            else {
                unreachable!()
            };
            *status = ToolStatus::Completed;
            shell.sync_markdown_cache();
            cx.notify();
        });
        cx.run_until_parked();
        let after = cx
            .debug_bounds("markdown-inline-container-0-0")
            .expect("the older context stays visible");
        assert_eq!(after.top(), before.top());
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn virtualized_paragraph_chunks_read_as_one_continuous_paragraph(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.transcript = vec![TranscriptItem {
                role: TranscriptRole::Assistant,
                text: "readable conversation ".repeat(60),
                parts: Vec::new(),
            }];
            shell.sync_markdown_cache();
            assert_eq!(shell.transcript_rows.len(), 2);
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(760.), px(1200.)));
        cx.run_until_parked();
        let first = cx.debug_bounds("markdown-inline-container-0-0").unwrap();
        let second = cx.debug_bounds("markdown-inline-container-0-1").unwrap();
        assert_eq!(second.top(), first.bottom());
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn turn_spacing_preserves_the_visible_text_when_history_is_prepended(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.transcript = vec![TranscriptItem {
                role: TranscriptRole::Assistant,
                text: "a longer response to read while older history loads ".repeat(90),
                parts: Vec::new(),
            }];
            shell.sync_markdown_cache();
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(680.), px(760.)));
        cx.run_until_parked();
        shell.update(cx, |shell, cx| {
            shell
                .transcript_list
                .set_follow_mode(gpui::FollowMode::Normal);
            shell.transcript_list.scroll_to(gpui::ListOffset {
                item_ix: 0,
                offset_in_item: px(30.),
            });
            cx.notify();
        });
        cx.run_until_parked();
        let before = cx.debug_bounds("markdown-inline-container-0-0").unwrap();
        shell.update(cx, |shell, cx| {
            shell.prepare_for_history_prepend(1);
            shell.model.ui.transcript.insert(
                0,
                TranscriptItem {
                    role: TranscriptRole::User,
                    text: "older question".into(),
                    parts: Vec::new(),
                },
            );
            shell.sync_markdown_cache();
            cx.notify();
        });
        cx.run_until_parked();
        let after = cx.debug_bounds("markdown-inline-container-1-0").unwrap();
        assert_eq!(after.top(), before.top());
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn new_tab_launcher_opens_closable_tool_tabs_without_changing_the_chat(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.simulate_resize(gpui::size(px(1440.), px(900.)));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = false;
            shell.layout.workbench_collapsed = false;
            shell.workbench_animation_id = 0;
            shell.model.ui.conversations = vec![navigation_conversation("a")];
            shell.model.ui.selected_session = Some("a".into());
            shell.model.ui.composer = "keep this follow-up".into();
            cx.notify();
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        let launcher = cx.debug_bounds("work-new-tab-page").unwrap();
        let pane = cx.debug_bounds("work-pane").unwrap();
        assert_eq!(launcher.bottom(), pane.bottom());
        let target = cx.debug_bounds("launch-changes").unwrap().center();
        cx.simulate_click(target, gpui::Modifiers::none());
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert_eq!(shell.work_tabs.active().kind, WorkTabKind::Changes)
        });
        let target = cx.debug_bounds("new-work-tab").unwrap().center();
        cx.simulate_click(target, gpui::Modifiers::none());
        cx.run_until_parked();
        let target = cx.debug_bounds("launch-terminal").unwrap().center();
        cx.simulate_click(target, gpui::Modifiers::none());
        cx.run_until_parked();
        let terminal = cx.debug_bounds("terminal-tab-content").unwrap();
        assert_eq!(terminal.bottom(), pane.bottom());
        assert_eq!(
            terminal.origin.y,
            cx.debug_bounds("work-header").unwrap().bottom()
        );
        shell.update(cx, |shell, _| {
            assert_eq!(shell.work_tabs.tabs.len(), 2);
            assert_eq!(shell.work_tabs.active().kind, WorkTabKind::Terminal);
        });
        let target = cx.debug_bounds("work-tab-0").unwrap().center();
        cx.simulate_click(target, gpui::Modifiers::none());
        cx.run_until_parked();
        let target = cx.debug_bounds("close-work-tab-1").unwrap().center();
        cx.simulate_click(target, gpui::Modifiers::none());
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert_eq!(shell.work_tabs.active().kind, WorkTabKind::Changes)
        });
        let target = cx.debug_bounds("close-work-tab-0").unwrap().center();
        cx.simulate_click(target, gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(cx.debug_bounds("work-new-tab-page").is_some());
        shell.update(cx, |shell, _| {
            assert_eq!(shell.work_tabs.tabs.len(), 1);
            assert_eq!(shell.model.ui.selected_session.as_deref(), Some("a"));
            assert_eq!(shell.model.ui.composer, "keep this follow-up");
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn browser_tab_selection_revokes_only_that_chats_previous_target(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.skip_drawing();
        let window =
            cx.add_window(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        let shell = window.root(cx).unwrap();
        let (sender, mut commands) = tokio::sync::mpsc::unbounded_channel();
        shell.update(cx, |shell, cx| {
            shell.command_tx = Some(sender);
            shell.connection_profile = Some(DesktopConnectionProfile::LocalWebSocket { url: "ws://127.0.0.1:25566".into() });
            shell.model.connection = crate::model::ShellConnection::Connected;
            shell.model.ui.selected_session = Some("a".into());
            shell.model.attached_session = Some("a".into());
            shell.browser_confirmed_sessions.extend(["a".into(), "b".into()]);
            let first = shell.work_tabs.active().id.clone();
            let second = shell.work_tabs.new_tab();
            shell.work_tabs.select(&first);
            shell.browser_url = "https://first.example/a-draft".into();
            let owner = shell.current_browser_owner().unwrap();
            let other = BrowserOwner { session_id: "b".into(), ..owner.clone() };
            shell.browser_registration_attempts.insert(owner.clone(), BrowserRegistration { page_instance: "page-a".into(), pending_replies: 0, retiring: false });
            shell.browser_registration_attempts.insert(other.clone(), BrowserRegistration { page_instance: "page-b".into(), pending_replies: 0, retiring: false });
            shell.select_work_tab(&second, cx);
            assert_eq!(shell.current_browser_owner().unwrap().tab_id, second);
            assert!(shell.browser_registration_attempts[&owner].retiring);
            assert!(shell.browser_registration_attempts.contains_key(&other));
            assert!(matches!(commands.try_recv().unwrap(), ClientMessage::UnregisterBrowserPage { session_id, page_instance } if session_id == "a" && page_instance == "page-a"));
            assert!(commands.try_recv().is_err());
            shell.complete_browser_registration("a", "page-a", None, cx);
            assert!(!shell.browser_registration_attempts.contains_key(&owner));
            shell.select_work_tab(&first, cx);
            assert_eq!(shell.browser_url, "https://first.example/a-draft");
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn workspace_panes_have_full_height_owners_and_aligned_headers(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.simulate_resize(gpui::size(px(1600.), px(900.)));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = false;
            shell.layout.workbench_collapsed = false;
            shell.layout.terminal_collapsed = true;
            shell.sidebar_animation_id = 0;
            shell.workbench_animation_id = 0;
            shell.model.ui.composer = "a draft that stays in the conversation".into();
            cx.notify();
        });
        cx.refresh().unwrap();
        cx.run_until_parked();

        let navigation = cx.debug_bounds("navigation-pane").unwrap();
        let conversation = cx.debug_bounds("conversation-pane").unwrap();
        let work = cx.debug_bounds("work-pane").unwrap();
        for (pane, header) in [
            (navigation, "navigation-header"),
            (conversation, "conversation-header"),
            (work, "work-header"),
        ] {
            let header = cx.debug_bounds(header).unwrap();
            assert_eq!(pane.origin.y, px(0.));
            assert_eq!(pane.size.height, px(900.));
            assert_eq!(header.origin, pane.origin);
            assert_eq!(header.size.width, pane.size.width);
            assert_eq!(header.size.height, px(38.));
        }
        let divider = cx.debug_bounds("work-divider").unwrap();
        assert_eq!(conversation.origin.x, navigation.right());
        assert_eq!(divider.origin.x, conversation.right());
        assert_eq!(divider.right(), work.origin.x);
        assert_eq!(divider.size.height, px(900.));
        assert_eq!(work.right(), px(1600.));
        assert!(work.size.width > conversation.size.width);
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn window_growth_goes_to_work_content_and_drag_resizes_the_conversation(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.simulate_resize(gpui::size(px(1440.), px(900.)));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = false;
            shell.layout.workbench_collapsed = false;
            shell.layout.terminal_collapsed = true;
            shell.sidebar_animation_id = 0;
            shell.workbench_animation_id = 0;
            cx.notify();
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        let conversation = cx.debug_bounds("conversation-pane").unwrap();
        let work = cx.debug_bounds("work-pane").unwrap();
        assert_eq!(conversation.size.width, px(480.));

        cx.simulate_resize(gpui::size(px(2240.), px(900.)));
        cx.run_until_parked();
        assert_eq!(
            cx.debug_bounds("conversation-pane").unwrap().size.width,
            conversation.size.width
        );
        assert_eq!(
            cx.debug_bounds("work-pane").unwrap().size.width,
            work.size.width + px(800.)
        );

        let grip = cx.debug_bounds("work-resize-grip").unwrap().center();
        cx.simulate_mouse_down(grip, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.simulate_mouse_move(
            grip + gpui::point(px(20.), px(0.)),
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.simulate_mouse_move(
            gpui::point(px(900.), grip.y),
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.simulate_mouse_up(
            gpui::point(px(900.), grip.y),
            gpui::MouseButton::Left,
            gpui::Modifiers::none(),
        );
        cx.run_until_parked();
        let resized = cx.debug_bounds("conversation-pane").unwrap();
        assert!(resized.size.width > conversation.size.width);
        assert_eq!(cx.debug_bounds("work-pane").unwrap().right(), px(2240.));
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn pane_toggles_keep_boundaries_flush_and_restore_the_conversation_width(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        cx.simulate_resize(gpui::size(px(1440.), px(900.)));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = false;
            shell.layout.workbench_collapsed = false;
            shell.layout.terminal_collapsed = true;
            shell.conversation_width = 540.;
            cx.notify();
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        shell.update(cx, |shell, cx| shell.toggle_workbench(cx));
        cx.run_until_parked();
        assert_eq!(
            cx.debug_bounds("conversation-pane").unwrap().right(),
            px(1440.)
        );
        shell.update(cx, |shell, cx| shell.toggle_workbench(cx));
        cx.run_until_parked();
        assert_eq!(
            cx.debug_bounds("conversation-pane").unwrap().size.width,
            px(540.)
        );
        shell.update(cx, |shell, cx| shell.toggle_sidebar(cx));
        cx.run_until_parked();
        assert_eq!(
            cx.debug_bounds("conversation-pane").unwrap().origin.x,
            px(0.)
        );
        shell.update(cx, |shell, cx| shell.toggle_sidebar(cx));
        cx.run_until_parked();
        let navigation = cx.debug_bounds("navigation-pane").unwrap();
        assert_eq!(
            cx.debug_bounds("conversation-pane").unwrap().origin.x,
            navigation.right()
        );
        assert_eq!(
            cx.debug_bounds("conversation-pane").unwrap().size.width,
            px(540.)
        );
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn work_tabs_switch_content_without_changing_the_chat(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.simulate_resize(gpui::size(px(1800.), px(900.)));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = false;
            shell.layout.workbench_collapsed = false;
            shell.layout.terminal_collapsed = true;
            shell.workbench_animation_id = 0;
            shell.work_tabs.open_tool(WorkTabKind::Browser);
            shell.work_tabs.open_tool(WorkTabKind::Changes);
            shell.work_tabs.open_tool(WorkTabKind::Files);
            shell.work_tabs.open_tool(WorkTabKind::Activity);
            shell.model.ui.composer = "retain this draft".into();
            cx.notify();
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        let browser = cx.debug_bounds("work-tab-0").unwrap();
        let changes = cx.debug_bounds("work-tab-1").unwrap();
        let files = cx.debug_bounds("work-tab-2").unwrap();
        let activity = cx.debug_bounds("work-tab-3").unwrap();
        for tab in [changes, files, activity] {
            assert_eq!(tab.origin.y, browser.origin.y);
        }
        assert!(browser.right() <= changes.origin.x);
        assert!(changes.right() <= files.origin.x);
        assert!(files.right() <= activity.origin.x);
        for (tab, view) in [
            (files, WorkTabKind::Files),
            (activity, WorkTabKind::Activity),
            (changes, WorkTabKind::Changes),
        ] {
            cx.simulate_click(tab.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            shell.update(cx, |shell, _| {
                assert_eq!(shell.work_tabs.active().kind, view);
                assert_eq!(shell.model.ui.composer, "retain this draft");
                assert!(shell.model.ui.selected_session.is_none());
            });
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn review_uses_the_work_pane_for_the_diff_and_adapts_its_file_list(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.simulate_resize(gpui::size(px(1800.), px(900.)));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = false;
            shell.layout.workbench_collapsed = false;
            shell.layout.terminal_collapsed = true;
            shell.layout.changes_expanded = true;
            shell.work_tabs.open_tool(WorkTabKind::Changes);
            shell.sidebar_animation_id = 0;
            shell.workbench_animation_id = 0;
            shell.review_diffs = vec![mew_diff::FileDiff::binary("image.png", None)];
            shell.select_review_file(0, cx);
            cx.notify();
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        let work = cx.debug_bounds("work-pane").unwrap();
        let diff = cx.debug_bounds("review-document").unwrap();
        let files = cx.debug_bounds("review-files").unwrap();
        assert_eq!(diff.origin.x, work.origin.x);
        assert_eq!(diff.right(), files.origin.x);
        assert_eq!(files.right(), work.right());
        assert_eq!(diff.origin.y, files.origin.y);
        assert_eq!(diff.bottom(), work.bottom());
        assert!(diff.size.width > files.size.width);

        shell.update(cx, |shell, cx| {
            shell.review_diffs = vec![mew_diff::diff_file(
                "src/main.rs",
                None,
                Some("old\n"),
                Some("new\n"),
            )];
            shell.select_review_file(0, cx);
        });
        cx.run_until_parked();
        let viewport = cx.debug_bounds("diff-viewport").unwrap();
        assert_eq!(viewport.origin, diff.origin);
        assert_eq!(viewport.size, diff.size);

        cx.simulate_resize(gpui::size(px(1200.), px(900.)));
        cx.run_until_parked();
        let work = cx.debug_bounds("work-pane").unwrap();
        let diff = cx.debug_bounds("review-document").unwrap();
        let files = cx.debug_bounds("review-files").unwrap();
        assert_eq!(files.origin.x, work.origin.x);
        assert_eq!(files.size.width, work.size.width);
        assert_eq!(files.bottom(), diff.origin.y);
        assert_eq!(diff.bottom(), work.bottom());
        assert!(diff.size.height > files.size.height);
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn terminal_tab_fills_work_content_and_remains_available_in_a_narrow_window(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.simulate_resize(gpui::size(px(1440.), px(900.)));
        shell.update(cx, |shell, cx| {
            shell.sidebar_width = SIDEBAR_MAX_WIDTH;
            shell.layout.sidebar_collapsed = false;
            shell.layout.workbench_collapsed = false;
            shell.layout.terminal_collapsed = false;
            shell.work_tabs.open_tool(WorkTabKind::Terminal);
            shell.sidebar_animation_id = 0;
            shell.workbench_animation_id = 0;
            cx.notify();
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        let terminal = cx.debug_bounds("terminal-tab-content").unwrap();
        let work = cx.debug_bounds("work-pane").unwrap();
        assert_eq!(terminal.origin.x, work.origin.x);
        assert_eq!(terminal.size.width, work.size.width);
        assert_eq!(terminal.bottom(), work.bottom());

        cx.simulate_resize(gpui::size(px(720.), px(900.)));
        cx.run_until_parked();
        shell.update(cx, |shell, cx| {
            assert!(shell.layout.workbench_collapsed);
            shell.workbench_animation_id = 0;
            cx.notify();
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        let conversation = cx.debug_bounds("conversation-pane").unwrap();
        let terminal = cx.debug_bounds("terminal-dock").unwrap();
        assert!(conversation.size.width >= px(CHAT_MIN_WIDTH));
        assert_eq!(terminal.origin.x, conversation.origin.x);
        assert_eq!(terminal.size.width, conversation.size.width);
        assert_eq!(terminal.bottom(), conversation.bottom());
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn sidebar_surface_continues_into_the_titlebar(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = false;
            shell.layout.workbench_collapsed = true;
            shell.layout.terminal_collapsed = true;
            cx.notify();
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        cx.update(|window, cx| {
            let shell = shell.read(cx);
            let sidebar: gpui::Hsla = theme_rgb(&shell.theme, "sidebar.background").into();
            let scale = window.scale_factor();
            assert!(
                window.painted_quads().iter().any(|quad| {
                    quad.background.as_solid() == Some(sidebar)
                        && quad.bounds.origin.x.as_f32() == 0.
                        && quad.bounds.origin.y.as_f32() == 0.
                        && quad.bounds.size.width.as_f32() == shell.sidebar_width * scale
                        && quad.bounds.size.height.as_f32() >= 38. * scale
                }),
                "the sidebar must share the window's top-left surface"
            );
        });
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn selecting_the_current_sidebar_chat_preserves_loaded_content(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        shell.update(cx, |shell, cx| {
            shell.model.ui.conversations = vec![navigation_conversation("a")];
            shell.model.ui.selected_session = Some("a".into());
            shell.model.attached_session = Some("a".into());
            shell.browser_title = "retained page".into();
            shell.history_before = Some(10);
            shell.attach_session("a".into(), cx);
            assert!(shell.model.session_is_ready());
            assert_eq!(shell.history_before, Some(10));
            assert_eq!(shell.browser_title, "retained page");
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn narrowing_the_window_collapses_the_workbench(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        shell.update(cx, |shell, _| {
            shell.layout.sidebar_collapsed = false;
            shell.layout.workbench_collapsed = false;
            shell.window_width = 1240.;
            assert!(shell.update_window_width(720.));
            assert!(shell.layout.workbench_collapsed);
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn sidebar_search_leaves_settings_and_reveals_navigation(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        shell.update(cx, |shell, _| {
            shell.settings_open = true;
            shell.layout.sidebar_collapsed = true;
        });
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                shell.action_focus_sidebar_search(&FocusSidebarSearch, window, cx);
                assert!(!shell.settings_open);
                assert!(!shell.layout.sidebar_collapsed);
                assert!(shell.sidebar_search_open);
                shell.quitting = true;
            });
        });
    }

    #[gpui::test]
    fn idle_workspace_chats_use_compact_sidebar_rows(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        shell.update(cx, |shell, cx| {
            let mut chat = navigation_conversation("a");
            chat.cwd = chat.workspace_path.clone();
            chat.last_message_at = Some(1);
            shell.model.ui.conversations = vec![chat];
            shell.collapsed_groups.clear();
            shell.layout.sidebar_collapsed = false;
            shell.rebuild_sidebar_rows();
            cx.notify();
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds("sidebar-chat-a")
            .expect("chat row must be visible");
        assert_eq!(bounds.size.height, px(30.));
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn selecting_a_chat_reveals_its_collapsed_workspace(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        shell.update(cx, |shell, cx| {
            shell.model.ui.conversations = vec![navigation_conversation("a")];
            shell.collapsed_groups.insert("/workspace/mew".into());
            shell.rebuild_sidebar_rows();
            assert!(!shell
                .sidebar_rows
                .iter()
                .any(|row| matches!(row, SidebarRow::Session(_))));
            shell.attach_session("a".into(), cx);
            assert!(shell
                .sidebar_rows
                .iter()
                .any(|row| { matches!(row, SidebarRow::Session(chat) if chat.session_id == "a") }));
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn selected_chat_highlight_spans_its_sidebar_row(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        shell.update(cx, |shell, cx| {
            let mut chat = navigation_conversation("a");
            chat.cwd = chat.workspace_path.clone();
            shell.model.ui.conversations = vec![chat];
            shell.model.ui.selected_session = Some("a".into());
            shell.collapsed_groups.clear();
            shell.layout.sidebar_collapsed = false;
            shell.rebuild_sidebar_rows();
            cx.notify();
        });
        cx.refresh().unwrap();
        cx.run_until_parked();
        let row = cx.debug_bounds("sidebar-chat-a").unwrap();
        cx.update(|window, cx| {
            let selected: gpui::Hsla = theme_rgb(&shell.read(cx).theme, "accent").into();
            let row = row.scale(window.scale_factor());
            assert!(
                window.painted_quads().iter().any(|quad| {
                    quad.background.as_solid() == Some(selected) && quad.bounds == row
                }),
                "selection must follow the complete rounded row"
            );
        });
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[test]
    fn frame_trace_percentile_uses_nearest_rank() {
        let mut samples = [
            Duration::from_millis(4),
            Duration::from_millis(1),
            Duration::from_millis(3),
            Duration::from_millis(2),
        ];

        assert_eq!(
            percentile_duration(&mut samples, 50),
            Duration::from_millis(2)
        );
        assert_eq!(
            percentile_duration(&mut samples, 95),
            Duration::from_millis(4)
        );
        assert_eq!(percentile_duration(&mut [], 95), Duration::ZERO);
    }

    #[test]
    fn browser_event_values_only_invalidate_when_changed() {
        let mut value = "https://example.com".to_owned();
        assert!(!replace_browser_event_value(
            &mut value,
            "https://example.com".to_owned()
        ));
        assert_eq!(value, "https://example.com");
        assert!(replace_browser_event_value(
            &mut value,
            "https://example.org".to_owned()
        ));
        assert_eq!(value, "https://example.org");
    }

    #[test]
    fn composer_offsets_round_trip_unicode() {
        let text = "a🙂b";
        assert_eq!(byte_offset_for_utf16(text, 0), 0);
        assert_eq!(byte_offset_for_utf16(text, 1), 1);
        assert_eq!(byte_offset_for_utf16(text, 3), 5);
        assert_eq!(byte_offset_for_utf16(text, 4), 6);
        assert_eq!(utf16_offset_for_byte(text, 5), 3);
        assert_eq!(utf16_offset_for_byte(text, 6), 4);
    }

    #[test]
    fn composer_deletion_moves_by_utf8_character_boundaries() {
        let text = "a🙂b";
        assert_eq!(previous_utf8_boundary(text, 0), 0);
        assert_eq!(previous_utf8_boundary(text, 5), 1);
        assert_eq!(previous_utf8_boundary(text, 6), 5);
        assert_eq!(next_utf8_boundary(text, 0), 1);
        assert_eq!(next_utf8_boundary(text, 1), 5);
        assert_eq!(next_utf8_boundary(text, 6), 6);

        let combining = "e\u{301}x";
        assert_eq!(previous_utf8_boundary(combining, combining.len()), 3);
        assert_eq!(next_utf8_boundary(combining, 0), 3);
    }

    #[test]
    fn transcript_selection_ranges_are_byte_safe() {
        let text = "a🙂b";
        assert_eq!(snap_to_char_boundary(text, 0), 0);
        assert_eq!(snap_to_char_boundary(text, 2), 1);
        assert_eq!(snap_to_char_boundary(text, 5), 5);
        assert_eq!(snap_to_char_boundary(text, text.len()), text.len());
    }

    #[test]
    fn transcript_selection_spans_rendered_blocks_in_both_directions() {
        let selection = TranscriptSelection {
            start: TranscriptSelectionPoint {
                message_index: 0,
                block_index: 0,
                offset: 2,
            },
            end: TranscriptSelectionPoint {
                message_index: 0,
                block_index: 2,
                offset: 3,
            },
        };
        assert_eq!(
            DesktopShell::selection_range_for(&selection, 0, 0, 5),
            Some(2..5)
        );
        assert_eq!(
            DesktopShell::selection_range_for(&selection, 0, 1, 4),
            Some(0..4)
        );
        assert_eq!(
            DesktopShell::selection_range_for(&selection, 0, 2, 8),
            Some(0..3)
        );

        let reversed = TranscriptSelection {
            start: selection.end,
            end: selection.start,
        };
        assert_eq!(
            DesktopShell::selection_range_for(&reversed, 0, 1, 4),
            Some(0..4)
        );
    }

    #[test]
    fn session_paths_are_displayed_relative_to_home() {
        let home = Path::new("/Users/natalie");
        assert_eq!(
            display_path_from_home("/Users/natalie/code/mew", Some(home)),
            "~/code/mew"
        );
        assert_eq!(display_path_from_home("/Users/natalie", Some(home)), "~");
        assert_eq!(display_path_from_home("/tmp/mew", Some(home)), "/tmp/mew");
    }

    #[test]
    fn session_times_use_compact_relative_labels() {
        let now = 1_700_000_000_000;
        assert_eq!(relative_session_time(Some(now), now), Some("now".into()));
        assert_eq!(
            relative_session_time(Some(now - 5 * 60_000), now),
            Some("5m".into())
        );
        assert_eq!(
            relative_session_time(Some(now - 3 * 3_600_000), now),
            Some("3h".into())
        );
        assert_eq!(
            relative_session_time(Some(now - 2 * 86_400_000), now),
            Some("2d".into())
        );
        assert_eq!(
            relative_session_time(Some(now - 14 * 86_400_000), now),
            Some("2w".into())
        );
        assert_eq!(relative_session_time(Some(0), now), None);
    }

    #[test]
    fn empty_picker_labels_use_actionable_fallbacks() {
        assert_eq!(non_empty_label(None, "choose a model"), "choose a model");
        assert_eq!(
            non_empty_label(Some(""), "choose a model"),
            "choose a model"
        );
        assert_eq!(
            non_empty_label(Some("  "), "choose a persona"),
            "choose a persona"
        );
        assert_eq!(
            non_empty_label(Some("gpt-5.2"), "choose a model"),
            "gpt-5.2"
        );
    }

    #[test]
    fn escape_dismisses_any_shell_popover_but_not_an_empty_state() {
        assert!(!escape_dismisses_shell_popover(
            "escape", false, false, false, false, false, false,
        ));
        assert!(escape_dismisses_shell_popover(
            "escape", true, false, false, false, false, false,
        ));
        assert!(escape_dismisses_shell_popover(
            "escape", false, true, false, false, false, false,
        ));
        assert!(escape_dismisses_shell_popover(
            "escape", false, false, true, false, false, false,
        ));
        assert!(escape_dismisses_shell_popover(
            "escape", false, false, false, true, false, false,
        ));
        assert!(escape_dismisses_shell_popover(
            "escape", false, false, false, false, true, false,
        ));
        assert!(escape_dismisses_shell_popover(
            "escape", false, false, false, false, false, true,
        ));
        assert!(!escape_dismisses_shell_popover(
            "enter", true, true, true, true, true, true,
        ));
    }

    #[test]
    fn composer_height_tracks_wrapped_lines_with_a_small_cap() {
        assert_eq!(composer_input_height(0, 20.), 48.);
        assert_eq!(composer_input_height(1, 20.), 48.);
        assert_eq!(composer_input_height(2, 20.), 48.);
        assert_eq!(composer_input_height(3, 20.), 60.);
        assert_eq!(composer_input_height(6, 20.), 96.);
    }

    #[test]
    fn picker_popup_sits_above_the_trigger_with_a_small_gap() {
        let trigger = Bounds::new(point(px(120.), px(400.)), gpui::size(px(200.), px(40.)));
        assert_eq!(
            picker_popup_position_in_window(trigger, px(288.), px(700.), px(8.), px(8.)),
            point(px(120.), px(104.))
        );
    }

    #[test]
    fn pending_actions_anchor_to_the_outer_composer_box() {
        let composer = Bounds::new(point(px(120.), px(400.)), gpui::size(px(200.), px(80.)));
        assert_eq!(pending_actions_anchor(composer), point(px(108.), px(380.)));
        assert_eq!(pending_actions_width(composer), px(224.));
    }

    #[test]
    fn picker_popup_flips_below_a_trigger_near_the_top_edge() {
        let trigger = Bounds::new(point(px(120.), px(20.)), gpui::size(px(200.), px(40.)));
        assert_eq!(
            picker_popup_position_in_window(trigger, px(160.), px(700.), px(8.), px(8.)),
            point(px(120.), px(68.))
        );
    }

    #[test]
    fn picker_popup_clamps_when_it_cannot_fit_in_the_window() {
        let trigger = Bounds::new(point(px(120.), px(20.)), gpui::size(px(200.), px(40.)));
        assert_eq!(
            picker_popup_position_in_window(trigger, px(240.), px(120.), px(8.), px(8.)),
            point(px(120.), px(8.))
        );
    }

    #[test]
    fn right_aligned_picker_stays_put_when_the_trigger_width_changes() {
        let wide_trigger = Bounds::new(point(px(500.), px(300.)), gpui::size(px(120.), px(40.)));
        let narrow_trigger = Bounds::new(point(px(560.), px(300.)), gpui::size(px(60.), px(40.)));
        let wide_position = picker_popup_position_in_window_right_aligned(
            wide_trigger,
            px(300.),
            px(160.),
            px(1000.),
            px(700.),
            px(8.),
            px(8.),
        );
        let narrow_position = picker_popup_position_in_window_right_aligned(
            narrow_trigger,
            px(300.),
            px(160.),
            px(1000.),
            px(700.),
            px(8.),
            px(8.),
        );
        assert_eq!(wide_position, narrow_position);
        assert_eq!(wide_position, point(px(320.), px(132.)));
    }

    #[test]
    fn picker_viewports_match_and_stay_bounded() {
        assert_eq!(model_picker_list_height(0), px(64.));
        assert_eq!(model_picker_list_height(5), px(320.));
        assert_eq!(model_picker_list_height(32), px(320.));
        assert_eq!(model_picker_height(32, 0), px(372.));
        assert_eq!(model_picker_height(32, 3), px(444.));
        assert_eq!(effort_index_for_pointer(0., 200., 3), Some(0));
        assert_eq!(effort_index_for_pointer(100., 200., 3), Some(1));
        assert_eq!(effort_index_for_pointer(200., 200., 3), Some(2));
        assert_eq!(effort_index_for_pointer(80., 0., 3), None);
        assert_eq!(effort_position_for_pointer(10., 200., 3), Some(0.));
        assert_eq!(effort_position_for_pointer(33.333, 200., 3), Some(0.));
        assert_eq!(effort_position_for_pointer(100., 200., 3), Some(0.5));
        assert_eq!(effort_position_for_pointer(200., 200., 3), Some(1.));
        assert!((effort_track_inset(200., 3) - 33.333_332).abs() < 0.001);
        assert!((effort_track_inset(200., 7) - 14.285_714).abs() < 0.001);
        assert!((effort_track_content_width(200., 7) - 171.428_57).abs() < 0.001);
        assert!((effort_stop_offset(0, 200., 3).unwrap() - 33.333_332).abs() < 0.001);
        assert!((effort_stop_offset(1, 200., 3).unwrap() - 100.).abs() < 0.001);
        assert!((effort_stop_offset(2, 200., 3).unwrap() - 166.666_67).abs() < 0.001);
        assert_eq!(effort_stop_offset(3, 200., 3), None);
        assert_eq!(effort_stop_offset(0, 0., 3), None);
        assert!((effort_stop_slot_width(200., 3) - 66.666_664).abs() < 0.001);
        assert!((effort_stop_slot_width(200., 1) - 200.).abs() < 0.001);
        assert_eq!(effort_stop_slot_width(0., 3), 0.);
        assert_eq!(effort_position_for_index(0, 7), 0.);
        assert_eq!(effort_position_for_index(3, 7), 0.5);
        assert_eq!(effort_position_for_index(6, 7), 1.);
        assert!(!thinking_effort_drag_is_active(None, true));
        assert!(!thinking_effort_drag_is_active(Some(0.5), false));
        assert!(thinking_effort_drag_is_active(Some(0.5), true));
        assert_eq!(effort_index_for_position(0.5, 3), Some(1));
        assert_eq!(effort_position_for_index(2, 3), 1.);
        assert_eq!(persona_picker_list_height(2), px(176.));
        assert_eq!(persona_picker_list_height(32), px(440.));
    }

    #[test]
    fn model_picker_query_matches_ids_and_descriptions() {
        assert!(model_matches_query(
            "openai/gpt-5",
            "openai",
            "gpt-5",
            Some("fast coding model"),
            "coding"
        ));
        assert!(model_matches_query(
            "anthropic/claude",
            "anthropic",
            "claude",
            None,
            "ANTHROPIC"
        ));
        assert!(!model_matches_query(
            "openai/gpt-5",
            "openai",
            "gpt-5",
            Some("fast coding model"),
            "vision"
        ));
    }

    #[test]
    fn recent_model_indices_preserve_saved_order_and_ignore_unknown_duplicates() {
        let models = vec![
            mew_protocol::ModelInfo {
                id: "openai/gpt-5".into(),
                provider: "openai".into(),
                model: "gpt-5".into(),
                description: None,
                thinking_variants: Vec::new(),
                thinking_budget: None,
                context_window: None,
            },
            mew_protocol::ModelInfo {
                id: "anthropic/claude".into(),
                provider: "anthropic".into(),
                model: "claude".into(),
                description: None,
                thinking_variants: Vec::new(),
                thinking_budget: None,
                context_window: None,
            },
            mew_protocol::ModelInfo {
                id: "z-ai/glm".into(),
                provider: "z-ai".into(),
                model: "glm".into(),
                description: None,
                thinking_variants: Vec::new(),
                thinking_budget: None,
                context_window: None,
            },
        ];
        let recent = vec![
            "missing/model".into(),
            "anthropic/claude".into(),
            "openai/gpt-5".into(),
            "anthropic/claude".into(),
            "z-ai/glm".into(),
        ];

        assert_eq!(recent_model_indices(&models, &recent), vec![1, 0, 2]);
    }

    #[test]
    fn model_picker_rows_put_recent_models_before_all_models() {
        let models = vec![
            mew_protocol::ModelInfo {
                id: "openai/gpt-5".into(),
                provider: "openai".into(),
                model: "gpt-5".into(),
                description: None,
                thinking_variants: Vec::new(),
                thinking_budget: None,
                context_window: None,
            },
            mew_protocol::ModelInfo {
                id: "anthropic/claude".into(),
                provider: "anthropic".into(),
                model: "claude".into(),
                description: None,
                thinking_variants: Vec::new(),
                thinking_budget: None,
                context_window: None,
            },
        ];

        let (selectable, rows) = build_model_picker_rows(&models, &["anthropic/claude".into()], "");

        assert_eq!(
            selectable,
            vec![1, 0, 1],
            "recent rows should be selected before the all-model rows"
        );
        assert_eq!(
            rows,
            vec![
                ModelPickerRow::Header("Recent"),
                ModelPickerRow::Model {
                    index: 1,
                    recent: true,
                },
                ModelPickerRow::Header("All Models"),
                ModelPickerRow::Model {
                    index: 0,
                    recent: false,
                },
                ModelPickerRow::Model {
                    index: 1,
                    recent: false,
                },
            ]
        );
    }

    #[test]
    fn model_picker_rows_hide_recent_section_while_searching() {
        let models = vec![mew_protocol::ModelInfo {
            id: "openai/gpt-5".into(),
            provider: "openai".into(),
            model: "gpt-5".into(),
            description: None,
            thinking_variants: Vec::new(),
            thinking_budget: None,
            context_window: None,
        }];
        let (selectable, rows) =
            build_model_picker_rows(&models, &["openai/gpt-5".into()], "anthropic");

        assert!(selectable.is_empty());
        assert!(rows.is_empty());
    }

    #[test]
    fn pushing_recent_model_moves_it_to_the_front_and_caps_the_list() {
        let mut recent = ["a", "b", "c", "d", "e", "f"]
            .into_iter()
            .map(String::from)
            .collect();

        push_recent_model(&mut recent, "g");
        assert_eq!(recent, ["g", "a", "b", "c", "d", "e"]);

        push_recent_model(&mut recent, "c");
        assert_eq!(recent, ["c", "g", "a", "b", "d", "e"]);
    }

    #[test]
    fn transcript_animation_is_limited_to_the_last_visible_row() {
        assert!(!should_animate_transcript_row(0, 0));
        assert!(should_animate_transcript_row(0, 1));
        assert!(!should_animate_transcript_row(0, 2));
        assert!(should_animate_transcript_row(1, 2));
    }

    #[test]
    fn markdown_cache_reuses_equal_content_across_allocations() {
        let cached = CachedMarkdown {
            source: "same content".into(),
            source_identity: 1,
            source_len: 12,
            render_blocks: Vec::new(),
            scroll_groups: Vec::new(),
            streaming: None,
            completed_tool: false,
        };
        let fresh = String::from("same content");
        assert!(cached_markdown_source_unchanged(
            &cached,
            &fresh,
            fresh.as_ptr() as usize + 1,
        ));
        assert!(!cached_markdown_source_unchanged(&cached, "different!", 1,));
    }

    #[test]
    fn selected_session_path_uses_the_session_cwd() {
        let home = PathBuf::from(std::env::var_os("HOME").expect("session path test needs HOME"));
        let conversations = [ConversationItem {
            session_id: "session-1".into(),
            title: "A session".into(),
            cwd: Some(home.join("code/mew").to_string_lossy().into_owned()),
            workspace_path: Some(home.join("code/other").to_string_lossy().into_owned()),
            last_message_at: None,
            state: mew_protocol::SessionState::Idle,
            last_turn_failed: false,
            failure_detail: None,
            unread_completion: false,
            needs_attention: false,
            archived: false,
            pinned: false,
            group_id: None,
        }];

        assert_eq!(
            selected_session_path(&conversations, Some("session-1")).as_deref(),
            Some("~/code/mew")
        );
        assert_eq!(selected_session_path(&conversations, Some("missing")), None);
    }

    #[test]
    fn ungrouped_section_only_appears_alongside_named_groups() {
        assert!(!show_ungrouped_group(0, 4));
        assert!(!show_ungrouped_group(2, 0));
        assert!(show_ungrouped_group(2, 4));
    }

    #[test]
    fn session_titles_are_normalized_and_capped_with_an_ellipsis() {
        assert_eq!(compact_session_title("  short   title "), "short title");

        let title = compact_session_title(
            "The user asked about a repository and the requested changes are extensive",
        );
        assert_eq!(title.chars().count(), 29);
        assert!(title.ends_with('…'));
    }

    #[test]
    fn latest_user_prompt_skips_empty_and_assistant_messages() {
        let transcript = vec![
            TranscriptItem {
                role: TranscriptRole::User,
                text: "first".into(),
                parts: Vec::new(),
            },
            TranscriptItem {
                role: TranscriptRole::Assistant,
                text: "answer".into(),
                parts: Vec::new(),
            },
            TranscriptItem {
                role: TranscriptRole::User,
                text: "  second prompt  ".into(),
                parts: Vec::new(),
            },
        ];

        assert_eq!(
            latest_user_prompt(&transcript).as_deref(),
            Some("second prompt")
        );
    }

    #[test]
    fn latest_user_submission_retains_file_attachments() {
        let transcript = vec![TranscriptItem {
            role: TranscriptRole::User,
            text: "review this".into(),
            parts: vec![TranscriptPart::File {
                label: "screenshot.png".into(),
                attachment: Attachment {
                    path: "/tmp/screenshot.png".into(),
                    mime: Some("image/png".into()),
                },
            }],
        }];

        let submission = latest_user_submission(&transcript).expect("submission should exist");

        assert_eq!(submission.text, "review this");
        assert_eq!(submission.attachments[0].path, "/tmp/screenshot.png");
        assert_eq!(submission.attachments[0].mime.as_deref(), Some("image/png"));
    }

    #[test]
    fn transcript_attention_only_describes_the_active_turn() {
        assert_eq!(
            transcript_attention(true, true, false, false),
            Some(TranscriptAttention::Running)
        );
        assert_eq!(
            transcript_attention(true, false, true, false),
            Some(TranscriptAttention::Failed)
        );
        assert_eq!(
            transcript_attention(true, false, false, false),
            Some(TranscriptAttention::Waiting)
        );
        assert_eq!(transcript_attention(false, true, false, false), None);
    }

    #[test]
    fn pending_actions_replace_generic_working_status() {
        assert_eq!(transcript_attention(true, true, false, true), None);
        assert_eq!(transcript_attention(true, false, true, true), None);
    }

    #[test]
    fn failed_partial_assistant_turn_has_retry_attention() {
        assert_eq!(
            transcript_attention(false, false, true, false),
            Some(TranscriptAttention::Failed)
        );
        assert_eq!(transcript_attention(false, true, true, false), None);
        assert_eq!(transcript_attention(false, false, true, true), None);
    }

    #[test]
    fn message_and_action_events_request_full_client_snapshots() {
        assert!(client_event_requires_transcript_snapshot(
            &ClientEvent::SessionReady {
                session_id: "session".into(),
            }
        ));
        assert!(!client_event_requires_transcript_snapshot(
            &ClientEvent::SessionListChanged
        ));
        assert!(!client_event_requires_metadata_sync(
            &ClientEvent::PermissionModeChanged {
                mode: "permissive".into(),
            }
        ));
        assert!(client_event_requires_transcript_snapshot(
            &ClientEvent::RequiredActionChanged {
                session_id: "session".into(),
                request_id: "request".into(),
            }
        ));
        assert!(client_event_requires_transcript_remeasure(
            &ClientEvent::MessageChanged {
                session_id: "session".into(),
            }
        ));
        assert!(!client_event_requires_transcript_remeasure(
            &ClientEvent::RequiredActionChanged {
                session_id: "session".into(),
                request_id: "request".into(),
            }
        ));
    }

    #[test]
    fn activity_events_request_full_session_snapshots() {
        assert!(client_event_requires_session_snapshot(
            &ClientEvent::ActivitySnapshot {
                session_id: "session".into()
            }
        ));
        assert!(client_event_requires_transcript_snapshot(
            &ClientEvent::ProviderMessageEnded {
                session_id: "session".into(),
                cost: 0.0,
                input_tokens: 0,
                output_tokens: 0,
            }
        ));
        assert!(client_event_requires_session_snapshot(
            &ClientEvent::SessionActivityChanged {
                session_id: "session".into(),
                running: true,
            }
        ));
        assert!(client_event_requires_session_snapshot(
            &ClientEvent::ProviderMessageEnded {
                session_id: "session".into(),
                cost: 0.0,
                input_tokens: 0,
                output_tokens: 0,
            }
        ));
        assert!(client_event_requires_session_snapshot(
            &ClientEvent::SubagentsChanged {
                session_id: "session".into(),
            }
        ));
        assert!(client_event_requires_session_snapshot(
            &ClientEvent::TodosChanged {
                session_id: "session".into(),
            }
        ));
        assert!(client_event_requires_session_snapshot(
            &ClientEvent::UsageChanged {
                session_id: "session".into(),
            }
        ));
        assert!(!client_event_requires_session_snapshot(
            &ClientEvent::SessionListChanged
        ));
        // Activity events refresh the workbench, not the transcript.
        assert!(!client_event_requires_transcript_snapshot(
            &ClientEvent::UsageChanged {
                session_id: "session".into(),
            }
        ));
    }

    #[test]
    fn usage_labels_compact_token_counts() {
        assert_eq!(format_token_count(340), "340");
        assert_eq!(format_token_count(1_200), "1.2k");
        assert_eq!(format_token_count(2_500_000), "2.50M");
        assert_eq!(usage_summary_label(UsageSummary::default()), None);
        assert_eq!(
            usage_summary_label(UsageSummary {
                input_tokens: 1_200,
                output_tokens: 340,
                cost: 0.0123,
                turns: 2,
            }),
            Some("1.2k in · 340 out · $0.0123".to_owned())
        );
    }

    #[test]
    fn expanded_reasoning_uses_one_virtualized_row_per_block() {
        let cached = CachedMarkdown {
            source: "reasoning".into(),
            source_identity: 0,
            source_len: 9,
            render_blocks: vec![
                MarkdownRenderBlock {
                    block: MarkdownBlock::Paragraph(InlineText {
                        text: "one".into(),
                        highlights: Vec::new(),
                        links: Vec::new(),
                    }),
                    continuation: false,
                    syntax_highlights: Vec::new(),
                };
                3
            ],
            scroll_groups: Vec::new(),
            streaming: None,
            completed_tool: false,
        };
        let reasoning = TranscriptPart::Reasoning("one\ntwo\nthree".into());
        assert_eq!(
            transcript_part_block_count(Some(&reasoning), &cached, false),
            1
        );
        assert_eq!(
            transcript_part_block_count(Some(&reasoning), &cached, true),
            3
        );
    }

    #[test]
    fn streaming_and_terminal_events_skip_expensive_metadata_sync() {
        assert!(!client_event_requires_metadata_sync(
            &ClientEvent::TextDelta {
                session_id: "session".into(),
                part_id: Default::default(),
                delta: "chunk".into(),
            }
        ));
        assert!(!client_event_requires_metadata_sync(
            &ClientEvent::TerminalOutput {
                terminal_id: "terminal".into(),
                bytes: b"output".to_vec(),
            }
        ));
        assert!(!client_event_requires_metadata_sync(
            &ClientEvent::ToolProgress {
                session_id: "session".into(),
                call_id: "call".into(),
                chunk: "chunk".into(),
            }
        ));
        assert!(client_event_requires_metadata_sync(
            &ClientEvent::SessionListChanged
        ));
        assert!(client_event_requires_metadata_sync(
            &ClientEvent::RequiredActionChanged {
                session_id: "session".into(),
                request_id: "request".into(),
            }
        ));
    }

    #[test]
    fn streaming_batches_are_coalesced_but_mixed_batches_render_immediately() {
        let streaming = vec![
            ClientEvent::TextDelta {
                session_id: "session".into(),
                part_id: Default::default(),
                delta: "chunk".into(),
            },
            ClientEvent::ToolProgress {
                session_id: "session".into(),
                call_id: "call".into(),
                chunk: "output".into(),
            },
        ];
        assert!(client_events_are_streaming_only(&streaming));

        let mut mixed = streaming;
        mixed.push(ClientEvent::SessionMetaChanged {
            session_id: "session".into(),
        });
        assert!(!client_events_are_streaming_only(&mixed));
    }

    #[test]
    fn terminal_output_batches_do_not_invalidate_the_shell() {
        assert!(client_events_require_shell_render(&[]));
        assert!(!client_events_require_shell_render(&[
            ClientEvent::TerminalOutput {
                terminal_id: "terminal".into(),
                bytes: b"output".to_vec(),
            },
        ]));
        assert!(client_events_require_shell_render(&[
            ClientEvent::TerminalOpened {
                terminal_id: "terminal".into(),
            },
        ]));
        assert!(client_events_require_shell_render(&[
            ClientEvent::TerminalOutput {
                terminal_id: "terminal".into(),
                bytes: b"output".to_vec(),
            },
            ClientEvent::TextDelta {
                session_id: "session".into(),
                part_id: Default::default(),
                delta: "text".into(),
            },
        ]));
    }

    #[test]
    fn sidebar_transition_reaches_both_panel_widths() {
        assert_eq!(SIDEBAR_COLLAPSED_WIDTH, 0.);
        assert_eq!(sidebar_transition_width(true, 0.0, 312.), 312.);
        assert_eq!(
            sidebar_transition_width(true, 1.0, 312.),
            SIDEBAR_COLLAPSED_WIDTH
        );
        assert_eq!(
            sidebar_transition_width(false, 0.0, 312.),
            SIDEBAR_COLLAPSED_WIDTH
        );
        assert_eq!(sidebar_transition_width(false, 1.0, 312.), 312.);
    }

    #[test]
    fn sidebar_width_stays_within_rail_bounds() {
        assert_eq!(sidebar_width_from_pointer(0., 1440.), SIDEBAR_MIN_WIDTH);
        assert_eq!(
            sidebar_width_from_pointer(SIDEBAR_MAX_WIDTH + 80., 1440.),
            SIDEBAR_MAX_WIDTH
        );
        assert_eq!(sidebar_width_from_pointer(312., 1440.), 312.);
        assert_eq!(sidebar_width_from_pointer(360., 720.), 300.);
    }

    #[test]
    fn workbench_transition_reaches_both_panel_widths() {
        assert_eq!(WORKBENCH_COLLAPSED_WIDTH, 0.);
        assert_eq!(workbench_transition_width(true, 0.0, 712.), 712.);
        assert_eq!(
            workbench_transition_width(true, 1.0, 712.),
            WORKBENCH_COLLAPSED_WIDTH
        );
        assert_eq!(
            workbench_transition_width(false, 0.0, 712.),
            WORKBENCH_COLLAPSED_WIDTH
        );
        assert_eq!(workbench_transition_width(false, 1.0, 712.), 712.);
    }

    #[test]
    fn workbench_resize_stays_within_usable_width_bounds() {
        assert_eq!(workbench_max_width(1440., SIDEBAR_EXPANDED_WIDTH), 755.);
        assert_eq!(
            conversation_width_from_pointer(1440., 1072., SIDEBAR_EXPANDED_WIDTH),
            808.
        );
        assert_eq!(
            conversation_width_from_pointer(1440., 0., SIDEBAR_EXPANDED_WIDTH),
            CHAT_MIN_WIDTH
        );
        assert_eq!(
            conversation_width_from_pointer(1440., 1400., SIDEBAR_EXPANDED_WIDTH),
            815.
        );
    }

    #[test]
    fn workbench_resize_respects_the_available_width_on_narrow_windows() {
        assert_eq!(workbench_max_width(720., SIDEBAR_EXPANDED_WIDTH), 35.);
        assert_eq!(
            conversation_width_from_pointer(720., 500., SIDEBAR_EXPANDED_WIDTH),
            CHAT_MIN_WIDTH
        );
    }

    #[test]
    fn workbench_only_opens_when_the_chat_can_keep_its_minimum_width() {
        assert!(!workbench_fits_window(720., SIDEBAR_EXPANDED_WIDTH));
        assert!(workbench_fits_window(1_240., SIDEBAR_EXPANDED_WIDTH));
        assert!(!workbench_fits_window(780., SIDEBAR_COLLAPSED_WIDTH));
        assert!(workbench_fits_window(781., SIDEBAR_COLLAPSED_WIDTH));
    }

    #[test]
    fn transcript_scroll_state_only_marks_a_list_scrolled_away_when_needed() {
        assert!(!transcript_is_scrolled_away(false, 0, 10, false));
        assert!(!transcript_is_scrolled_away(true, 10, 10, false));
        assert!(!transcript_is_scrolled_away(true, 4, 10, true));
        assert!(transcript_is_scrolled_away(true, 4, 10, false));
    }

    #[test]
    fn shell_layout_round_trips_through_shared_state() {
        let mut state = mew_config::State::default();
        state.sidebar_collapsed.insert("shell.sidebar".into(), true);
        state.sidebar_collapsed.insert("shell.changes".into(), true);
        let layout = ShellLayoutState::from_state(&state);
        assert!(layout.sidebar_collapsed);
        assert!(!layout.changes_expanded);

        let mut saved = mew_config::State::default();
        layout.write_state(&mut saved);
        assert_eq!(saved.sidebar_collapsed.get("shell.sidebar"), Some(&true));
        assert_eq!(saved.sidebar_collapsed.get("shell.changes"), Some(&true));
    }

    #[test]
    fn settings_pages_have_stable_navigation_metadata() {
        let pages = [
            SettingsPage::General,
            SettingsPage::Terminal,
            SettingsPage::Workspace,
            SettingsPage::Connection,
        ];
        let keys = pages.map(SettingsPage::key);
        assert_eq!(keys, ["general", "terminal", "workspace", "connection"]);
        assert_eq!(SettingsPage::Terminal.title(), "Terminal");
        assert_eq!(
            SettingsPage::Connection.description(),
            "daemon and remote workspace status"
        );
    }

    #[test]
    fn browser_rect_converts_gpui_coordinates_to_appkit_coordinates() {
        let rect = browser_native_rect(
            Bounds::new(point(px(20.), px(100.)), gpui::size(px(300.), px(200.))),
            gpui::size(px(800.), px(600.)),
            true,
        );
        assert_eq!(rect.x, 20.);
        assert_eq!(rect.y, 300.);
        assert_eq!(rect.width, 300.);
        assert_eq!(rect.height, 200.);
        assert!(rect.visible);
    }

    #[test]
    fn browser_initialization_latches_failures_until_retry() {
        assert!(browser_initialization_is_needed(true, false, false, false));
        assert!(!browser_initialization_is_needed(true, true, false, false));
        assert!(!browser_initialization_is_needed(true, false, true, false));
        assert!(!browser_initialization_is_needed(true, false, false, true));
        assert!(!browser_initialization_is_needed(
            false, false, false, false
        ));
    }

    #[test]
    fn browser_pages_require_a_confirmed_loopback_profile() {
        for url in [
            "ws://127.0.0.1:25566",
            "ws://[::1]:25566",
            "ws://localhost:25566",
        ] {
            let profile = DesktopConnectionProfile::LocalWebSocket { url: url.into() };
            assert!(browser_profile_is_local(&profile));
        }
        for url in [
            "ws://192.0.2.1:25566",
            "ws://127.0.0.1.evil:25566",
            "ws://user@127.0.0.1:25566",
            "ws://localhost:0",
        ] {
            let profile = DesktopConnectionProfile::LocalWebSocket { url: url.into() };
            assert!(!browser_profile_is_local(&profile));
        }
        let profile = DesktopConnectionProfile::RemoteIroh {
            node_id: "remote".into(),
            pairing_token: None,
            device_name: "test".into(),
        };
        assert!(!browser_profile_is_local(&profile));
    }

    #[test]
    fn deferred_browser_creation_rejects_replaced_or_disconnected_owners() {
        let owner = BrowserOwner {
            connection_profile: "ws://127.0.0.1:25566".into(),
            session_id: "a".into(),
            tab_id: "browser".into(),
        };
        let first = BrowserPage {
            owner: owner.clone(),
            page_instance: "first".into(),
        };
        let replacement = BrowserPage {
            owner,
            page_instance: "replacement".into(),
        };
        assert!(browser_creation_is_current(
            Some(&first),
            &first,
            true,
            false
        ));
        assert!(!browser_creation_is_current(
            Some(&replacement),
            &first,
            true,
            false
        ));
        assert!(!browser_creation_is_current(None, &first, true, false));
        assert!(!browser_creation_is_current(
            Some(&first),
            &first,
            false,
            false
        ));
        assert!(!browser_creation_is_current(
            Some(&first),
            &first,
            true,
            true
        ));
    }

    #[test]
    fn hidden_browser_pump_survives_until_runtime_exit() {
        assert!(browser_pump_is_needed(true, false));
        assert!(!browser_pump_is_needed(false, false));
        assert!(!browser_pump_is_needed(true, true));
    }

    #[test]
    fn browser_view_state_allows_the_default_and_web_urls() {
        assert!(browser_url_is_navigable("about:blank"));
        assert!(browser_url_is_navigable("https://example.com"));
        assert!(browser_url_is_navigable("http://127.0.0.1:3000"));
        assert!(!browser_url_is_navigable("file:///tmp/page.html"));
        assert!(!browser_url_is_navigable("javascript:alert(1)"));
    }

    #[test]
    fn browser_urls_are_normalized_without_allowing_other_schemes() {
        assert_eq!(
            normalize_browser_url("example.com/docs").as_deref(),
            Some("https://example.com/docs")
        );
        assert_eq!(
            normalize_browser_url("  http://localhost:3000  ").as_deref(),
            Some("http://localhost:3000")
        );
        assert_eq!(normalize_browser_url("javascript:alert(1)"), None);
        assert_eq!(normalize_browser_url("https://example.com/a b"), None);
    }

    #[test]
    fn desktop_theme_mode_selects_the_configured_variant() {
        assert_eq!(
            desktop_theme_name(
                DesktopThemeMode::System,
                WindowAppearance::Light,
                "light-custom",
                "dark-custom"
            ),
            "light-custom"
        );
        assert_eq!(
            desktop_theme_name(
                DesktopThemeMode::System,
                WindowAppearance::Dark,
                "light-custom",
                "dark-custom"
            ),
            "dark-custom"
        );
        assert_eq!(
            desktop_theme_name(
                DesktopThemeMode::Light,
                WindowAppearance::Dark,
                "light-custom",
                "dark-custom"
            ),
            "light-custom"
        );
        assert_eq!(
            desktop_theme_name(
                DesktopThemeMode::Dark,
                WindowAppearance::Light,
                "light-custom",
                "dark-custom"
            ),
            "dark-custom"
        );
    }

    #[test]
    fn attachment_mime_is_case_insensitive_and_conservative() {
        assert_eq!(attachment_mime(Path::new("screen.PNG")), Some("image/png"));
        assert_eq!(
            attachment_mime(Path::new("report.pdf")),
            Some("application/pdf")
        );
        assert_eq!(attachment_mime(Path::new("archive.bin")), None);
        assert_eq!(attachment_mime(Path::new("no-extension")), None);
    }

    #[test]
    fn shell_shortcuts_require_platform_modifier() {
        assert_eq!(shell_command_for_key("b", false), None);
        assert_eq!(
            shell_command_for_key("b", true),
            Some(ShellCommand::ToggleSidebar)
        );
        assert_eq!(
            shell_command_for_key("j", true),
            Some(ShellCommand::ToggleTerminal)
        );
        assert_eq!(
            shell_command_for_key("n", true),
            Some(ShellCommand::NewConversation)
        );
        assert_eq!(
            shell_command_for_key("w", true),
            Some(ShellCommand::CloseConversation)
        );
        assert_eq!(shell_command_for_key("2", true), None);
        assert_eq!(shell_command_for_key("0", true), None);
    }

    #[test]
    fn revealing_workbench_preserves_sidebar_when_it_fits() {
        let mut layout = ShellLayoutState {
            workbench_collapsed: true,
            ..Default::default()
        };
        assert!(layout.reveal_workbench(1240., SIDEBAR_EXPANDED_WIDTH));
        assert!(!layout.sidebar_collapsed);
        assert!(!layout.workbench_collapsed);
    }

    #[test]
    fn revealing_workbench_folds_sidebar_only_when_needed() {
        let mut layout = ShellLayoutState {
            workbench_collapsed: true,
            ..Default::default()
        };
        assert!(layout.reveal_workbench(900., SIDEBAR_EXPANDED_WIDTH));
        assert!(layout.sidebar_collapsed);
        assert!(!layout.workbench_collapsed);
        let mut layout = ShellLayoutState {
            workbench_collapsed: true,
            ..Default::default()
        };
        let original = layout;
        assert!(!layout.reveal_workbench(
            CHAT_MIN_WIDTH + WORKBENCH_MIN_WIDTH + PANE_DIVIDER_WIDTH - 1.,
            SIDEBAR_EXPANDED_WIDTH
        ));
        assert_eq!(layout, original);
        assert!(layout.reveal_workbench(
            CHAT_MIN_WIDTH + WORKBENCH_MIN_WIDTH + PANE_DIVIDER_WIDTH,
            SIDEBAR_EXPANDED_WIDTH
        ));
    }

    #[test]
    fn metadata_only_updates_are_synced_without_display_events() {
        assert!(client_update_requires_metadata_sync(&[]));
        assert!(!client_update_requires_metadata_sync(&[
            ClientEvent::TextDelta {
                session_id: "session-1".into(),
                part_id: Default::default(),
                delta: "delta".into(),
            }
        ]));
    }

    #[test]
    fn prompt_history_recalls_entries_and_restores_the_stash() {
        let mut history = PromptHistory::default();
        assert_eq!(history.recall_older("draft"), None);

        history.record("first".into());
        history.record("second".into());
        history.record("second".into());
        history.record("  ".into());

        assert_eq!(history.recall_older("draft").as_deref(), Some("second"));
        assert_eq!(history.recall_older("").as_deref(), Some("first"));
        assert_eq!(history.recall_older("").as_deref(), Some("first"));
        assert_eq!(history.recall_newer().as_deref(), Some("second"));
        assert_eq!(history.recall_newer().as_deref(), Some("draft"));
        assert!(!history.is_recalling());
        assert_eq!(history.recall_newer(), None);
    }

    #[test]
    fn prompt_history_recording_resets_recall_and_caps_entries() {
        let mut history = PromptHistory::default();
        history.record("one".into());
        history.recall_older("");
        assert!(history.is_recalling());
        history.record("two".into());
        assert!(!history.is_recalling());

        for index in 0..PROMPT_HISTORY_LIMIT + 10 {
            history.record(format!("prompt-{index}"));
        }
        let mut oldest = None;
        history.recall_older("");
        for _ in 0..PROMPT_HISTORY_LIMIT {
            oldest = history.recall_older("");
        }
        assert_eq!(oldest.as_deref(), Some("prompt-10"));
    }

    #[test]
    fn slash_menu_filters_from_a_leading_slash_query() {
        assert_eq!(filtered_slash_commands("").len(), 0);
        assert_eq!(filtered_slash_commands("hello").len(), 0);
        assert_eq!(filtered_slash_commands("/").len(), SLASH_COMMANDS.len());
        assert_eq!(filtered_slash_commands("/cl")[0].name, "/clear");
        assert_eq!(filtered_slash_commands("/C").len(), 2);
        // Typing arguments stops matching so the menu closes on its own.
        assert_eq!(filtered_slash_commands("/goal fix the bug").len(), 0);
    }

    #[test]
    fn daemon_slash_commands_route_full_command_lines() {
        assert_eq!(daemon_slash_command("/clear").as_deref(), Some("/clear"));
        assert_eq!(
            daemon_slash_command("/goal ship it").as_deref(),
            Some("/goal ship it")
        );
        assert_eq!(
            daemon_slash_command("  /compact  ").as_deref(),
            Some("/compact")
        );
        assert_eq!(daemon_slash_command("/unknown"), None);
        assert_eq!(daemon_slash_command("plain prompt"), None);
        assert_eq!(daemon_slash_command(""), None);
    }

    #[test]
    fn permission_mode_labels_cover_every_wire_id() {
        assert_eq!(permission_mode_label(None), "permissions");
        assert_eq!(permission_mode_label(Some("")), "permissions");
        assert_eq!(permission_mode_label(Some("auto_plus")), "Auto+");
        assert_eq!(permission_mode_label(Some("custom")), "custom");
        for (id, ..) in PERMISSION_MODES {
            assert!(!id.is_empty());
            assert_ne!(permission_mode_label(Some(id)), "permissions");
        }
    }

    #[test]
    fn thinking_variants_come_from_the_current_model() {
        let models = vec![
            mew_protocol::ModelInfo {
                id: "deepseek/deepseek-v4-flash".into(),
                provider: "deepseek".into(),
                model: "deepseek-v4-flash".into(),
                description: None,
                thinking_variants: vec![
                    mew_protocol::ThinkingVariantInfo {
                        name: "high".into(),
                    },
                    mew_protocol::ThinkingVariantInfo { name: "max".into() },
                ],
                thinking_budget: None,
                context_window: None,
            },
            mew_protocol::ModelInfo {
                id: "z-ai/glm-5".into(),
                provider: "z-ai".into(),
                model: "glm-5".into(),
                description: None,
                thinking_variants: Vec::new(),
                thinking_budget: None,
                context_window: None,
            },
        ];

        assert_eq!(
            thinking_variants_for_model(&models, Some("deepseek"), Some("deepseek-v4-flash")),
            vec!["high".to_owned(), "max".to_owned()]
        );
        assert!(thinking_variants_for_model(&models, Some("z-ai"), Some("glm-5")).is_empty());
        assert!(thinking_variants_for_model(&models, None, Some("glm-5")).is_empty());
        assert!(thinking_variants_for_model(&models, Some("missing"), Some("glm-5")).is_empty());
    }

    #[test]
    fn thinking_variants_hide_catalog_off_entries() {
        let models = vec![mew_protocol::ModelInfo {
            id: "alibaba-token-plan/qwen3.8-max-preview".into(),
            provider: "alibaba-token-plan".into(),
            model: "qwen3.8-max-preview".into(),
            description: None,
            thinking_variants: vec![
                mew_protocol::ThinkingVariantInfo { name: "low".into() },
                mew_protocol::ThinkingVariantInfo {
                    name: "medium".into(),
                },
                mew_protocol::ThinkingVariantInfo {
                    name: "xhigh".into(),
                },
                mew_protocol::ThinkingVariantInfo {
                    name: " off ".into(),
                },
            ],
            thinking_budget: None,
            context_window: None,
        }];

        assert_eq!(
            thinking_variants_for_model(
                &models,
                Some("alibaba-token-plan"),
                Some("qwen3.8-max-preview")
            ),
            vec!["low", "medium", "xhigh"]
        );
    }

    #[test]
    fn thinking_effort_option_ids_are_unique_when_catalog_has_off_variant() {
        let ids = (0..5)
            .map(thinking_effort_option_id)
            .collect::<BTreeSet<_>>();

        assert_eq!(ids.len(), 5);
    }

    fn conversation(session_id: &str, archived: bool, group_id: Option<&str>) -> ConversationItem {
        conversation_at(session_id, archived, group_id, None)
    }

    fn conversation_at(
        session_id: &str,
        archived: bool,
        group_id: Option<&str>,
        last_message_at: Option<i64>,
    ) -> ConversationItem {
        ConversationItem {
            session_id: session_id.into(),
            title: format!("Session {session_id}"),
            cwd: Some("/tmp/project".into()),
            workspace_path: Some("/tmp/project".into()),
            last_message_at,
            state: mew_protocol::SessionState::Idle,
            last_turn_failed: false,
            failure_detail: None,
            unread_completion: false,
            needs_attention: false,
            archived,
            pinned: false,
            group_id: group_id.map(str::to_owned),
        }
    }

    #[test]
    fn mention_query_requires_a_whitespace_delimited_at_token() {
        assert_eq!(mention_query_at_cursor("@", 1), Some((0, String::new())));
        assert_eq!(
            mention_query_at_cursor("look at @src/ma", 15),
            Some((8, "src/ma".into()))
        );
        // The cursor bounds the query; text after it is ignored.
        assert_eq!(
            mention_query_at_cursor("check @README.md please", 13),
            Some((6, "README".into()))
        );
        // No `@` token at the cursor.
        assert_eq!(mention_query_at_cursor("plain text", 10), None);
        assert_eq!(mention_query_at_cursor("email me at a@b", 15), None);
        assert_eq!(
            mention_query_at_cursor("@done here", 5),
            Some((0, "done".into()))
        );
    }

    #[test]
    fn mention_candidates_match_workspace_files_case_insensitively() {
        let mut entries = BTreeMap::new();
        entries.insert(
            String::new(),
            vec![
                DirEntry {
                    name: "src".into(),
                    is_dir: true,
                    size: None,
                },
                DirEntry {
                    name: "README.md".into(),
                    is_dir: false,
                    size: Some(12),
                },
            ],
        );
        entries.insert(
            "src".into(),
            vec![DirEntry {
                name: "main.rs".into(),
                is_dir: false,
                size: Some(4),
            }],
        );

        let paths = mention_file_paths(&entries);
        assert_eq!(
            paths,
            vec!["README.md".to_owned(), "src/main.rs".to_owned()]
        );
        assert_eq!(
            filter_mention_candidates(&paths, "MAIN"),
            vec!["src/main.rs"]
        );
        assert_eq!(filter_mention_candidates(&paths, "").len(), 2);
        assert!(filter_mention_candidates(&paths, "missing").is_empty());
        assert_eq!(join_tree_path("", "src"), "src");
        assert_eq!(join_tree_path("src", "main.rs"), "src/main.rs");
    }

    #[test]
    fn file_tree_rows_follow_expanded_directories() {
        let mut entries = BTreeMap::new();
        entries.insert(
            String::new(),
            vec![
                DirEntry {
                    name: "src".into(),
                    is_dir: true,
                    size: None,
                },
                DirEntry {
                    name: "README.md".into(),
                    is_dir: false,
                    size: Some(1),
                },
            ],
        );
        entries.insert(
            "src".into(),
            vec![DirEntry {
                name: "main.rs".into(),
                is_dir: false,
                size: Some(2),
            }],
        );

        let collapsed = collect_file_tree_rows(&entries, &BTreeSet::new());
        assert_eq!(collapsed.len(), 2);
        assert_eq!(collapsed[0].path, "src");
        assert!(collapsed[0].is_dir && !collapsed[0].expanded && collapsed[0].loaded);

        let expanded = collect_file_tree_rows(&entries, &BTreeSet::from(["src".to_owned()]));
        assert_eq!(expanded.len(), 3);
        assert_eq!(expanded[1].path, "src/main.rs");
        assert_eq!(expanded[1].depth, 1);

        // An expanded directory without a fetched listing shows no children.
        let mut unfetched = BTreeSet::new();
        unfetched.insert("missing".to_owned());
        let rows = collect_file_tree_rows(&entries, &unfetched);
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn clipboard_image_names_carry_format_and_timestamp() {
        assert_eq!(clipboard_image_file_name("png", 42), "mew-paste-42.png");
        assert_eq!(clipboard_image_file_name("jpg", 7), "mew-paste-7.jpg");
    }

    #[test]
    fn sidebar_rows_separate_archived_sessions_into_their_own_section() {
        let conversations = vec![
            conversation("a", false, Some("grp")),
            conversation("b", false, None),
            conversation("c", true, Some("grp")),
            conversation("d", true, None),
        ];
        let groups = vec![mew_protocol::GroupInfo {
            id: "grp".into(),
            name: "Project".into(),
            color: None,
            order: 0,
        }];

        let rows = build_sidebar_rows(&conversations, &groups, &BTreeSet::new());
        let group_counts: Vec<(&str, usize)> = rows
            .iter()
            .filter_map(|row| match row {
                SidebarRow::Group { id, count, .. } => Some((id.as_str(), *count)),
                _ => None,
            })
            .collect();
        // Archived sessions are excluded from their groups and the ungrouped
        // section, and collected under "Archived" instead.
        assert_eq!(
            group_counts,
            vec![("grp", 1), ("__ungrouped__", 1), ("__archived__", 2)]
        );
        let session_ids: Vec<&str> = rows
            .iter()
            .filter_map(|row| match row {
                SidebarRow::Session(conversation) => Some(conversation.session_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(session_ids, vec!["a", "b", "c", "d"]);

        let collapsed = build_sidebar_rows(
            &conversations,
            &groups,
            &BTreeSet::from(["__archived__".to_owned()]),
        );
        let session_ids: Vec<&str> = collapsed
            .iter()
            .filter_map(|row| match row {
                SidebarRow::Session(conversation) => Some(conversation.session_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(session_ids, vec!["a", "b"]);
    }

    #[test]
    fn sidebar_sessions_sort_newest_first_with_stable_ties() {
        let conversations = vec![
            conversation_at("older", false, Some("grp"), Some(10)),
            conversation_at("newer", false, Some("grp"), Some(30)),
            conversation_at("tie-b", false, None, Some(20)),
            conversation_at("tie-a", false, None, Some(20)),
            conversation("unknown", false, None),
        ];
        let groups = vec![mew_protocol::GroupInfo {
            id: "grp".into(),
            name: "Project".into(),
            color: None,
            order: 0,
        }];

        let rows = build_sidebar_rows(&conversations, &groups, &BTreeSet::new());
        let session_ids: Vec<&str> = rows
            .iter()
            .filter_map(|row| match row {
                SidebarRow::Session(conversation) => Some(conversation.session_id.as_str()),
                _ => None,
            })
            .collect();

        assert_eq!(
            session_ids,
            vec!["newer", "older", "tie-a", "tie-b", "unknown"]
        );
    }

    #[test]
    fn sidebar_rows_keep_pinned_sessions_in_a_top_section() {
        let mut pinned = conversation_at("pinned", false, Some("grp"), Some(10));
        pinned.pinned = true;
        let conversations = vec![
            pinned,
            conversation_at("regular", false, Some("grp"), Some(20)),
        ];
        let groups = vec![mew_protocol::GroupInfo {
            id: "grp".into(),
            name: "Project".into(),
            color: None,
            order: 0,
        }];

        let rows = build_sidebar_rows(&conversations, &groups, &BTreeSet::new());
        let group_ids: Vec<&str> = rows
            .iter()
            .filter_map(|row| match row {
                SidebarRow::Group { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        let session_ids: Vec<&str> = rows
            .iter()
            .filter_map(|row| match row {
                SidebarRow::Session(conversation) => Some(conversation.session_id.as_str()),
                _ => None,
            })
            .collect();

        assert_eq!(group_ids, vec![PINNED_GROUP_ID, "grp"]);
        assert_eq!(session_ids, vec!["pinned", "regular"]);
    }

    #[test]
    fn sidebar_search_filters_sessions_and_expands_matching_groups() {
        let conversations = vec![
            conversation("alpha", false, Some("grp")),
            conversation("beta", false, Some("grp")),
            conversation("gamma", false, None),
        ];
        let groups = vec![mew_protocol::GroupInfo {
            id: "grp".into(),
            name: "Project".into(),
            color: None,
            order: 0,
        }];

        let rows = build_sidebar_rows_with_query(
            &conversations,
            &groups,
            &BTreeSet::from(["grp".to_owned()]),
            "ALPHA",
        );
        let sessions: Vec<&str> = rows
            .iter()
            .filter_map(|row| match row {
                SidebarRow::Session(conversation) => Some(conversation.session_id.as_str()),
                _ => None,
            })
            .collect();
        let group = rows.iter().find_map(|row| match row {
            SidebarRow::Group { id, collapsed, .. } if id == "grp" => Some(*collapsed),
            _ => None,
        });

        assert_eq!(sessions, vec!["alpha"]);
        assert_eq!(group, Some(false));
    }

    #[test]
    fn mention_menu_height_scales_with_capped_options() {
        assert_eq!(mention_menu_height(0), px(40.));
        assert_eq!(
            mention_menu_height(2),
            mention_menu_height(MENTION_MENU_LIMIT.min(2))
        );
        assert_eq!(
            mention_menu_height(100),
            mention_menu_height(MENTION_MENU_LIMIT)
        );
    }
}

#[cfg(test)]
mod code_continuity_tests {
    use super::super::*;

    #[gpui::test]
    fn markdown_horizontal_scroll_retains_append_identity_and_resets_for_replacement(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        shell.update(cx, |shell, _| {
            shell.model.ui.transcript = vec![TranscriptItem {
                role: TranscriptRole::Assistant,
                text: "```text\none physical line\n```".into(),
                parts: Vec::new(),
            }];
            shell.sync_markdown_cache();
            shell
                .markdown_scroll_handle(0, 0)
                .unwrap()
                .set_offset(point(px(-60.), px(0.)));
            shell.model.ui.transcript[0].text.push_str("\n\nafter");
            shell.sync_markdown_cache();
            assert_eq!(
                shell.markdown_scroll_handle(0, 0).unwrap().offset().x,
                px(-60.)
            );

            shell.model.ui.transcript[0].text = "```text\na replacement fence\n```".into();
            shell.sync_markdown_cache();
            assert_eq!(
                shell.markdown_scroll_handle(0, 0).unwrap().offset().x,
                px(0.)
            );
            assert_eq!(shell.markdown_cache[0][0].scroll_groups.len(), 1);
            shell.model.ui.transcript.clear();
            shell.sync_markdown_cache();
            assert!(shell.markdown_cache.is_empty());
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn code_continuations_share_line_leading_and_horizontal_position(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        let code = std::iter::once(format!("let caption = \"{}🙂\";", "long ".repeat(90)))
            .chain((0..64).map(|index| format!("line_{index}: 中🙂;")))
            .collect::<Vec<_>>()
            .join("\n");
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.transcript = vec![TranscriptItem {
                role: TranscriptRole::Assistant,
                text: format!("```rust\n{code}\n```\n\n```text\nseparate fence\n```"),
                parts: Vec::new(),
            }];
            shell.transcript_selection = Some(TranscriptSelection {
                start: TranscriptSelectionPoint {
                    message_index: 0,
                    block_index: 0,
                    offset: 0,
                },
                end: TranscriptSelectionPoint {
                    message_index: 0,
                    block_index: 0,
                    offset: 0,
                },
            });
            shell.sync_markdown_cache();
            shell.transcript_list.scroll_to(gpui::ListOffset::default());
            cx.notify();
        });
        for width in [380., 920.] {
            cx.simulate_resize(gpui::size(px(width), px(1900.)));
            cx.run_until_parked();
            let initial_x = shell.read_with(cx, |shell, _| {
                let entries = shell.transcript_text_registry.borrow();
                let mut code_entries = entries
                    .iter()
                    .filter(|entry| entry.block_index < 2)
                    .collect::<Vec<_>>();
                code_entries.sort_by_key(|entry| entry.block_index);
                assert_eq!(code_entries.len(), 2);
                let first = code_entries[0];
                let second = code_entries[1];
                let last_line = first
                    .layout
                    .position_for_index(first.text.trim_end_matches('\n').len())
                    .unwrap();
                let next_line = second.layout.position_for_index(0).unwrap();
                assert_eq!(
                    next_line.y - last_line.y,
                    px(20.),
                    "a virtual seam has normal line leading"
                );
                assert_eq!(next_line.x, first.layout.position_for_index(0).unwrap().x);
                next_line.x
            });
            assert!(cx.debug_bounds("copy-code-0-0").is_some());
            assert!(cx.debug_bounds("copy-code-0-1").is_none());
            assert!(
                cx.debug_bounds("copy-code-0-5").is_some(),
                "an adjacent fence keeps its own copy button"
            );
            let first = cx.debug_bounds("markdown-inline-container-0-0").unwrap();
            let viewport_left = shell.read_with(cx, |shell, _| {
                shell.markdown_scroll_handle(0, 0).unwrap().bounds().left()
            });
            cx.simulate_event(gpui::ScrollWheelEvent {
                position: point(viewport_left + px(20.), first.top() + px(8.)),
                delta: gpui::ScrollDelta::Pixels(point(px(-80.), px(0.))),
                modifiers: gpui::Modifiers::none(),
                touch_phase: gpui::TouchPhase::Moved,
            });
            cx.run_until_parked();
            shell.read_with(cx, |shell, _| {
                let entries = shell.transcript_text_registry.borrow();
                let first = entries.iter().find(|entry| entry.block_index == 0).unwrap().layout.position_for_index(0).unwrap();
                let second = entries.iter().find(|entry| entry.block_index == 1).unwrap().layout.position_for_index(0).unwrap();
                assert!(first.x < initial_x, "long code scrolls horizontally: first={first:?}, initial={initial_x:?}, handle={:?}", shell.markdown_scroll_handle(0, 0).map(|handle| (handle.offset(), handle.max_offset(), handle.bounds())));
                assert_eq!(first.x, second.x, "all code chunks share the horizontal position");
            });
            let button = cx.debug_bounds("copy-code-0-0").unwrap();
            cx.simulate_click(button.center(), gpui::Modifiers::none());
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().as_deref(),
                Some(code.as_str())
            );
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }
}
