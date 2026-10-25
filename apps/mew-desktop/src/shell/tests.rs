#[cfg(test)]
mod quiet_tools_tests {
    use super::super::*;

    fn tool(status: ToolStatus) -> TranscriptPart {
        TranscriptPart::ToolCall {
            tool_name: "shell".into(),
            call_id: "quiet-call".into(),
            status,
            input: serde_json::json!({
                "command": format!("cargo test {}末尾", "very_long_package_name/".repeat(18)),
            })
            .to_string(),
            output: Some("complete output é🙂\nsecond line\n".into()),
            error: (status == ToolStatus::Error)
                .then(|| "\n  permission denied: src/main.rs\nfull error details".into()),
            diff: Some("-before\n+after".into()),
        }
    }

    #[gpui::test]
    fn quiet_completed_tool_uses_one_line_and_keeps_disclosure_and_copy(
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
                text: String::new(),
                parts: vec![tool(ToolStatus::Completed)],
            }];
            shell.sync_markdown_cache();
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(420.), px(900.)));
        cx.run_until_parked();
        let content = cx.debug_bounds("transcript-content-0").unwrap();
        let header = cx.debug_bounds("chat-part-0-0-header").unwrap();
        assert_eq!(
            content.size.height,
            px(20.),
            "completed activity is one line"
        );
        assert!(header.left() >= content.left() && header.right() <= content.right());
        shell.update(cx, |shell, cx| {
            let TranscriptPart::ToolCall { input, .. } = &mut shell.model.ui.transcript[0].parts[0]
            else {
                unreachable!()
            };
            *input = serde_json::json!({ "command": "cargo test\nprintf 'done'" }).to_string();
            shell.sync_markdown_cache();
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(
            cx.debug_bounds("transcript-content-0").unwrap().size.height,
            px(20.),
            "multiline commands also keep one summary line"
        );
        let header = cx.debug_bounds("chat-part-0-0-header").unwrap();
        cx.simulate_click(header.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(cx.debug_bounds("tool-input-0-0-line-0").is_some());
        assert!(cx.debug_bounds("tool-diff-0-0-line-0").is_some());
        let copy = cx.debug_bounds("copy-tool-output-0-0").unwrap();
        cx.simulate_click(copy.center(), gpui::Modifiers::none());
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("complete output é🙂\nsecond line\n")
        );
        let header = cx.debug_bounds("chat-part-0-0-header").unwrap();
        cx.simulate_click(header.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(cx.debug_bounds("tool-input-0-0-line-0").is_none());
        for (key, expanded) in [("enter", true), ("space", false)] {
            let keystroke = gpui::Keystroke::parse(key).unwrap();
            cx.simulate_event(gpui::KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            cx.simulate_event(gpui::KeyUpEvent { keystroke });
            cx.run_until_parked();
            assert_eq!(cx.debug_bounds("tool-input-0-0-line-0").is_some(), expanded);
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn quiet_tool_completion_preserves_visible_failure_and_distinct_active_states(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        for status in [
            ToolStatus::Pending,
            ToolStatus::Running,
            ToolStatus::Error,
            ToolStatus::Completed,
        ] {
            shell.update(cx, |shell, cx| {
                shell.layout.sidebar_collapsed = true;
                shell.layout.workbench_collapsed = true;
                shell.model.ui.transcript = vec![TranscriptItem {
                    role: TranscriptRole::Assistant,
                    text: String::new(),
                    parts: vec![tool(status)],
                }];
                shell.sync_markdown_cache();
                cx.notify();
            });
            cx.simulate_resize(gpui::size(px(420.), px(900.)));
            cx.run_until_parked();
            let content = cx.debug_bounds("transcript-content-0").unwrap();
            if status == ToolStatus::Completed {
                assert_eq!(content.size.height, px(20.));
            } else {
                assert!(content.size.height > px(20.));
            }
            assert_eq!(
                cx.debug_bounds("tool-error-0-0").is_some(),
                status == ToolStatus::Error,
                "collapsed failure exposes the cause and clears it after completion"
            );
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }
}

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
            assert_eq!(
                a.center().y,
                controls.last().unwrap().center().y,
                "settings and send stay on one stable toolbar row"
            );
        }
        assert_eq!(controls.last().unwrap().size.width, px(28.));
        let path = cx.debug_bounds("checkout-path").unwrap();
        let status = cx.debug_bounds("session-status").unwrap();
        assert!(path.right() <= status.left());
        assert!(path.left() >= composer.left());
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn composer_toolbar_keeps_its_height_and_picker_anchors_across_pane_widths(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            cx.notify();
        });
        for width in [420., 480., 640., 900.] {
            cx.simulate_resize(gpui::size(px(width), px(760.)));
            for long in [false, true] {
                for running in [false, true] {
                    shell.update(cx, |shell, cx| {
                        shell.model.ui.model = Some(if long { "provider/a-very-long-model-identifier-with-a-version-and-release-date" } else { "gpt-5.2" }.into());
                        shell.model.ui.current_persona = Some(if long { "a-long-persona-name-é🙂" } else { "builder" }.into());
                        shell.model.ui.permission_mode = Some(if long { "custom-permission-policy-with-a-long-name" } else { "standard" }.into());
                        shell.model.ui.thinking_variant = Some("high".into());
                        shell.model.ui.running = running;
                        cx.notify();
                    });
                    cx.run_until_parked();
                    let composer = cx.debug_bounds("composer").unwrap();
                    let send = cx
                        .debug_bounds(if running {
                            "cancel-turn"
                        } else {
                            "submit-prompt"
                        })
                        .unwrap();
                    assert_eq!(send.size, gpui::size(px(28.), px(28.)));
                    assert!((composer.right() - send.right()) <= px(12.));
                    for selector in [
                        "permission-picker-trigger",
                        "persona-picker-trigger",
                        "model-picker-trigger",
                    ] {
                        let bounds = cx.debug_bounds(selector).unwrap();
                        assert_eq!(
                            bounds.center().y,
                            send.center().y,
                            "{selector}, width {width}, long {long}, running {running}"
                        );
                        assert!(bounds.left() >= composer.left() && bounds.right() < send.left());
                        assert_eq!(bounds.size.height, px(26.));
                        assert!(bounds.size.width > px(24.));
                    }
                    let permission = cx.debug_bounds("permission-picker-trigger");
                    let persona = cx.debug_bounds("persona-picker-trigger");
                    let model = cx.debug_bounds("model-picker-trigger");
                    if !long && width == 900. {
                        assert!(
                            model.unwrap().size.width < px(160.),
                            "a short model name does not stretch into a wide empty button"
                        );
                    }
                    shell.update(cx, |shell, _| {
                        assert_eq!(shell.permission_picker_bounds, permission);
                        assert_eq!(shell.persona_picker_bounds, persona);
                        assert_eq!(shell.model_picker_bounds, model);
                    });
                }
            }
        }
        for selector in [
            "permission-picker-trigger",
            "persona-picker-trigger",
            "model-picker-trigger",
        ] {
            let trigger = cx.debug_bounds(selector).unwrap();
            cx.simulate_click(trigger.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            shell.update(cx, |shell, _| {
                assert_eq!(
                    shell.permission_picker_open,
                    selector == "permission-picker-trigger"
                );
                assert_eq!(
                    shell.persona_picker_open,
                    selector == "persona-picker-trigger"
                );
                assert_eq!(shell.model_picker_open, selector == "model-picker-trigger");
            });
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn composer_uses_current_provider_display_name_and_refreshes_catalog_labels(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.current_provider = Some("second".into());
            shell.model.ui.model = Some("internal-model-snapshot-20261024".into());
            shell.model.ui.models = serde_json::from_value(serde_json::json!([
                {"id":"first/internal-model-snapshot-20261024", "provider":"first",
                 "model":"internal-model-snapshot-20261024", "display_name":"A different provider with a long name"},
                {"id":"second/internal-model-snapshot-20261024", "provider":"second",
                 "model":"internal-model-snapshot-20261024", "display_name":"Swift"}
            ])).unwrap();
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(680.), px(760.)));
        cx.run_until_parked();
        let named_width = cx.debug_bounds("model-picker-trigger").unwrap().size.width;
        assert!(
            named_width < px(100.),
            "the selected provider's short name replaces its long raw ID"
        );

        for name in [Some("A renamed catalog model"), Some(" \t"), None] {
            shell.update(cx, |shell, cx| {
                let mut model = serde_json::to_value(&shell.model.ui.models[1]).unwrap();
                model["display_name"] = serde_json::json!(name);
                shell.model.ui.models[1] = serde_json::from_value(model).unwrap();
                cx.notify();
            });
            cx.run_until_parked();
            assert!(cx.debug_bounds("model-picker-trigger").unwrap().size.width > named_width);
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn model_picker_searches_display_names_and_selects_original_identity(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.models = serde_json::from_value(serde_json::json!([
                {"id":"first/snapshot", "provider":"first", "model":"snapshot", "display_name":"Aurora"},
                {"id":"second/snapshot", "provider":"second", "model":"snapshot", "display_name":"Swift"}
            ])).unwrap();
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(420.), px(760.)));
        cx.run_until_parked();
        let trigger = cx.debug_bounds("model-picker-trigger").unwrap();
        cx.simulate_click(trigger.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        cx.simulate_input("aUrOrA");
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert_eq!(shell.model_picker_filtered_indices, vec![0])
        });
        assert!(cx.debug_bounds("model-option-all-1").is_none());
        let option = cx
            .debug_bounds("model-option-all-0")
            .expect("search matches the display name");
        cx.simulate_click(option.center(), gpui::Modifiers::none());
        shell.update(cx, |shell, _| {
            assert_eq!(
                shell.pending_model,
                Some(("first".into(), "snapshot".into()))
            );
            assert!(!shell.model_picker_open);
            shell.quitting = true;
        });
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
    fn sidebar_chat_rows_keep_one_height_across_activity_and_nested_paths(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            let mut chats = (0..5)
                .map(|id| navigation_conversation(&id.to_string()))
                .collect::<Vec<_>>();
            chats[0].cwd = chats[0].workspace_path.clone();
            chats[1].state = mew_protocol::SessionState::Running;
            chats[2].last_turn_failed = true;
            chats[3].unread_completion = true;
            chats[4].needs_attention = true;
            shell.model.ui.conversations = chats;
            shell.collapsed_groups.clear();
            shell.layout.sidebar_collapsed = false;
            shell.rebuild_sidebar_rows();
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(1100.), px(800.)));
        cx.run_until_parked();
        for selector in [
            "sidebar-chat-0",
            "sidebar-chat-1",
            "sidebar-chat-2",
            "sidebar-chat-3",
            "sidebar-chat-4",
        ] {
            let row = cx.debug_bounds(selector).unwrap();
            assert_eq!(row.size.height, px(30.), "{selector} stays on one line");
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn sidebar_quick_actions_preserve_selection_and_send_the_target_chat(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        let (sender, mut commands) = tokio::sync::mpsc::unbounded_channel();
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.command_tx = Some(sender);
            shell.model.ui.conversations = vec![navigation_conversation("a")];
            shell.model.ui.selected_session = Some("other".into());
            shell.sidebar_keyboard_session = Some("a".into());
            shell.collapsed_groups.clear();
            shell.layout.sidebar_collapsed = false;
            shell.rebuild_sidebar_rows();
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let sidebar = shell.read(cx).sidebar_focus_handle.clone();
            window.focus(&sidebar, cx);
        });
        cx.run_until_parked();
        let pin = cx
            .debug_bounds("sidebar-pin-a")
            .expect("focused chats expose pin");
        cx.simulate_click(pin.center(), gpui::Modifiers::none());
        assert!(
            matches!(commands.try_recv().unwrap(), ClientMessage::PinSession { session_id, pinned: true } if session_id == "a")
        );
        cx.run_until_parked();
        let archive = cx.debug_bounds("sidebar-archive-a").unwrap();
        cx.simulate_click(archive.center(), gpui::Modifiers::none());
        assert!(
            matches!(commands.try_recv().unwrap(), ClientMessage::ArchiveSession { session_id, archived: true } if session_id == "a")
        );
        cx.update(|window, cx| {
            let composer = shell.read(cx).composer_focus_handle.clone();
            window.focus(&composer, cx);
        });
        cx.simulate_event(gpui::MouseMoveEvent {
            position: gpui::point(px(800.), px(700.)),
            pressed_button: None,
            modifiers: gpui::Modifiers::none(),
        });
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("sidebar-pin-a").is_none(),
            "secondary actions hide when writing resumes"
        );
        let row = cx.debug_bounds("sidebar-chat-a").unwrap();
        cx.simulate_mouse_move(row.center(), None, gpui::Modifiers::none());
        cx.run_until_parked();
        let pin = cx
            .debug_bounds("sidebar-pin-a")
            .expect("hover exposes the same actions");
        assert_eq!(cx.debug_bounds("sidebar-chat-a").unwrap(), row);
        cx.simulate_click(pin.center(), gpui::Modifiers::none());
        assert!(
            matches!(commands.try_recv().unwrap(), ClientMessage::PinSession { session_id, pinned: true } if session_id == "a")
        );
        cx.update(|window, cx| {
            let sidebar = shell.read(cx).sidebar_focus_handle.clone();
            window.focus(&sidebar, cx);
        });
        let keystroke = gpui::Keystroke::parse("shift-f10").unwrap();
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui::KeyUpEvent { keystroke });
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert_eq!(shell.model.ui.selected_session.as_deref(), Some("other"));
            assert_eq!(shell.session_menu_session.as_deref(), Some("a"));
        });
        let keystroke = gpui::Keystroke::parse("escape").unwrap();
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui::KeyUpEvent { keystroke });
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert!(
                shell.session_menu_session.is_none(),
                "Escape dismisses the chat menu"
            );
        });
        let row = cx.debug_bounds("sidebar-chat-a").unwrap();
        cx.simulate_mouse_down(
            row.center(),
            gpui::MouseButton::Right,
            gpui::Modifiers::none(),
        );
        cx.simulate_mouse_up(
            row.center(),
            gpui::MouseButton::Right,
            gpui::Modifiers::none(),
        );
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert_eq!(shell.session_menu_session.as_deref(), Some("a"));
            assert_eq!(shell.model.ui.selected_session.as_deref(), Some("other"));
        });
        cx.simulate_click(gpui::point(px(800.), px(700.)), gpui::Modifiers::none());
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert!(
                shell.session_menu_session.is_none(),
                "outside clicks dismiss the chat menu"
            );
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn sidebar_inbox_is_available_without_search_and_tracks_unread_and_requests(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            let mut chats = (0..4)
                .map(|id| navigation_conversation(&id.to_string()))
                .collect::<Vec<_>>();
            chats[0].unread_completion = true;
            chats[1].needs_attention = true;
            chats[2].last_turn_failed = true;
            chats[3].unread_completion = true;
            chats[3].archived = true;
            shell.model.ui.conversations = chats;
            shell.model.ui.composer = "keep this draft".into();
            shell.collapsed_groups.insert("/workspace/mew".into());
            shell.layout.sidebar_collapsed = false;
            shell.rebuild_sidebar_rows();
            cx.notify();
        });
        cx.run_until_parked();
        let inbox = cx
            .debug_bounds("sidebar-inbox")
            .expect("inbox is a permanent navigation entry");
        assert!(cx.debug_bounds("sidebar-inbox-count-2").is_some());
        cx.simulate_click(inbox.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(cx.debug_bounds("sidebar-chat-0").is_some());
        assert!(cx.debug_bounds("sidebar-chat-1").is_some());
        assert!(cx.debug_bounds("sidebar-chat-2").is_none());
        assert!(cx.debug_bounds("sidebar-chat-3").is_none());
        assert!(cx.debug_bounds("sidebar-search-frame").is_none());
        shell.update(cx, |shell, cx| {
            assert_eq!(shell.sidebar_filter, SidebarFilter::Inbox);
            assert_eq!(shell.model.ui.composer, "keep this draft");
            shell.model.ui.conversations[0].unread_completion = false;
            shell.rebuild_sidebar_rows();
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("sidebar-inbox-count-1").is_some());
        assert!(cx.debug_bounds("sidebar-chat-0").is_none());
        shell.update(cx, |shell, cx| {
            shell.model.ui.conversations[1].needs_attention = false;
            shell.rebuild_sidebar_rows();
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("sidebar-inbox-empty").is_some());
        assert!(cx.debug_bounds("sidebar-inbox-count-1").is_none());
        let inbox = cx.debug_bounds("sidebar-inbox").unwrap();
        cx.simulate_click(inbox.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert_eq!(shell.sidebar_filter, SidebarFilter::All);
            assert!(shell.collapsed_groups.contains("/workspace/mew"));
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn a_new_chat_leaves_inbox_so_its_workspace_can_reveal_it(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        shell.update(cx, |shell, cx| {
            shell.sidebar_filter = SidebarFilter::Inbox;
            shell.new_conversation(cx);
            assert_eq!(shell.sidebar_filter, SidebarFilter::All);
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn hovering_a_chat_reveals_its_full_title_and_workspace_preview(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            let mut chat = navigation_conversation("a");
            chat.title = "Investigate why long conversation titles were truncated before the row could measure them é🙂".into();
            shell.model.ui.conversations = vec![chat];
            shell.collapsed_groups.clear();
            shell.layout.sidebar_collapsed = false;
            shell.rebuild_sidebar_rows();
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(1100.), px(800.)));
        cx.run_until_parked();
        let row = cx.debug_bounds("sidebar-chat-a").unwrap();
        cx.simulate_mouse_move(row.center(), None, gpui::Modifiers::none());
        cx.run_until_parked();
        cx.dispatcher.advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        let preview = cx
            .debug_bounds("sidebar-chat-preview")
            .expect("chat hover reveals its preview");
        assert_eq!(preview.size.width, px(320.));
        assert_eq!(
            cx.debug_bounds("sidebar-chat-a").unwrap(),
            row,
            "hover preserves row geometry"
        );
        cx.simulate_mouse_move(
            gpui::point(px(800.), px(500.)),
            None,
            gpui::Modifiers::none(),
        );
        cx.run_until_parked();
        cx.dispatcher.advance_clock(Duration::from_secs(1));
        cx.run_until_parked();
        assert!(cx.debug_bounds("sidebar-chat-preview").is_none());
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn archived_chats_can_be_found_and_restored_from_the_sidebar(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        let (sender, mut commands) = tokio::sync::mpsc::unbounded_channel();
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            let mut archived = navigation_conversation("a");
            archived.archived = true;
            shell.model.ui.conversations = vec![archived, navigation_conversation("b")];
            shell.command_tx = Some(sender);
            shell.layout.sidebar_collapsed = false;
            shell.collapsed_groups.clear();
            shell.rebuild_sidebar_rows();
            cx.notify();
        });
        cx.run_until_parked();
        let archive = cx
            .debug_bounds("sidebar-archived")
            .expect("archived chats have a navigation entry");
        cx.simulate_click(archive.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        assert!(cx.debug_bounds("sidebar-chat-a").is_some());
        assert!(cx.debug_bounds("sidebar-chat-b").is_none());
        shell.update(cx, |shell, cx| {
            shell.sidebar_keyboard_session = Some("a".into());
            cx.notify();
        });
        cx.update(|window, cx| {
            let sidebar = shell.read(cx).sidebar_focus_handle.clone();
            window.focus(&sidebar, cx);
        });
        cx.run_until_parked();
        let restore = cx.debug_bounds("sidebar-archive-a").unwrap();
        cx.simulate_click(restore.center(), gpui::Modifiers::none());
        assert!(
            matches!(commands.try_recv().unwrap(), ClientMessage::ArchiveSession { session_id, archived: false } if session_id == "a")
        );
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
        assert_eq!(composer_input_height(0, 20., 96.), 48.);
        assert_eq!(composer_input_height(1, 20., 96.), 48.);
        assert_eq!(composer_input_height(2, 20., 96.), 48.);
        assert_eq!(composer_input_height(3, 20., 96.), 60.);
        assert_eq!(composer_input_height(6, 20., 96.), 96.);
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
        assert_eq!(pending_actions_anchor(composer), point(px(120.), px(392.)));
        assert_eq!(pending_actions_width(composer), px(200.));
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
        let model_rows = vec![
            ModelPickerRow::Model {
                index: 0,
                recent: false
            };
            32
        ];
        assert_eq!(model_picker_list_height(&[]), px(64.));
        assert_eq!(model_picker_list_height(&model_rows[..5]), px(280.));
        assert_eq!(model_picker_list_height(&model_rows), px(320.));
        assert_eq!(model_picker_height(&model_rows, 0), px(372.));
        assert_eq!(model_picker_height(&model_rows, 3), px(444.));
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
    fn model_picker_query_matches_names_ids_providers_and_descriptions() {
        let model = serde_json::from_value(serde_json::json!({
            "id":"openai/gpt-5", "provider":"openai", "model":"gpt-5",
            "display_name":"Aurora", "description":"fast coding model"
        }))
        .unwrap();
        for query in ["coding", "OPENAI", "gpt-5", "openai/gpt-5", " aUrOrA ", ""] {
            assert!(model_matches_query(&model, query), "{query}");
        }
        assert!(!model_matches_query(&model, "vision"));
    }

    #[test]
    fn model_display_names_trim_catalog_labels_and_fall_back_to_exact_ids() {
        for name in [Some(" Aurora "), Some(" \t"), Some(""), None] {
            let model = serde_json::from_value(serde_json::json!({
                "id":"local/org/model-v2", "provider":"local", "model":"org/model-v2",
                "display_name": name
            }))
            .unwrap();
            assert_eq!(
                model_display_name(&model),
                if name == Some(" Aurora ") {
                    "Aurora"
                } else {
                    "org/model-v2"
                }
            );
        }
    }

    #[test]
    fn recent_model_indices_preserve_saved_order_and_ignore_unknown_duplicates() {
        let models = vec![
            mew_protocol::ModelInfo {
                id: "openai/gpt-5".into(),
                provider: "openai".into(),
                model: "gpt-5".into(),
                display_name: None,
                description: None,
                thinking_variants: Vec::new(),
                thinking_budget: None,
                context_window: None,
            },
            mew_protocol::ModelInfo {
                id: "anthropic/claude".into(),
                provider: "anthropic".into(),
                model: "claude".into(),
                display_name: None,
                description: None,
                thinking_variants: Vec::new(),
                thinking_budget: None,
                context_window: None,
            },
            mew_protocol::ModelInfo {
                id: "z-ai/glm".into(),
                provider: "z-ai".into(),
                model: "glm".into(),
                display_name: None,
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
    fn model_picker_rows_put_unique_recent_models_before_other_models() {
        let models = vec![
            mew_protocol::ModelInfo {
                id: "openai/gpt-5".into(),
                provider: "openai".into(),
                model: "gpt-5".into(),
                display_name: None,
                description: None,
                thinking_variants: Vec::new(),
                thinking_budget: None,
                context_window: None,
            },
            mew_protocol::ModelInfo {
                id: "anthropic/claude".into(),
                provider: "anthropic".into(),
                model: "claude".into(),
                display_name: None,
                description: None,
                thinking_variants: Vec::new(),
                thinking_budget: None,
                context_window: None,
            },
        ];

        let (selectable, rows) = build_model_picker_rows(&models, &["anthropic/claude".into()], "");

        assert_eq!(
            selectable,
            vec![1, 0],
            "each model appears once, with recent choices first"
        );
        assert_eq!(
            rows,
            vec![
                ModelPickerRow::Header("Recent"),
                ModelPickerRow::Model {
                    index: 1,
                    recent: true,
                },
                ModelPickerRow::Header("Other models"),
                ModelPickerRow::Model {
                    index: 0,
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
            display_name: None,
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
    fn session_titles_are_normalized_without_discarding_the_full_title() {
        assert_eq!(compact_session_title("  short   title "), "short title");

        let title = compact_session_title(
            "The user asked about a repository and the requested changes are extensive",
        );
        assert_eq!(
            title,
            "The user asked about a repository and the requested changes are extensive"
        );
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
        assert_eq!(
            transcript_attention(false, true, false, false),
            Some(TranscriptAttention::Running)
        );
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
        assert_eq!(
            transcript_attention(false, true, true, false),
            Some(TranscriptAttention::Running)
        );
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
                display_name: None,
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
                display_name: None,
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
            display_name: None,
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

#[cfg(test)]
mod markdown_content_tests {
    use super::super::*;

    fn load_markdown(shell: &Entity<DesktopShell>, cx: &mut gpui::VisualTestContext, text: &str) {
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.transcript = vec![TranscriptItem {
                role: TranscriptRole::Assistant,
                text: text.into(),
                parts: Vec::new(),
            }];
            shell.sync_markdown_cache();
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(480.), px(1000.)));
        cx.run_until_parked();
    }

    #[gpui::test]
    fn markdown_list_markers_align_and_nested_items_keep_their_indent(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        load_markdown(&shell, cx, "9. first item\n   - nested item with enough words to wrap inside this narrow reading column and keep the following lines aligned\n10. last item");
        let first = cx.debug_bounds("markdown-list-content-0-0-0").unwrap();
        let nested = cx.debug_bounds("markdown-list-content-0-0-1").unwrap();
        let last = cx.debug_bounds("markdown-list-content-0-0-2").unwrap();
        assert_eq!(
            first.left(),
            last.left(),
            "ordered siblings share a content column"
        );
        assert_eq!(nested.left(), first.left() + px(24.));
        assert!(
            nested.size.height >= px(48.),
            "nested text wraps inside its indented column"
        );
        let first_marker = cx.debug_bounds("markdown-list-marker-0-0-0").unwrap();
        let last_marker = cx.debug_bounds("markdown-list-marker-0-0-2").unwrap();
        assert_eq!(first_marker.right(), last_marker.right());
        assert_eq!(first.left() - first_marker.right(), px(8.));
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn markdown_table_columns_stay_aligned_and_scroll_inside_the_conversation(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        load_markdown(&shell, cx, "| name | description | count | result |\n| :--- | :--- | ---: | :---: |\n| mew | a longer description that should wrap within its cell | 42 | ready |\n| é🙂 | small | 1 | done |");
        for width in [480., 840.] {
            cx.simulate_resize(gpui::size(px(width), px(1000.)));
            cx.run_until_parked();
            let header = cx.debug_bounds("markdown-table-cell-0-0-0-1").unwrap();
            let body = cx.debug_bounds("markdown-table-cell-0-0-1-1").unwrap();
            let second = cx.debug_bounds("markdown-table-cell-0-0-2-1").unwrap();
            let viewport = cx.debug_bounds("markdown-table-viewport-0-0").unwrap();
            assert_eq!(header.left(), body.left());
            assert_eq!(header.size.width, body.size.width);
            assert_eq!(header.left(), second.left());
            assert!(
                body.size.height > header.size.height,
                "cell content wraps, increasing row height"
            );
            let conversation = cx.debug_bounds("conversation-pane").unwrap();
            assert!(viewport.left() >= conversation.left());
            assert!(viewport.right() <= conversation.right());
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn markdown_table_selection_copies_adjacent_cells_in_reading_order(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        load_markdown(
            &shell,
            cx,
            "| name | value |\n| --- | --- |\n| é🙂 | **right 中** |",
        );
        let first = cx.debug_bounds("markdown-table-cell-0-0-1-0").unwrap();
        let last = cx.debug_bounds("markdown-table-cell-0-0-1-1").unwrap();
        let start = point(first.left() + px(12.1), first.top() + px(18.));
        let end = point(last.right() - px(12.), last.top() + px(18.));
        cx.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.run_until_parked();
        cx.simulate_mouse_move(end, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.simulate_mouse_up(end, gpui::MouseButton::Left, gpui::Modifiers::none());
        cx.run_until_parked();
        cx.dispatch_action(TextCopy);
        assert_eq!(
            cx.read_from_clipboard().unwrap().text().as_deref(),
            Some("é🙂\tright 中")
        );
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn markdown_table_alignment_and_horizontal_scroll_survive_virtual_rows(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        let mut source =
            "| name | detail | count | result | extra |\n| --- | --- | ---: | :---: | --- |\n"
                .to_owned();
        for index in 0..17 {
            source.push_str(&format!("| row{index} | text | {index} | ok | end |\n"));
        }
        load_markdown(&shell, cx, &source);
        shell.update(cx, |shell, cx| {
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
            cx.notify();
        });
        cx.run_until_parked();
        let centered_cell = cx.debug_bounds("markdown-table-cell-0-0-1-3").unwrap();
        shell.read_with(cx, |shell, _| {
            let registry = shell.transcript_text_registry.borrow();
            let right_edges = ["0", "10"].map(|text| {
                let entry = registry.iter().find(|entry| entry.text == text).unwrap();
                entry.layout.position_for_index(entry.text.len()).unwrap().x
            });
            assert_eq!(
                right_edges[0], right_edges[1],
                "right aligned numbers share an edge"
            );
            let centered = registry.iter().find(|entry| entry.text == "ok").unwrap();
            let start = centered.layout.position_for_index(0).unwrap();
            let end = centered
                .layout
                .position_for_index(centered.text.len())
                .unwrap();
            assert!(
                ((start.x + end.x) / 2. - centered_cell.center().x).abs() <= px(1.),
                "centered content stays in its column and retains correct pointer geometry"
            );
        });
        let first = cx.debug_bounds("markdown-table-cell-0-0-0-0").unwrap();
        let last = cx.debug_bounds("markdown-table-cell-0-1-0-0").unwrap();
        let viewport = cx.debug_bounds("markdown-table-viewport-0-0").unwrap();
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: viewport.origin + point(px(30.), px(20.)),
            delta: gpui::ScrollDelta::Pixels(point(px(-80.), px(0.))),
            ..Default::default()
        });
        cx.run_until_parked();
        let scrolled_first = cx.debug_bounds("markdown-table-cell-0-0-0-0").unwrap();
        let scrolled_last = cx.debug_bounds("markdown-table-cell-0-1-0-0").unwrap();
        assert!(
            scrolled_first.left() < first.left(),
            "wide tables scroll horizontally"
        );
        assert_eq!(
            scrolled_first.left() - first.left(),
            scrolled_last.left() - last.left(),
            "all table chunks share the horizontal position"
        );
        shell.update(cx, |shell, _| shell.quitting = true);
    }
}

#[cfg(test)]
mod progress_tests {
    use super::super::*;

    #[gpui::test]
    fn working_status_stays_compact_through_streaming_and_keeps_stop_available(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        let (sender, mut commands) = tokio::sync::mpsc::unbounded_channel();
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.command_tx = Some(sender);
            shell.model.ui.running = true;
            shell.model.ui.transcript = vec![TranscriptItem {
                role: TranscriptRole::User,
                text: "check the layout".into(),
                parts: Vec::new(),
            }];
            shell.sync_markdown_cache();
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(480.), px(760.)));
        cx.run_until_parked();
        let before = cx
            .debug_bounds("conversation-working")
            .expect("an active turn has a working line");
        assert!(
            before.size.height <= px(24.),
            "working fits a single quiet line"
        );
        assert!(cx.debug_bounds("conversation-attention").is_none());
        shell.update(cx, |shell, cx| {
            shell.model.ui.transcript.push(TranscriptItem {
                role: TranscriptRole::Assistant,
                text: "checking the first part".into(),
                parts: Vec::new(),
            });
            shell.sync_markdown_cache();
            cx.notify();
        });
        cx.run_until_parked();
        assert_eq!(
            cx.debug_bounds("conversation-working").unwrap(),
            before,
            "the status does not jump or disappear when assistant text arrives"
        );
        let stop = cx
            .debug_bounds("cancel-turn")
            .expect("the composer retains Stop");
        cx.simulate_click(stop.center(), gpui::Modifiers::none());
        assert!(matches!(
            commands.try_recv().unwrap(),
            ClientMessage::Cancel
        ));
        shell.update(cx, |shell, cx| {
            shell.model.ui.running = false;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("conversation-working").is_none());
        shell.update(cx, |shell, cx| {
            shell.model.ui.running = true;
            shell.model.ui.transcript.clear();
            shell.sync_markdown_cache();
            cx.notify();
        });
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("conversation-working").is_some(),
            "running metadata can arrive before history"
        );
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn required_actions_replace_working_and_partial_failures_keep_retry(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        let (sender, mut commands) = tokio::sync::mpsc::unbounded_channel();
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.command_tx = Some(sender);
            shell.model.ui.selected_session = Some("test".into());
            shell.model.attached_session = Some("test".into());
            shell.model.ui.running = true;
            shell.model.ui.transcript = vec![
                TranscriptItem {
                    role: TranscriptRole::User,
                    text: "please check".into(),
                    parts: Vec::new(),
                },
                TranscriptItem {
                    role: TranscriptRole::Assistant,
                    text: "partial response".into(),
                    parts: Vec::new(),
                },
            ];
            shell.sync_markdown_cache();
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(480.), px(760.)));
        cx.run_until_parked();
        assert!(cx.debug_bounds("conversation-working").is_some());
        shell.update(cx, |shell, cx| {
            shell.model.ui.pending_actions.push(PendingAction {
                request_id: "permission".into(),
                kind: ActionKind::Permission {
                    tool_name: "shell".into(),
                    input: serde_json::json!({"command":"cargo test"}),
                },
            });
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("conversation-working").is_none());
        shell.update(cx, |shell, cx| {
            shell.model.ui.pending_actions.clear();
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("conversation-working").is_some());
        shell.update(cx, |shell, cx| {
            shell.model.ui.running = false;
            shell.model.ui.conversations = vec![ConversationItem {
                session_id: "test".into(),
                title: "test".into(),
                cwd: None,
                workspace_path: None,
                last_message_at: None,
                state: mew_protocol::SessionState::Idle,
                last_turn_failed: true,
                failure_detail: Some("connection interrupted".into()),
                unread_completion: false,
                needs_attention: false,
                archived: false,
                pinned: false,
                group_id: None,
            }];
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("conversation-working").is_none());
        assert!(
            cx.debug_bounds("conversation-attention")
                .unwrap()
                .size
                .height
                > px(24.)
        );
        let retry = cx.debug_bounds("retry-attention-turn").unwrap();
        cx.simulate_click(retry.center(), gpui::Modifiers::none());
        assert!(
            matches!(commands.try_recv().unwrap(), ClientMessage::Prompt { text, .. } if text == "please check")
        );
        shell.update(cx, |shell, _| shell.quitting = true);
    }
}

#[cfg(test)]
mod composer_attachment_interaction_tests {
    use super::super::*;

    fn draft_attachments() -> Vec<Attachment> {
        [
            format!("/tmp/{}末尾.txt", "long-é🙂-filename-".repeat(28)),
            "/tmp/second-attachment-資料.txt".into(),
            "/tmp/third.txt".into(),
        ]
        .into_iter()
        .map(|path| Attachment { path, mime: None })
        .collect()
    }

    fn prepare_draft(shell: &Entity<DesktopShell>, cx: &mut gpui::VisualTestContext) {
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.composer = "keep my draft é🙂".into();
            shell.attachments = draft_attachments();
            cx.notify();
        });
    }

    fn assert_composer_focused(shell: &Entity<DesktopShell>, cx: &mut gpui::VisualTestContext) {
        cx.update(|window, cx| {
            assert!(shell.read(cx).composer_focus_handle.is_focused(window));
        });
    }

    #[gpui::test]
    fn attachment_chips_keep_long_unicode_names_and_remove_targets_inside_the_reading_column(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        prepare_draft(&shell, cx);
        for width in [420., 480., 900.] {
            cx.simulate_resize(gpui::size(px(width), px(760.)));
            cx.run_until_parked();
            let composer = cx.debug_bounds("composer").unwrap();
            let row = cx.debug_bounds("attachment-chips").unwrap();
            assert_eq!(row.left(), composer.left());
            assert_eq!(row.right(), composer.right());
            let mut chips = Vec::new();
            for (chip_selector, remove_selector) in [
                ("attachment-0", "remove-attachment-0"),
                ("attachment-1", "remove-attachment-1"),
                ("attachment-2", "remove-attachment-2"),
            ] {
                let chip = cx.debug_bounds(chip_selector).unwrap();
                let remove = cx.debug_bounds(remove_selector).unwrap();
                assert!(chip.left() >= composer.left());
                assert!(
                    chip.right() <= composer.right(),
                    "{chip_selector} overflows at {width}px"
                );
                assert!(chip.top() >= row.top() && chip.bottom() <= row.bottom());
                assert!(remove.left() >= chip.left() && remove.right() <= chip.right());
                assert!(remove.top() >= chip.top() && remove.bottom() <= chip.bottom());
                assert_eq!(remove.size, gpui::size(px(24.), px(24.)));
                chips.push(chip);
            }
            for (index, chip) in chips.iter().enumerate() {
                for other in &chips[index + 1..] {
                    assert!(!chip.intersects(other), "attachment chips overlap");
                }
            }
            assert!(
                chips[2].size.width < px(150.),
                "short chips keep their natural width"
            );
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn attachment_remove_pointer_targets_preserve_other_files_and_the_draft(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        prepare_draft(&shell, cx);
        cx.simulate_resize(gpui::size(px(420.), px(760.)));
        cx.run_until_parked();
        let remove = cx.debug_bounds("remove-attachment-0").unwrap();
        cx.simulate_click(remove.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        cx.update(|_, cx| {
            assert_eq!(shell.read(cx).attachments, draft_attachments()[1..]);
            assert_eq!(shell.read(cx).model.ui.composer, "keep my draft é🙂");
        });
        assert_composer_focused(&shell, cx);
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn attachment_removal_is_tabbable_and_returns_keyboard_focus_to_the_draft(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        prepare_draft(&shell, cx);
        cx.simulate_resize(gpui::size(px(420.), px(760.)));
        cx.run_until_parked();
        for (key, remaining) in [("enter", 2), ("space", 1), ("enter", 0)] {
            cx.update(|window, cx| {
                let focus = shell.read(cx).composer_focus_handle.clone();
                window.focus(&focus, cx);
                window.focus_prev(cx);
            });
            cx.run_until_parked();
            let keystroke = gpui::Keystroke::parse(key).unwrap();
            cx.simulate_event(gpui::KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            cx.simulate_event(gpui::KeyUpEvent { keystroke });
            cx.run_until_parked();
            cx.update(|_, cx| {
                assert_eq!(shell.read(cx).attachments, draft_attachments()[..remaining]);
                assert_eq!(shell.read(cx).model.ui.composer, "keep my draft é🙂");
            });
            assert_composer_focused(&shell, cx);
        }
        assert!(cx.debug_bounds("attachment-chips").is_none());
        shell.update(cx, |shell, _| shell.quitting = true);
    }
}

#[cfg(test)]
mod composer_model_picker_geometry_regressions {
    use super::super::*;

    fn configure_picker(shell: &mut DesktopShell, recent: bool) {
        shell.layout.sidebar_collapsed = true;
        shell.layout.workbench_collapsed = true;
        shell.model.ui.current_provider = Some("picker-qa".into());
        shell.model.ui.model = Some("alpha".into());
        shell.model.ui.models = serde_json::from_value(serde_json::json!([
            {"id":"picker-qa/alpha", "provider":"picker-qa", "model":"alpha",
             "display_name":"Alpha", "thinking_variants":[{"name":"low"},{"name":"high"}]},
            {"id":"picker-qa/beta", "provider":"picker-qa", "model":"beta", "display_name":"Beta"},
            {"id":"picker-qa/gamma", "provider":"picker-qa", "model":"gamma", "display_name":"Gamma"}
        ])).unwrap();
        shell.recent_models = if recent {
            vec!["picker-qa/beta".into(), "picker-qa/alpha".into()]
        } else {
            Vec::new()
        };
    }

    #[gpui::test]
    fn picker_uses_rendered_rows_and_width_for_its_trigger_alignment(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        for width in [420., 900.] {
            cx.simulate_resize(gpui::size(px(width), px(760.)));
            for (recent, effort) in [(false, false), (false, true), (true, false), (true, true)] {
                for query in ["", "beta", "no such model"] {
                    shell.update(cx, |shell, cx| {
                        configure_picker(shell, recent);
                        shell.model.ui.model = Some(if effort { "alpha" } else { "beta" }.into());
                        shell.model_picker_open = true;
                        shell.model_picker_query = query.into();
                        cx.notify();
                    });
                    cx.run_until_parked();
                    let trigger = cx.debug_bounds("model-picker-trigger").unwrap();
                    let popup = cx.debug_bounds("model-picker").unwrap();
                    assert!(popup.left() >= px(8.) && popup.right() <= px(width - 8.));
                    assert!(popup.top() >= px(8.) && popup.bottom() <= px(752.));
                    assert!(
                        popup.bottom() <= trigger.top() - px(8.),
                        "the popup fits above its trigger, width {width}, recent {recent}, query {query:?}: {popup:?}, trigger {trigger:?}"
                    );
                    if trigger.right() - popup.size.width >= px(8.) {
                        assert_eq!(
                            popup.right(),
                            trigger.right(),
                            "the rendered popup width defines right alignment"
                        );
                    }
                    for selector in ["model-picker-search-frame"]
                        .into_iter()
                        .chain(effort.then_some("model-picker-effort"))
                    {
                        let bounds = cx.debug_bounds(selector).unwrap();
                        assert!(bounds.left() >= popup.left() && bounds.right() <= popup.right());
                        assert!(bounds.top() >= popup.top() && bounds.bottom() <= popup.bottom());
                    }
                    assert_eq!(cx.debug_bounds("model-picker-effort").is_some(), effort);
                    shell.update(cx, |shell, _| {
                        if query == "no such model" {
                            assert!(shell.model_picker_rows.is_empty());
                            assert!(shell.model_picker_filtered_indices.is_empty());
                        }
                    });
                }
            }
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn short_picker_keeps_search_effort_and_scrollable_models_inside_window(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            configure_picker(shell, true);
            shell.model_picker_open = true;
            cx.notify();
        });
        for height in [260., 340., 480., 500.] {
            cx.simulate_resize(gpui::size(px(420.), px(height)));
            cx.run_until_parked();
            let popup = cx.debug_bounds("model-picker").unwrap();
            let trigger = cx.debug_bounds("model-picker-trigger").unwrap();
            assert!(
                popup.top() >= px(8.) && popup.bottom() <= px(height - 8.),
                "popup stays within a {height}px window: {popup:?}, trigger {trigger:?}"
            );
            assert!(
                popup.bottom() <= trigger.top() - px(8.),
                "the list shrinks to retain the trigger gap in a {height}px window"
            );
            let search = cx.debug_bounds("model-picker-search-frame").unwrap();
            let effort = cx.debug_bounds("model-picker-effort").unwrap();
            let list = cx.debug_bounds("model-picker-list").unwrap();
            assert_eq!(search.size.height, px(28.));
            assert!(search.top() >= popup.top() && search.bottom() < list.bottom());
            assert!(list.size.height > px(24.));
            assert!(list.bottom() <= effort.top());
            assert!(effort.bottom() <= popup.bottom());
            shell.update(cx, |shell, _| {
                let track = shell.thinking_effort_track_bounds.unwrap();
                assert!(track.top() >= popup.top() && track.bottom() <= popup.bottom());
            });
            cx.simulate_event(gpui::ScrollWheelEvent {
                position: list.center(),
                delta: gpui::ScrollDelta::Pixels(point(px(0.), px(-500.))),
                ..Default::default()
            });
            cx.run_until_parked();
            let last = cx.debug_bounds("model-option-all-2").unwrap();
            assert!(last.top() < list.bottom() && last.bottom() > list.top());
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }
}

#[cfg(test)]
mod composer_picker_navigation_regressions {
    use super::super::*;

    fn prepare(shell: &mut DesktopShell) {
        shell.layout.sidebar_collapsed = true;
        shell.layout.workbench_collapsed = true;
        shell.model.ui.models = serde_json::from_value(serde_json::json!([
            {"id":"first/shared", "provider":"first", "model":"shared", "display_name":"Alpha"},
            {"id":"second/shared", "provider":"second", "model":"shared", "display_name":"Beta é中"},
            {"id":"third/gamma", "provider":"third", "model":"gamma", "display_name":"Gamma"}
        ])).unwrap();
        shell.recent_models = vec!["second/shared".into(), "first/shared".into()];
    }

    fn open(shell: &Entity<DesktopShell>, cx: &mut gpui::VisualTestContext) {
        shell.update(cx, |shell, cx| {
            prepare(shell);
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(420.), px(760.)));
        cx.run_until_parked();
        let trigger = cx.debug_bounds("model-picker-trigger").unwrap();
        cx.simulate_click(trigger.center(), gpui::Modifiers::none());
        cx.run_until_parked();
    }

    #[test]
    fn recent_models_do_not_leave_duplicate_choices_or_an_empty_other_section() {
        let models: Vec<mew_protocol::ModelInfo> = serde_json::from_value(serde_json::json!([
            {"id":"first/shared", "provider":"first", "model":"shared"},
            {"id":"second/shared", "provider":"second", "model":"shared"}
        ]))
        .unwrap();
        let (indices, rows) = build_model_picker_rows(
            &models,
            &[
                "missing/model".into(),
                "second/shared".into(),
                "first/shared".into(),
                "second/shared".into(),
            ],
            "  ",
        );
        assert_eq!(indices, vec![1, 0]);
        assert_eq!(
            rows,
            vec![
                ModelPickerRow::Header("Recent"),
                ModelPickerRow::Model {
                    index: 1,
                    recent: true
                },
                ModelPickerRow::Model {
                    index: 0,
                    recent: true
                },
            ]
        );
    }

    #[gpui::test]
    fn picker_headers_are_compact_and_model_rows_keep_their_reading_height(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        open(&shell, cx);
        let list = cx.debug_bounds("model-picker-list").unwrap();
        let model = cx.debug_bounds("model-option-recent-1").unwrap();
        assert_eq!(model.top() - list.top(), px(24.));
        assert_eq!(model.size.height, px(56.));
        assert!(cx.debug_bounds("model-option-all-0").is_none());
        assert!(cx.debug_bounds("model-option-all-1").is_none());
        assert!(cx.debug_bounds("model-option-all-2").is_some());
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn arrows_skip_section_headers_and_enter_selects_the_highlighted_identity(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        open(&shell, cx);
        for key in ["up", "down", "down", "down", "up", "enter"] {
            cx.simulate_keystrokes(key);
            cx.run_until_parked();
        }
        shell.update(cx, |shell, _| {
            assert_eq!(shell.pending_model, Some(("first".into(), "shared".into())));
            assert!(!shell.model_picker_open);
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn changing_a_query_resets_keyboard_choice_and_empty_results_cannot_submit(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(super::super::platform::set_app_menus);
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        open(&shell, cx);
        for key in ["down", "down"] {
            cx.simulate_keystrokes(key);
        }
        cx.simulate_input("a");
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        shell.update(cx, |shell, _| {
            assert_eq!(shell.pending_model, Some(("first".into(), "shared".into())));
        });
        open(&shell, cx);
        shell.update(cx, |shell, _| shell.pending_model = None);
        cx.simulate_input("no matching model");
        cx.run_until_parked();
        cx.simulate_keystrokes("down");
        cx.simulate_keystrokes("up");
        cx.simulate_keystrokes("enter");
        shell.update(cx, |shell, _| {
            assert!(shell.pending_model.is_none());
            assert!(shell.model_picker_open);
        });
        cx.simulate_keystrokes("cmd-a");
        cx.simulate_input("é中");
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        shell.update(cx, |shell, _| {
            assert_eq!(
                shell.pending_model,
                Some(("second".into(), "shared".into()))
            );
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn marked_model_search_keeps_composition_keys_out_of_model_navigation(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        open(&shell, cx);
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                shell.replace_and_mark_text_in_range(None, "a", Some(1..1), window, cx);
            })
        });
        cx.run_until_parked();
        for key in [
            "down",
            "up",
            "left",
            "right",
            "backspace",
            "delete",
            "enter",
        ] {
            // IME key events do not imply a plain-text newline commit.
            let keystroke = gpui::Keystroke::parse(key).unwrap();
            cx.simulate_event(gpui::KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            });
            cx.simulate_event(gpui::KeyUpEvent { keystroke });
            cx.run_until_parked();
            shell.update(cx, |shell, _| {
                assert!(
                    shell.model_picker_open,
                    "{key} must not close a composing picker"
                );
                assert!(
                    shell.pending_model.is_none(),
                    "{key} must not select a model during composition"
                );
                assert_eq!(shell.model_picker_query, "a");
                assert_eq!(shell.model_picker_marked_range, Some(0..1));
                assert_eq!(
                    shell.model_picker_highlighted_index,
                    Some(0),
                    "{key} belongs to the composing input method"
                );
            });
        }
        cx.update(|window, cx| {
            shell.update(cx, |shell, cx| {
                shell.replace_text_in_range(None, "é中", window, cx);
            })
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        shell.update(cx, |shell, _| {
            assert_eq!(
                shell.pending_model,
                Some(("second".into(), "shared".into()))
            );
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn keyboard_navigation_reveals_models_beyond_the_short_picker_viewport(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.models = serde_json::from_value(serde_json::Value::Array(
                (0..50).map(|index| serde_json::json!({
                    "id":format!("many/model-{index}"), "provider":"many",
                    "model":format!("model-{index}"), "display_name":format!("Model {index}")
                })).collect()
            )).unwrap();
            shell.recent_models.clear();
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(420.), px(340.)));
        cx.run_until_parked();
        let trigger = cx.debug_bounds("model-picker-trigger").unwrap();
        cx.simulate_click(trigger.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        for _ in 0..49 {
            cx.simulate_keystrokes("down");
            cx.run_until_parked();
        }
        let list = cx.debug_bounds("model-picker-list").unwrap();
        let last = cx
            .debug_bounds("model-option-all-49")
            .expect("keyboard target is rendered");
        assert!(last.top() >= list.top() && last.bottom() <= list.bottom());
        cx.simulate_keystrokes("enter");
        shell.update(cx, |shell, _| {
            assert_eq!(
                shell.pending_model,
                Some(("many".into(), "model-49".into()))
            );
            shell.quitting = true;
        });
    }
}

#[cfg(test)]
mod composer_primary_action_regressions {
    use super::super::*;

    fn assert_send_background(
        shell: &Entity<DesktopShell>,
        cx: &mut gpui::VisualTestContext,
        token: &str,
    ) {
        let bounds = cx.debug_bounds("submit-prompt").unwrap();
        cx.update(|window, cx| {
            let expected: gpui::Hsla = theme_rgb(&shell.read(cx).theme, token).into();
            let bounds = bounds.scale(window.scale_factor());
            assert!(
                window
                    .painted_quads()
                    .iter()
                    .any(|quad| quad.bounds == bounds
                        && quad.background.as_solid() == Some(expected)),
                "the send surface must show its availability"
            );
        });
    }

    #[gpui::test]
    fn empty_send_stays_quiet_and_inert_until_the_draft_has_text(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        let (sender, mut commands) = tokio::sync::mpsc::unbounded_channel();
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.command_tx = Some(sender);
            shell.model.attached_session = Some("button-test".into());
            shell.model.ui.selected_session = Some("button-test".into());
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(420.), px(760.)));
        for text in ["", " \n\t", "\u{2003}"] {
            shell.update(cx, |shell, cx| {
                shell.model.ui.composer = text.into();
                shell.attachments = vec![Attachment {
                    path: "/tmp/composer-only-attachment.txt".into(),
                    mime: Some("text/plain".into()),
                }];
                cx.notify();
            });
            cx.run_until_parked();
            assert_send_background(&shell, cx, "muted");
            let send = cx.debug_bounds("submit-prompt").unwrap();
            cx.simulate_click(send.center(), gpui::Modifiers::none());
            assert!(commands.try_recv().is_err());
        }
        shell.update(cx, |shell, cx| {
            shell.attachments.clear();
            shell.model.ui.composer = "a real prompt é中".into();
            cx.notify();
        });
        cx.run_until_parked();
        assert_send_background(&shell, cx, "text.body");
        let send = cx.debug_bounds("submit-prompt").unwrap();
        cx.simulate_click(send.center(), gpui::Modifiers::none());
        assert!(
            matches!(commands.try_recv().unwrap(), ClientMessage::Prompt { text, .. } if text == "a real prompt é中")
        );
        shell.update(cx, |shell, _| {
            assert!(shell.model.ui.composer.is_empty());
            shell.quitting = true;
        });
    }

    #[gpui::test]
    fn running_question_uses_answer_action_while_normal_working_turn_keeps_stop(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        let (sender, mut commands) = tokio::sync::mpsc::unbounded_channel();
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.command_tx = Some(sender);
            shell.model.attached_session = Some("button-test".into());
            shell.model.ui.selected_session = Some("button-test".into());
            shell.model.ui.running = true;
            shell.model.ui.pending_actions = vec![PendingAction {
                request_id: "question-1".into(),
                kind: ActionKind::AskUser {
                    subagent: None,
                    call_id: "ask-1".into(),
                    questions: vec![
                        mew_protocol::Question {
                            prompt: "which layout?".into(),
                            options: Vec::new(),
                        },
                        mew_protocol::Question {
                            prompt: "which width?".into(),
                            options: Vec::new(),
                        },
                    ],
                },
            }];
            shell.model.ui.composer = "three panes\n420 pixels".into();
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(420.), px(760.)));
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("cancel-turn").is_none(),
            "answering must not offer Cancel as its primary action"
        );
        let answer = cx
            .debug_bounds("submit-prompt")
            .expect("a running question has a submit action");
        cx.simulate_click(answer.center(), gpui::Modifiers::none());
        assert!(
            matches!(commands.try_recv().unwrap(), ClientMessage::AskUserResponse { request_id, answers, cancelled: false } if request_id == "question-1" && answers == ["three panes", "420 pixels"])
        );
        assert!(commands.try_recv().is_err());
        cx.run_until_parked();
        assert_send_background(&shell, cx, "muted");
        shell.update(cx, |shell, cx| {
            shell.model.ui.composer = "sidebar\n480 pixels".into();
            cx.notify();
        });
        cx.update(|window, cx| {
            let focus = shell.read(cx).composer_focus_handle.clone();
            window.focus(&focus, cx);
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        assert!(
            matches!(commands.try_recv().unwrap(), ClientMessage::AskUserResponse { request_id, answers, cancelled: false } if request_id == "question-1" && answers == ["sidebar", "480 pixels"])
        );
        shell.update(cx, |shell, cx| {
            shell.model.ui.pending_actions = vec![PendingAction {
                request_id: "permission-1".into(),
                kind: ActionKind::Permission {
                    tool_name: "shell".into(),
                    input: serde_json::json!({}),
                },
            }];
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("submit-prompt").is_none());
        let stop = cx
            .debug_bounds("cancel-turn")
            .expect("ordinary running turn retains Stop");
        cx.simulate_click(stop.center(), gpui::Modifiers::none());
        assert!(matches!(
            commands.try_recv().unwrap(),
            ClientMessage::Cancel
        ));
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn model_switch_disables_send_and_preserves_the_draft(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        let (sender, mut commands) = tokio::sync::mpsc::unbounded_channel();
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.command_tx = Some(sender);
            shell.model.attached_session = Some("button-test".into());
            shell.model.ui.selected_session = Some("button-test".into());
            shell.model.ui.composer = "keep this draft".into();
            shell.awaiting_model_switch = Some(("provider".into(), "new-model".into()));
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(480.), px(760.)));
        cx.run_until_parked();
        assert_send_background(&shell, cx, "muted");
        let send = cx.debug_bounds("submit-prompt").unwrap();
        cx.simulate_click(send.center(), gpui::Modifiers::none());
        shell.update(cx, |shell, cx| {
            assert_eq!(shell.model.ui.composer, "keep this draft");
            shell.submit_prompt(cx);
            assert!(shell
                .model
                .last_error
                .as_deref()
                .unwrap()
                .contains("still switching"));
            shell.awaiting_model_switch = None;
            cx.notify();
        });
        assert!(commands.try_recv().is_err());
        cx.run_until_parked();
        assert_send_background(&shell, cx, "text.body");
        shell.update(cx, |shell, _| shell.quitting = true);
    }
}

#[cfg(test)]
mod composer_narrow_settings_regressions {
    use super::super::*;

    #[gpui::test]
    fn secondary_pickers_follow_their_triggers_when_the_window_grows(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            cx.notify();
        });
        for (trigger, popup) in [
            ("persona-picker-trigger", "persona-picker"),
            ("permission-picker-trigger", "permission-picker"),
        ] {
            cx.simulate_resize(gpui::size(px(420.), px(600.)));
            cx.run_until_parked();
            let target = cx.debug_bounds(trigger).unwrap();
            cx.simulate_click(target.center(), gpui::Modifiers::none());
            cx.run_until_parked();
            let before = cx.debug_bounds(popup).unwrap();
            cx.simulate_resize(gpui::size(px(420.), px(760.)));
            cx.run_until_parked();
            let after = cx.debug_bounds(popup).unwrap();
            let trigger = cx.debug_bounds(trigger).unwrap();
            assert_eq!(
                after.top() - before.top(),
                px(160.),
                "{popup} follows its current trigger"
            );
            assert!(after.bottom() <= trigger.top() - px(8.));
            shell.update(cx, |shell, cx| {
                shell.persona_picker_open = false;
                shell.permission_picker_open = false;
                cx.notify();
            });
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn settings_stay_compact_until_the_animated_pane_has_room(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(false));
        cx.simulate_resize(gpui::size(px(1000.), px(760.)));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = false;
            shell.conversation_width = 420.;
            shell.model.ui.current_persona = Some("builder".into());
            cx.notify();
        });
        cx.run_until_parked();
        shell.update(cx, |shell, cx| shell.toggle_workbench(cx));
        cx.run_until_parked();
        let pane = cx.debug_bounds("conversation-pane").unwrap();
        assert!(pane.size.width < px(480.));
        assert!(
            cx.debug_bounds("composer-persona-label").is_none(),
            "the label must wait for actual space, even after the workbench is marked collapsed"
        );
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();
        assert!(cx.debug_bounds("conversation-pane").unwrap().size.width >= px(480.));
        assert!(cx.debug_bounds("composer-persona-label").is_some());
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn settings_follow_the_conversation_width_and_keep_effort_visible(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.model.ui.current_persona = Some("builder".into());
            shell.model.ui.model = Some("Claude Sonnet 4.5".into());
            shell.model.ui.thinking_variant = Some("high".into());
            cx.notify();
        });
        for split in [false, true] {
            for width in [420., 479., 480., 600., 420.] {
                cx.simulate_resize(gpui::size(px(if split { 1280. } else { width }), px(760.)));
                shell.update(cx, |shell, cx| {
                    shell.layout.sidebar_collapsed = !split;
                    shell.sidebar_width = 264.;
                    shell.layout.workbench_collapsed = !split;
                    shell.conversation_width = width;
                    cx.notify();
                });
                cx.run_until_parked();
                let pane = cx.debug_bounds("conversation-pane").unwrap();
                assert_eq!(pane.size.width, px(width));
                let persona = cx.debug_bounds("persona-picker-trigger").unwrap();
                if width < 480. {
                    assert_eq!(
                        persona.size.width,
                        px(26.),
                        "secondary persona control stays compact at {width}px"
                    );
                    assert!(cx.debug_bounds("composer-persona-label").is_none());
                } else {
                    assert!(
                        cx.debug_bounds("composer-persona-label")
                            .unwrap()
                            .size
                            .width
                            > px(30.)
                    );
                }
                let model = cx.debug_bounds("model-picker-trigger").unwrap();
                let name = cx.debug_bounds("composer-model-label").unwrap();
                let effort = cx
                    .debug_bounds("composer-effort-label")
                    .expect("effort has its own untruncated label");
                assert!(
                    name.size.width >= px(90.),
                    "the selected model remains identifiable"
                );
                assert!(effort.size.width >= px(24.));
                assert!(name.right() <= effort.left() && effort.right() < model.right());
                shell.update(cx, |shell, _| {
                    assert_eq!(shell.model_picker_bounds, Some(model))
                });
            }
        }
        shell.update(cx, |shell, cx| {
            shell.model.ui.model = Some(
                "a-long-model-name-which-must-truncate-before-it-can-hide-the-effort-é中".into(),
            );
            cx.notify();
        });
        cx.run_until_parked();
        let model = cx.debug_bounds("model-picker-trigger").unwrap();
        let effort = cx.debug_bounds("composer-effort-label").unwrap();
        assert!(effort.size.width >= px(24.) && effort.right() < model.right());
        shell.update(cx, |shell, cx| {
            shell.model.ui.thinking_variant = None;
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("composer-effort-label").is_none());
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn compact_persona_keeps_its_picker_and_keyboard_action(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.current_persona = Some("builder".into());
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(420.), px(760.)));
        cx.run_until_parked();
        let trigger = cx.debug_bounds("persona-picker-trigger").unwrap();
        assert_eq!(trigger.size.width, px(26.));
        cx.simulate_click(trigger.center(), gpui::Modifiers::none());
        cx.run_until_parked();
        shell.update(cx, |shell, cx| {
            assert!(shell.persona_picker_open);
            shell.persona_picker_open = false;
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let focus = shell.read(cx).composer_focus_handle.clone();
            window.focus(&focus, cx);
            for _ in 0..3 {
                window.focus_next(cx);
            }
        });
        cx.run_until_parked();
        let keystroke = gpui::Keystroke::parse("enter").unwrap();
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui::KeyUpEvent { keystroke });
        cx.run_until_parked();
        shell.update(cx, |shell, _| {
            assert!(
                shell.persona_picker_open,
                "the compact control is keyboard accessible"
            );
            shell.quitting = true;
        });
    }
}

#[cfg(test)]
mod composer_question_response_regressions {
    use super::super::*;

    fn request(id: &str, count: usize) -> PendingAction {
        PendingAction {
            request_id: id.into(),
            kind: ActionKind::AskUser {
                subagent: None,
                call_id: format!("call-{id}"),
                questions: (0..count)
                    .map(|index| mew_protocol::Question {
                        prompt: format!("Question {}", index + 1),
                        options: Vec::new(),
                    })
                    .collect(),
            },
        }
    }

    fn configure(shell: &mut DesktopShell) {
        shell.layout.sidebar_collapsed = true;
        shell.layout.workbench_collapsed = true;
        shell.model.attached_session = Some("question-test".into());
        shell.model.ui.selected_session = Some("question-test".into());
        shell.model.ui.running = true;
    }

    #[gpui::test]
    fn one_question_preserves_every_paragraph_for_pointer_and_enter(cx: &mut gpui::TestAppContext) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        let (sender, mut commands) = tokio::sync::mpsc::unbounded_channel();
        shell.update(cx, |shell, cx| {
            configure(shell);
            shell.command_tx = Some(sender);
            shell.model.ui.pending_actions = vec![request("one-question", 1)];
            cx.notify();
        });
        cx.simulate_resize(gpui::size(px(420.), px(760.)));
        for pointer in [true, false] {
            shell.update(cx, |shell, cx| {
                shell.model.ui.composer = " first é中\n\n  second paragraph\nthird\t ".into();
                cx.notify();
            });
            cx.run_until_parked();
            if pointer {
                let send = cx.debug_bounds("submit-prompt").unwrap();
                cx.simulate_click(send.center(), gpui::Modifiers::none());
            } else {
                cx.update(|window, cx| {
                    let focus = shell.read(cx).composer_focus_handle.clone();
                    window.focus(&focus, cx);
                });
                cx.run_until_parked();
                cx.simulate_keystrokes("enter");
            }
            assert!(matches!(commands.try_recv().unwrap(),
                ClientMessage::AskUserResponse { request_id, answers, cancelled: false }
                if request_id == "one-question" && answers == ["first é中\n\n  second paragraph\nthird"]
            ));
            assert!(commands.try_recv().is_err());
            shell.update(cx, |shell, _| assert!(shell.model.ui.composer.is_empty()));
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn multiple_questions_reject_missing_blank_and_extra_answers_without_losing_draft(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        let (sender, mut commands) = tokio::sync::mpsc::unbounded_channel();
        shell.update(cx, |shell, _| {
            configure(shell);
            shell.command_tx = Some(sender);
        });
        cx.simulate_resize(gpui::size(px(420.), px(760.)));
        for (count, draft) in [
            (2, "first\nsecond\nextra"),
            (2, "only one"),
            (3, "first\n \nthird"),
        ] {
            shell.update(cx, |shell, cx| {
                shell.model.ui.pending_actions = vec![request("several-questions", count)];
                shell.model.ui.composer = draft.into();
                cx.notify();
            });
            cx.run_until_parked();
            let send = cx.debug_bounds("submit-prompt").unwrap();
            cx.simulate_click(send.center(), gpui::Modifiers::none());
            assert!(
                commands.try_recv().is_err(),
                "invalid answers must not be sent"
            );
            shell.update(cx, |shell, _| assert_eq!(shell.model.ui.composer, draft));
            assert!(cx
                .debug_bounds("question-answer-validation-several-questions")
                .is_some());
        }
        shell.update(cx, |shell, cx| {
            shell.model.ui.pending_actions = vec![request("several-questions", 2)];
            shell.model.ui.composer = " first é中 \r\n second ".into();
            cx.notify();
        });
        cx.run_until_parked();
        assert!(cx
            .debug_bounds("question-answer-validation-several-questions")
            .is_none());
        let send = cx.debug_bounds("submit-prompt").unwrap();
        cx.simulate_click(send.center(), gpui::Modifiers::none());
        assert!(matches!(commands.try_recv().unwrap(),
            ClientMessage::AskUserResponse { request_id, answers, cancelled: false }
            if request_id == "several-questions" && answers == ["first é中", "second"]
        ));
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn question_cancel_is_scoped_and_preserves_the_draft_for_main_and_child_requests(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        let (sender, mut commands) = tokio::sync::mpsc::unbounded_channel();
        shell.update(cx, |shell, _| {
            configure(shell);
            shell.command_tx = Some(sender);
        });
        cx.simulate_resize(gpui::size(px(420.), px(760.)));
        for (id, selector, child) in [
            (
                "question-main",
                "action-question-main-close-question",
                false,
            ),
            (
                "question-child",
                "action-question-child-close-question",
                true,
            ),
        ] {
            shell.update(cx, |shell, cx| {
                let mut action = request(id, 1);
                if let ActionKind::AskUser { subagent, .. } = &mut action.kind {
                    *subagent = child.then(|| mew_protocol::SubagentQuestionSource {
                        parent_call_id: "parent-call".into(),
                        child_session_id: "child-session".into(),
                        name: "researcher".into(),
                        display_name: Some("Researcher".into()),
                    });
                }
                shell.model.ui.pending_actions = vec![action];
                shell.model.ui.composer = "keep these notes\nand this paragraph".into();
                cx.notify();
            });
            cx.run_until_parked();
            let cancel = cx
                .debug_bounds(selector)
                .expect("every question has a cancel action");
            cx.simulate_click(cancel.center(), gpui::Modifiers::none());
            assert!(matches!(commands.try_recv().unwrap(),
                ClientMessage::AskUserResponse { request_id, answers, cancelled: true }
                if request_id == id && answers.is_empty()
            ));
            assert!(commands.try_recv().is_err());
            shell.update(cx, |shell, _| {
                assert_eq!(
                    shell.model.ui.composer,
                    "keep these notes\nand this paragraph"
                );
                assert!(shell.model.ui.running);
            });
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }

    #[gpui::test]
    fn required_actions_stay_above_the_outer_composer_for_short_and_long_drafts(
        cx: &mut gpui::TestAppContext,
    ) {
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            configure(shell);
            shell.model.ui.pending_actions = vec![request("geometry-question", 1)];
            cx.notify();
        });
        for width in [420., 480.] {
            cx.simulate_resize(gpui::size(px(width), px(800.)));
            for long in [false, true] {
                shell.update(cx, |shell, cx| {
                    shell.model.ui.composer = if long {
                        "draft line\n".repeat(30)
                    } else {
                        "short draft".into()
                    };
                    shell.composer_expanded = false;
                    cx.notify();
                });
                cx.run_until_parked();
                let outer = cx.debug_bounds("composer").unwrap();
                let actions = cx.debug_bounds("required-actions").unwrap();
                assert_eq!(actions.left(), outer.left());
                assert_eq!(actions.size.width, outer.size.width);
                assert!(actions.bottom() <= outer.top() - px(8.), "requests retain a gap above the complete composer, width {width}, long {long}: {actions:?}, {outer:?}");
                if long {
                    if let Some(expand) = cx.debug_bounds("composer-expand-toggle") {
                        cx.simulate_click(expand.center(), gpui::Modifiers::none());
                        cx.run_until_parked();
                        let outer = cx.debug_bounds("composer").unwrap();
                        let actions = cx.debug_bounds("required-actions").unwrap();
                        assert!(actions.bottom() <= outer.top() - px(8.));
                    }
                }
            }
        }
        shell.update(cx, |shell, _| shell.quitting = true);
    }
}

#[cfg(test)]
mod transcript_scrollbar_tiny_viewport_regressions {
    use super::super::chat_render::transcript_scrollbar_metrics;
    use super::super::*;

    struct MeasuredList {
        state: gpui::ListState,
        height: Pixels,
    }

    impl Render for MeasuredList {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().child(
                gpui::list(self.state.clone(), |_, _, _| {
                    div()
                        .h(px(100.))
                        .child("scrollable content")
                        .into_any_element()
                })
                .w(px(200.))
                .h(self.height),
            )
        }
    }

    #[gpui::test]
    fn transcript_scrollbar_stays_inside_tiny_measured_viewports(cx: &mut gpui::TestAppContext) {
        let (view, cx) = cx.add_window_view(|_, _| MeasuredList {
            state: gpui::ListState::new(2, gpui::ListAlignment::Top, px(200.)),
            height: px(100.),
        });
        for height in [0., 1., 2., 13.5, 27., 28., 29., 100., 200.] {
            view.update(cx, |view, cx| {
                view.height = px(height);
                cx.notify();
            });
            cx.run_until_parked();
            view.update(cx, |view, _| {
                let viewport = f32::from(view.state.viewport_bounds().size.height);
                assert!((viewport - height).abs() <= 0.5);
                let metrics = transcript_scrollbar_metrics(&view.state);
                if height <= 1. || height == 200. {
                    assert!(metrics.is_none());
                } else {
                    let metrics = metrics.unwrap_or_else(|| panic!(
                        "overflowing content retains scrollbar metrics, height {height}, viewport {viewport}, max offset {:?}",
                        view.state.max_offset_for_scrollbar()
                    ));
                    assert!(metrics.thumb_height > 0. && metrics.thumb_height <= viewport);
                    assert!(metrics.thumb_top >= 0.);
                    assert!(metrics.thumb_top + metrics.thumb_height <= viewport);
                    assert!(metrics.travel >= 0.);
                    if height < 28. {
                        assert_eq!(metrics.thumb_height, viewport);
                        assert_eq!(metrics.travel, 0.);
                    } else {
                        assert!(metrics.thumb_height >= 28.);
                    }
                }
            });
        }
    }

    #[gpui::test]
    fn adding_eight_wrapped_attachments_keeps_tiny_transcript_viewports_safe(
        cx: &mut gpui::TestAppContext,
    ) {
        let persisted_state = mew_config::load_state().unwrap();
        let (shell, cx) =
            cx.add_window_view(|window, cx| DesktopShell::new(None, None, None, None, window, cx));
        cx.update(|_, cx| cx.set_reduce_motion(true));
        shell.update(cx, |shell, cx| {
            shell.layout.sidebar_collapsed = true;
            shell.layout.workbench_collapsed = true;
            shell.model.ui.transcript = (0..16).map(|index| TranscriptItem {
                role: TranscriptRole::Assistant,
                text: format!("Paragraph {index}: retained conversation content that overflows the remaining viewport."),
                parts: Vec::new(),
            }).collect();
            shell.model.ui.composer = "draft line with preserved details\n".repeat(12);
            let end = shell.model.ui.composer.len();
            shell.composer_selection = end..end;
            shell.sync_markdown_cache();
            cx.notify();
        });
        let attachments = (0..8).map(|index| Attachment {
            path: format!("/tmp/{index}-a-very-long-attachment-filename-which-takes-one-row-in-the-narrow-composer-é中.txt"),
            mime: Some("text/plain".into()),
        }).collect::<Vec<_>>();
        let mut saw_tiny_viewport = false;
        for width in [420., 480.] {
            cx.simulate_resize(gpui::size(px(width), px(480.)));
            for count in 0..=8 {
                shell.update(cx, |shell, cx| {
                    shell.attachments = attachments[..count].to_vec();
                    cx.notify();
                });
                cx.run_until_parked();
                shell.update(cx, |shell, _| {
                    let viewport = shell.transcript_list.viewport_bounds();
                    let height = f32::from(viewport.size.height);
                    saw_tiny_viewport |= height > 1. && height < 28.;
                    if let Some(metrics) = transcript_scrollbar_metrics(&shell.transcript_list) {
                        assert!(metrics.thumb_height <= height);
                        assert!(metrics.thumb_top + metrics.thumb_height <= height);
                    }
                });
            }
        }
        assert!(
            saw_tiny_viewport,
            "the crowded composer must exercise the reported tiny viewport"
        );
        shell.update(cx, |shell, _| shell.quitting = true);
        mew_config::save_state(&persisted_state).unwrap();
    }
}
