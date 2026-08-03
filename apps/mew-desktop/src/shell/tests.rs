#[cfg(test)]
mod shell_tests {
    use super::super::composer::composer_input_height;
    use super::super::session_data::transcript_part_block_count;
    use super::super::*;

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
    fn picker_viewports_match_and_stay_bounded() {
        assert_eq!(model_picker_list_height(0), px(64.));
        assert_eq!(model_picker_list_height(5), px(320.));
        assert_eq!(model_picker_list_height(32), px(320.));
        assert_eq!(model_picker_height(32), px(336.));
        assert_eq!(model_picker_list_height(32), persona_picker_list_height(32));
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
            streaming: None,
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
        let conversations = [ConversationItem {
            session_id: "session-1".into(),
            title: "A session".into(),
            cwd: Some("/Users/natalie/code/mew".into()),
            last_message_at: None,
            state: mew_protocol::SessionState::Idle,
            last_turn_failed: false,
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
    }

    #[test]
    fn activity_events_request_full_session_snapshots() {
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
                    }),
                    continuation: false,
                    syntax_highlights: Vec::new(),
                };
                3
            ],
            streaming: None,
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
        assert_eq!(sidebar_transition_width(true, 0.0), SIDEBAR_EXPANDED_WIDTH);
        assert_eq!(sidebar_transition_width(true, 1.0), SIDEBAR_COLLAPSED_WIDTH);
        assert_eq!(
            sidebar_transition_width(false, 0.0),
            SIDEBAR_COLLAPSED_WIDTH
        );
        assert_eq!(sidebar_transition_width(false, 1.0), SIDEBAR_EXPANDED_WIDTH);
    }

    #[test]
    fn sidebar_transition_moves_the_surface_offscreen() {
        assert_eq!(sidebar_transition_offset(true, 0.0), 0.);
        assert_eq!(
            sidebar_transition_offset(true, 1.0),
            -SIDEBAR_EXPANDED_WIDTH
        );
        assert_eq!(
            sidebar_transition_offset(false, 0.0),
            -SIDEBAR_EXPANDED_WIDTH
        );
        assert_eq!(sidebar_transition_offset(false, 1.0), 0.);
    }

    #[test]
    fn workbench_transition_reaches_both_panel_widths() {
        assert_eq!(WORKBENCH_COLLAPSED_WIDTH, 0.);
        assert_eq!(
            workbench_transition_width(true, 0.0),
            WORKBENCH_EXPANDED_WIDTH
        );
        assert_eq!(
            workbench_transition_width(true, 1.0),
            WORKBENCH_COLLAPSED_WIDTH
        );
        assert_eq!(
            workbench_transition_width(false, 0.0),
            WORKBENCH_COLLAPSED_WIDTH
        );
        assert_eq!(
            workbench_transition_width(false, 1.0),
            WORKBENCH_EXPANDED_WIDTH
        );
    }

    #[test]
    fn workbench_transition_moves_the_surface_offscreen() {
        let expanded_width = 512.;
        assert_eq!(workbench_transition_offset(true, 0.0, expanded_width), 0.);
        assert_eq!(
            workbench_transition_offset(true, 1.0, expanded_width),
            -expanded_width
        );
        assert_eq!(
            workbench_transition_offset(false, 0.0, expanded_width),
            -expanded_width
        );
        assert_eq!(workbench_transition_offset(false, 1.0, expanded_width), 0.);
    }

    #[test]
    fn workbench_resize_stays_within_usable_width_bounds() {
        assert_eq!(workbench_max_width(1440., SIDEBAR_EXPANDED_WIDTH), 756.);
        assert_eq!(
            workbench_width_from_pointer(1440., 1072., SIDEBAR_EXPANDED_WIDTH),
            360.
        );
        assert_eq!(
            workbench_width_from_pointer(1440., 0., SIDEBAR_EXPANDED_WIDTH),
            756.
        );
        assert_eq!(
            workbench_width_from_pointer(1440., 1400., SIDEBAR_EXPANDED_WIDTH),
            WORKBENCH_MIN_WIDTH
        );
    }

    #[test]
    fn workbench_resize_respects_the_available_width_on_narrow_windows() {
        assert_eq!(workbench_max_width(720., SIDEBAR_EXPANDED_WIDTH), 36.);
        assert_eq!(
            workbench_width_from_pointer(720., 500., SIDEBAR_EXPANDED_WIDTH),
            36.
        );
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
    fn tab_shortcuts_require_platform_modifier() {
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
            Some(ShellCommand::CloseActiveTab)
        );
        assert_eq!(
            shell_command_for_key("2", true),
            Some(ShellCommand::SelectTab(1))
        );
        assert_eq!(shell_command_for_key("0", true), None);
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

        history.record("first");
        history.record("second");
        history.record("second");
        history.record("  ");

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
        history.record("one");
        history.recall_older("");
        assert!(history.is_recalling());
        history.record("two");
        assert!(!history.is_recalling());

        for index in 0..PROMPT_HISTORY_LIMIT + 10 {
            history.record(&format!("prompt-{index}"));
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
            last_message_at,
            state: mew_protocol::SessionState::Idle,
            last_turn_failed: false,
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
