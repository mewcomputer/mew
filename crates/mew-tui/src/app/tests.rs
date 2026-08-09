use super::*;

#[test]
fn test_parse_file_mentions_basic() {
    let text = "fix the bug in @src/main.rs";
    let mentions = parse_file_mentions(text);
    assert_eq!(mentions, vec!["src/main.rs"]);
}

#[test]
fn test_parse_file_mentions_multiple() {
    let text = "compare @a.txt and @b.txt";
    let mentions = parse_file_mentions(text);
    assert_eq!(mentions, vec!["a.txt", "b.txt"]);
}

#[test]
fn test_parse_file_mentions_with_punctuation() {
    let text = "check @README.md, then @Cargo.toml.";
    let mentions = parse_file_mentions(text);
    assert_eq!(mentions, vec!["README.md", "Cargo.toml"]);
}

#[test]
fn test_parse_file_mentions_none() {
    let text = "no mentions here";
    let mentions = parse_file_mentions(text);
    assert!(mentions.is_empty());
}

// --- Skill reference parser tests ---

#[test]
fn test_parse_namespace_refs_skill() {
    let refs = parse_namespace_refs("use @skill:clarify for this");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].kind, "skill");
    assert_eq!(refs[0].value, "clarify");
    assert_eq!(refs[0].raw, "@skill:clarify");
}

#[test]
fn test_parse_namespace_refs_model() {
    let refs = parse_namespace_refs("use @model:openai/gpt-4o");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].kind, "model");
    assert_eq!(refs[0].value, "openai/gpt-4o");
    assert_eq!(refs[0].raw, "@model:openai/gpt-4o");
}

#[test]
fn test_parse_namespace_refs_subagent() {
    let refs = parse_namespace_refs("spawn @subagent:researcher");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].kind, "subagent");
    assert_eq!(refs[0].value, "researcher");
    assert_eq!(refs[0].raw, "@subagent:researcher");
}

#[test]
fn test_parse_namespace_refs_multiple() {
    let refs = parse_namespace_refs("@skill:clarify and @model:openai/gpt-4o and @subagent:coder");
    assert_eq!(refs.len(), 3);
    assert_eq!(refs[0].kind, "skill");
    assert_eq!(refs[1].kind, "model");
    assert_eq!(refs[2].kind, "subagent");
}

#[test]
fn test_parse_namespace_refs_none() {
    assert!(parse_namespace_refs("no refs here").is_empty());
    assert!(parse_namespace_refs("@src/main.rs is a file mention").is_empty());
}

#[test]
fn test_parse_namespace_refs_trailing_punct() {
    let refs = parse_namespace_refs("use @skill:clarify.");
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].value, "clarify");
}

#[test]
fn test_file_mentions_skip_namespaces() {
    let mentions = parse_file_mentions(
        "@skill:clarify @model:openai/gpt-4o @subagent:researcher @src/main.rs",
    );
    assert_eq!(mentions, vec!["src/main.rs"]);
}

#[test]
fn test_builtin_slash_commands_has_core_commands() {
    let cmds = App::builtin_slash_commands();
    let names: Vec<&str> = cmds.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"/help"));
    assert!(names.contains(&"/clear"));
    assert!(names.contains(&"/quit"));
    assert!(names.contains(&"/permissions"));
}

#[test]
fn test_permissions_slash_with_no_arg_opens_picker() {
    let app = App::new();
    let result = app.handle_slash("/permissions");
    assert!(matches!(result, SlashResult::PermissionModeMenu));
}

#[test]
fn test_permissions_slash_with_dangerous_arg() {
    let app = App::new();
    let result = app.handle_slash("/permissions dangerous");
    assert!(matches!(
        result,
        SlashResult::SetPermissionMode(mew_hooks::PermissionMode::Dangerous)
    ));
}

#[test]
fn test_permissions_slash_with_standard_arg() {
    let app = App::new();
    let result = app.handle_slash("/permissions standard");
    assert!(matches!(
        result,
        SlashResult::SetPermissionMode(mew_hooks::PermissionMode::Standard)
    ));
}

#[test]
fn test_permissions_slash_with_permissive_arg() {
    let app = App::new();
    let result = app.handle_slash("/permissions permissive");
    assert!(matches!(
        result,
        SlashResult::SetPermissionMode(mew_hooks::PermissionMode::Permissive)
    ));
}

#[test]
fn test_permissions_slash_with_auto_arg() {
    let app = App::new();
    let result = app.handle_slash("/permissions auto");
    assert!(matches!(
        result,
        SlashResult::SetPermissionMode(mew_hooks::PermissionMode::Auto)
    ));
}

#[test]
fn test_permissions_slash_with_auto_plus_arg() {
    let app = App::new();
    let result = app.handle_slash("/permissions auto_plus");
    assert!(matches!(
        result,
        SlashResult::SetPermissionMode(mew_hooks::PermissionMode::AutoPlus)
    ));
}

#[test]
fn test_permission_mode_picker_has_five_items() {
    let mut app = App::new();
    app.open_permission_mode_picker();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.items.len(), 5, "picker should show all five modes");
    let ids: Vec<&str> = picker.items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["standard", "permissive", "auto", "auto_plus", "dangerous",],
        "modes ordered from most-restrictive to least"
    );
}

#[test]
fn test_permission_mode_picker_preselects_autoplus() {
    let mut app = App::new();
    app.permission_mode = mew_hooks::PermissionMode::AutoPlus;
    app.open_permission_mode_picker();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(
        picker.selected, 3,
        "AutoPlus should pre-select index 3 (between Auto and Dangerous!)"
    );
    let autoplus = picker.items.iter().find(|i| i.id == "auto_plus").unwrap();
    assert!(
        autoplus.label.contains("● active"),
        "active marker on Auto+ row: {:?}",
        autoplus.label
    );
}

#[test]
fn test_permission_mode_picker_preselects_permissive() {
    let mut app = App::new();
    app.permission_mode = mew_hooks::PermissionMode::Permissive;
    app.open_permission_mode_picker();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(
        picker.selected, 1,
        "Permissive should pre-select index 1 (middle item)"
    );
    let permissive = picker.items.iter().find(|i| i.id == "permissive").unwrap();
    assert!(
        permissive.label.contains("● active"),
        "active marker on Permissive row: {:?}",
        permissive.label
    );
}

#[test]
fn test_permissions_slash_with_unknown_arg_errors() {
    let app = App::new();
    let result = app.handle_slash("/permissions banana");
    assert!(matches!(result, SlashResult::Message(_)));
}

#[test]
fn test_permission_mode_picker_marks_active_mode() {
    let mut app = App::new();
    app.permission_mode = mew_hooks::PermissionMode::Dangerous;
    app.open_permission_mode_picker();
    let picker = app.picker.as_ref().expect("picker opened");
    assert_eq!(picker.kind, "permission_mode");
    let dangerous = picker
        .items
        .iter()
        .find(|i| i.id == "dangerous")
        .expect("dangerous item");
    assert!(
        dangerous.label.contains("● active"),
        "active mode should be marked: {:?}",
        dangerous.label
    );
    let standard = picker.items.iter().find(|i| i.id == "standard").unwrap();
    assert!(!standard.label.contains("● active"));
}

#[test]
fn test_permission_mode_picker_preselects_active() {
    let mut app = App::new();
    app.permission_mode = mew_hooks::PermissionMode::Dangerous;
    app.open_permission_mode_picker();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(
        picker.selected, 4,
        "Dangerous index should be 4 (fifth item in slider with Auto and Auto+)"
    );
}

#[test]
fn test_all_slash_commands_includes_dynamic() {
    let mut app = App::new();
    app.add_dynamic_slash_commands(vec![SlashCommand {
        name: "/buddy".into(),
        description: "pet companion".into(),
    }]);
    let all = app.all_slash_commands();
    let names: Vec<&str> = all.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"/buddy"));
    assert!(names.contains(&"/help"));
}

#[test]
fn test_all_slash_commands_no_dynamic_still_has_builtins() {
    let app = App::new();
    let all = app.all_slash_commands();
    let builtins = App::builtin_slash_commands();
    assert_eq!(all.len(), builtins.len());
}

#[test]
fn test_handle_slash_routes_dynamic_command() {
    let mut app = App::new();
    app.add_dynamic_slash_commands(vec![SlashCommand {
        name: "/buddy".into(),
        description: "pet companion".into(),
    }]);
    let result = app.handle_slash("/buddy pet");
    match result {
        SlashResult::PluginCommand { name, args } => {
            assert_eq!(name, "/buddy");
            assert_eq!(args, "pet");
        }
        _ => panic!("expected PluginCommand, got {:?}", result),
    }
}

#[test]
fn test_handle_slash_unknown_dynamic_command_continues() {
    let mut app = App::new();
    app.add_dynamic_slash_commands(vec![SlashCommand {
        name: "/buddy".into(),
        description: "pet companion".into(),
    }]);
    let result = app.handle_slash("/nonexistent");
    assert!(matches!(result, SlashResult::Continue));
}

#[test]
fn test_handle_slash_dynamic_command_without_args() {
    let mut app = App::new();
    app.add_dynamic_slash_commands(vec![SlashCommand {
        name: "/stats".into(),
        description: "show stats".into(),
    }]);
    let result = app.handle_slash("/stats");
    match result {
        SlashResult::PluginCommand { name, args } => {
            assert_eq!(name, "/stats");
            assert_eq!(args, "");
        }
        _ => panic!("expected PluginCommand"),
    }
}

#[test]
fn test_add_dynamic_slash_commands_accumulates() {
    let mut app = App::new();
    app.add_dynamic_slash_commands(vec![SlashCommand {
        name: "/foo".into(),
        description: "first".into(),
    }]);
    app.add_dynamic_slash_commands(vec![SlashCommand {
        name: "/bar".into(),
        description: "second".into(),
    }]);
    let all = app.all_slash_commands();
    let names: Vec<&str> = all.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"/foo"));
    assert!(names.contains(&"/bar"));
}

#[test]
fn test_filtered_slash_commands_includes_dynamic() {
    let mut app = App::new();
    app.add_dynamic_slash_commands(vec![SlashCommand {
        name: "/buddy".into(),
        description: "pet companion".into(),
    }]);
    app.input = "/bud".to_string();
    let filtered = app.filtered_slash_commands();
    assert!(!filtered.is_empty());
    assert!(filtered.iter().any(|c| c.name == "/buddy"));
}

#[test]
fn test_plugin_ui_starts_empty() {
    let app = App::new();
    assert!(app.plugin_ui.is_empty());
}

#[test]
fn test_plugin_ui_can_store_values() {
    let mut app = App::new();
    app.plugin_ui
        .insert("buddy/sprite".into(), "(\u{b7}>".into());
    assert_eq!(app.plugin_ui.get("buddy/sprite").unwrap(), "(\u{b7}>");
}

#[test]
fn test_filtered_slash_commands_no_match_returns_empty() {
    let app = App::new();
    // Set input to a prefix that matches nothing
    let mut app = app;
    app.input = "/zzz".to_string();
    let filtered = app.filtered_slash_commands();
    assert!(filtered.is_empty(), "no commands should match /zzz");
}

#[test]
fn test_filtered_slash_commands_non_slash_no_results() {
    let mut app = App::new();
    app.input = "hello".to_string();
    let filtered = app.filtered_slash_commands();
    assert!(filtered.is_empty(), "non-slash input returns empty");
}

#[test]
fn test_dynamic_slash_commands_uses_all_slash_for_autocomplete() {
    let mut app = App::new();
    app.add_dynamic_slash_commands(vec![
        SlashCommand {
            name: "/buddy".into(),
            description: "buddy".into(),
        },
        SlashCommand {
            name: "/stats".into(),
            description: "stats".into(),
        },
    ]);
    app.input = "/bu".to_string();
    let filtered = app.filtered_slash_commands();
    assert!(!filtered.is_empty());
    let names: Vec<&str> = filtered.iter().map(|c| c.name.as_str()).collect();
    assert!(
        !names.contains(&"/stats"),
        "/stats should not match /bu prefix"
    );
}

#[test]
fn test_subagent_status_event_stores_progress() {
    use mew_agent::AgentEvent;
    let mut app = App::new();

    // Subagent starts.
    app.handle_agent_event(AgentEvent::SubagentStart {
        parent_call_id: "task-1".into(),
        name: "researcher".into(),
        child_session_id: "child-1".into(),
        display_name: Some("Curie".into()),
    });
    assert_eq!(app.subagents.len(), 1);
    assert_eq!(app.subagents[0].display_name.as_deref(), Some("Curie"));
    assert!(app.subagents[0].last_progress.is_none());

    // Subagent reports its first status.
    app.handle_agent_event(AgentEvent::SubagentStatus {
        parent_call_id: "task-1".into(),
        tool_name: "progress_update".into(),
        message: "scanning the repo".into(),
    });
    assert_eq!(
        app.subagents[0].last_progress.as_deref(),
        Some("scanning the repo")
    );

    // A second status replaces the first.
    app.handle_agent_event(AgentEvent::SubagentStatus {
        parent_call_id: "task-1".into(),
        tool_name: "progress_update".into(),
        message: "writing the report".into(),
    });
    assert_eq!(
        app.subagents[0].last_progress.as_deref(),
        Some("writing the report")
    );

    // A status for an unknown subagent is ignored.
    app.handle_agent_event(AgentEvent::SubagentStatus {
        parent_call_id: "no-such-task".into(),
        tool_name: "progress_update".into(),
        message: "ignored".into(),
    });
    assert_eq!(
        app.subagents[0].last_progress.as_deref(),
        Some("writing the report")
    );
}

#[test]
fn test_subagent_start_without_display_name_falls_back() {
    use mew_agent::AgentEvent;
    let mut app = App::new();

    // Older callers (or callers that opt out) may not set a
    // display_name. The state should still record the entry, with
    // display_name == None, and the sidebar's "fall back to def
    // name" path takes over.
    app.handle_agent_event(AgentEvent::SubagentStart {
        parent_call_id: "task-1".into(),
        name: "researcher".into(),
        child_session_id: "child-1".into(),
        display_name: None,
    });
    assert_eq!(app.subagents.len(), 1);
    assert_eq!(app.subagents[0].name, "researcher");
    assert!(app.subagents[0].display_name.is_none());
}

#[test]
fn test_ask_user_event_stores_state_and_sets_mode() {
    use mew_agent::{AgentEvent, AskUserQuestion, QuestionOption};
    let mut app = App::new();
    let (tx, _rx) = tokio::sync::oneshot::channel::<Vec<String>>();
    app.handle_agent_event(AgentEvent::AskUser {
        call_id: "c1".into(),
        questions: vec![
            AskUserQuestion {
                prompt: "which branch?".into(),
                options: vec![
                    QuestionOption {
                        label: "main".into(),
                        description: "production".into(),
                    },
                    QuestionOption {
                        label: "dev".into(),
                        description: "".into(),
                    },
                ],
            },
            AskUserQuestion {
                prompt: "confirm?".into(),
                options: vec![
                    QuestionOption {
                        label: "yes".into(),
                        description: "".into(),
                    },
                    QuestionOption {
                        label: "no".into(),
                        description: "".into(),
                    },
                ],
            },
        ],
        tx,
    });
    assert_eq!(app.mode, Mode::UserQuestion);
    let uq = app.user_question.as_ref().expect("question stored");
    assert_eq!(uq.questions.len(), 2);
    assert_eq!(uq.questions[0].prompt, "which branch?");
    assert_eq!(uq.questions[0].options[0].label, "main");
    assert!(uq.answers.iter().all(|a| a.is_empty()));
    assert_eq!(uq.page, 0);
    assert_eq!(uq.selected, 0);
    assert!(!uq.review);
}

#[test]
fn test_single_question_picks_option_and_submits() {
    use mew_agent::{AgentEvent, AskUserQuestion, QuestionOption};
    let mut app = App::new();
    let (tx, mut rx) = tokio::sync::oneshot::channel::<Vec<String>>();
    app.handle_agent_event(AgentEvent::AskUser {
        call_id: "c1".into(),
        questions: vec![AskUserQuestion {
            prompt: "branch?".into(),
            options: vec![
                QuestionOption {
                    label: "main".into(),
                    description: "".into(),
                },
                QuestionOption {
                    label: "dev".into(),
                    description: "".into(),
                },
            ],
        }],
        tx,
    });
    // Move highlight to "dev" and confirm. Single question auto-submits.
    app.user_question_select_next();
    app.user_question_confirm();
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.user_question.is_none());
    let answers = rx.try_recv().expect("answers sent");
    assert_eq!(answers, vec!["dev"]);
}

fn open_plan_approval(app: &mut App) -> tokio::sync::oneshot::Receiver<mew_agent::PlanDecision> {
    use mew_agent::AgentEvent;
    let (tx, rx) = tokio::sync::oneshot::channel::<mew_agent::PlanDecision>();
    app.handle_agent_event(AgentEvent::PlanApprovalRequest {
        call_id: "c1".into(),
        plan_path: "/repo/PLAN.md".into(),
        plan_markdown: "# Goal\n\n1. do it".into(),
        persona: "builder".into(),
        tx,
    });
    rx
}

#[test]
fn test_plan_approval_event_sets_mode() {
    let mut app = App::new();
    let _rx = open_plan_approval(&mut app);
    assert_eq!(app.mode, Mode::PlanApproval);
    let pa = app.plan_approval.as_ref().expect("state stored");
    assert_eq!(pa.persona, "builder");
    assert!(pa.plan_markdown.contains("do it"));
}

#[test]
fn test_plan_approval_approve_sends_decision() {
    let mut app = App::new();
    let mut rx = open_plan_approval(&mut app);
    // selected defaults to 0 (approve).
    app.plan_approval_confirm();
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.plan_approval.is_none());
    match rx.try_recv().expect("decision sent") {
        mew_agent::PlanDecision::Approved => {}
        other => panic!("expected Approved, got {other:?}"),
    }
}

#[test]
fn test_plan_approval_request_changes_flow() {
    let mut app = App::new();
    let mut rx = open_plan_approval(&mut app);
    // Toggle to "request changes"; first confirm enters the feedback editor.
    app.plan_approval_toggle();
    app.plan_approval_confirm();
    assert_eq!(app.mode, Mode::PlanApproval, "still open, now editing");
    assert!(app.plan_approval.as_ref().unwrap().editing_feedback);
    // Empty feedback doesn't submit.
    app.plan_approval_confirm();
    assert!(app.plan_approval.is_some());
    // Type feedback, then confirm.
    for c in "add tests".chars() {
        app.plan_approval_type_char(c);
    }
    app.plan_approval_confirm();
    assert_eq!(app.mode, Mode::Normal);
    match rx.try_recv().expect("decision sent") {
        mew_agent::PlanDecision::ChangesRequested(f) => assert_eq!(f, "add tests"),
        other => panic!("expected ChangesRequested, got {other:?}"),
    }
}

#[test]
fn test_plan_approval_cancel_drops_tx() {
    let mut app = App::new();
    let mut rx = open_plan_approval(&mut app);
    app.cancel_plan_approval();
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.plan_approval.is_none());
    // Dropping tx makes the agent's rx.await return Err.
    assert!(rx.try_recv().is_err());
}

#[test]
fn test_multi_question_goes_to_review_before_submit() {
    use mew_agent::{AgentEvent, AskUserQuestion, QuestionOption};
    let mut app = App::new();
    let (tx, mut rx) = tokio::sync::oneshot::channel::<Vec<String>>();
    app.handle_agent_event(AgentEvent::AskUser {
        call_id: "c1".into(),
        questions: vec![
            AskUserQuestion {
                prompt: "branch?".into(),
                options: vec![
                    QuestionOption {
                        label: "main".into(),
                        description: "".into(),
                    },
                    QuestionOption {
                        label: "dev".into(),
                        description: "".into(),
                    },
                ],
            },
            AskUserQuestion {
                prompt: "scope?".into(),
                options: vec![
                    QuestionOption {
                        label: "minimal".into(),
                        description: "".into(),
                    },
                    QuestionOption {
                        label: "wide".into(),
                        description: "".into(),
                    },
                ],
            },
        ],
        tx,
    });
    // First question: pick "dev" (next from 0), confirm.
    app.user_question_select_next();
    app.user_question_confirm();
    let uq = app.user_question.as_ref().expect("still active");
    assert_eq!(uq.page, 1);
    assert_eq!(uq.selected, 0);
    assert!(!uq.review);
    // Second question: pick "wide" (next twice), confirm. Multi-question
    // should now go to the review page rather than submit.
    app.user_question_select_next();
    app.user_question_confirm();
    let uq = app.user_question.as_ref().expect("still active");
    assert!(uq.review, "should be on the review page");
    assert_eq!(uq.review_selected, 0);
    assert_eq!(uq.answers, vec!["dev", "wide"]);
    assert!(rx.try_recv().is_err(), "should not have submitted yet");
    // Confirm Submit on the review page.
    app.user_question_confirm();
    assert_eq!(app.mode, Mode::Normal);
    let answers = rx.try_recv().expect("answers sent");
    assert_eq!(answers, vec!["dev", "wide"]);
}

#[test]
fn test_freeform_text_commits_via_typing() {
    use mew_agent::{AgentEvent, AskUserQuestion, QuestionOption};
    let mut app = App::new();
    let (tx, mut rx) = tokio::sync::oneshot::channel::<Vec<String>>();
    app.handle_agent_event(AgentEvent::AskUser {
        call_id: "c1".into(),
        questions: vec![AskUserQuestion {
            prompt: "branch?".into(),
            options: vec![
                QuestionOption {
                    label: "main".into(),
                    description: "".into(),
                },
                QuestionOption {
                    label: "dev".into(),
                    description: "".into(),
                },
            ],
        }],
        tx,
    });
    // Two options + freeform = 3 rows. Jump to row 3.
    app.user_question_jump(3);
    app.user_question_type_char('f');
    app.user_question_type_char('o');
    app.user_question_type_char('o');
    app.user_question_backspace();
    app.user_question_confirm();
    let answers = rx.try_recv().expect("answers sent");
    assert_eq!(answers, vec!["fo"]);
}

#[test]
fn test_freeform_does_not_advance_with_empty_text() {
    use mew_agent::{AgentEvent, AskUserQuestion, QuestionOption};
    let mut app = App::new();
    let (tx, _rx) = tokio::sync::oneshot::channel::<Vec<String>>();
    app.handle_agent_event(AgentEvent::AskUser {
        call_id: "c1".into(),
        questions: vec![AskUserQuestion {
            prompt: "branch?".into(),
            options: vec![
                QuestionOption {
                    label: "main".into(),
                    description: "".into(),
                },
                QuestionOption {
                    label: "dev".into(),
                    description: "".into(),
                },
            ],
        }],
        tx,
    });
    app.user_question_jump(3);
    app.user_question_confirm();
    // Should still be on the same page; nothing sent.
    let uq = app.user_question.as_ref().expect("still active");
    assert_eq!(uq.page, 0);
    assert_eq!(uq.selected, 2);
}

#[test]
fn test_review_cancel_drops_state() {
    use mew_agent::{AgentEvent, AskUserQuestion, QuestionOption};
    let mut app = App::new();
    let (tx, mut rx) = tokio::sync::oneshot::channel::<Vec<String>>();
    app.handle_agent_event(AgentEvent::AskUser {
        call_id: "c1".into(),
        questions: vec![
            AskUserQuestion {
                prompt: "a".into(),
                options: vec![
                    QuestionOption {
                        label: "x".into(),
                        description: "".into(),
                    },
                    QuestionOption {
                        label: "y".into(),
                        description: "".into(),
                    },
                ],
            },
            AskUserQuestion {
                prompt: "b".into(),
                options: vec![
                    QuestionOption {
                        label: "x".into(),
                        description: "".into(),
                    },
                    QuestionOption {
                        label: "y".into(),
                        description: "".into(),
                    },
                ],
            },
        ],
        tx,
    });
    app.user_question_confirm();
    app.user_question_confirm();
    // Now on the review page. Move to Cancel and confirm.
    app.user_question_review_next();
    app.user_question_confirm();
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.user_question.is_none());
    assert!(rx.try_recv().is_err());
}

#[test]
fn test_typing_only_affects_freeform_row() {
    use mew_agent::{AgentEvent, AskUserQuestion, QuestionOption};
    let mut app = App::new();
    let (tx, _rx) = tokio::sync::oneshot::channel::<Vec<String>>();
    app.handle_agent_event(AgentEvent::AskUser {
        call_id: "c1".into(),
        questions: vec![AskUserQuestion {
            prompt: "branch?".into(),
            options: vec![
                QuestionOption {
                    label: "main".into(),
                    description: "".into(),
                },
                QuestionOption {
                    label: "dev".into(),
                    description: "".into(),
                },
            ],
        }],
        tx,
    });
    // With the first option highlighted, typing should be a no-op.
    app.user_question_type_char('z');
    let uq = app.user_question.as_ref().unwrap();
    assert!(uq.freeform_text.is_empty());
}

#[test]
fn test_cancel_user_question_drops_without_sending() {
    use mew_agent::{AgentEvent, AskUserQuestion, QuestionOption};
    let mut app = App::new();
    let (tx, mut rx) = tokio::sync::oneshot::channel::<Vec<String>>();
    app.handle_agent_event(AgentEvent::AskUser {
        call_id: "c1".into(),
        questions: vec![AskUserQuestion {
            prompt: "q".into(),
            options: vec![
                QuestionOption {
                    label: "a".into(),
                    description: "".into(),
                },
                QuestionOption {
                    label: "b".into(),
                    description: "".into(),
                },
            ],
        }],
        tx,
    });
    assert_eq!(app.mode, Mode::UserQuestion);
    app.cancel_user_question();
    assert_eq!(app.mode, Mode::Normal);
    assert!(app.user_question.is_none());
    // Sender was dropped without sending → the receiver sees a disconnect,
    // which the agent handler turns into a "cancelled" tool result.
    assert!(rx.try_recv().is_err());
}

#[test]
fn test_todos_updated_event_stores_snapshot() {
    use mew_agent::{AgentEvent, Todo, TodoStatus};
    let mut app = App::new();
    assert!(app.todos.is_empty());
    app.handle_agent_event(AgentEvent::TodosUpdated {
        todos: vec![
            Todo {
                id: 1,
                content: "write tests".into(),
                status: TodoStatus::Done,
                depends_on: vec![],
            },
            Todo {
                id: 2,
                content: "ship".into(),
                status: TodoStatus::InProgress,
                depends_on: vec![1],
            },
        ],
    });
    assert_eq!(app.todos.len(), 2);
    assert_eq!(app.todos[0].id, 1);
    assert_eq!(app.todos[1].status, TodoStatus::InProgress);
}

#[test]
fn test_insert_mention_replaces_trigger_at() {
    // The '@' that opens the picker is already in the input; the picked
    // mention carries its own '@'. Without replacement you get '@@path'.
    let mut app = App::new();
    app.input = "@".to_string();
    app.cursor = 1;
    app.insert_mention("@src/main.rs");
    assert_eq!(app.input, "@src/main.rs");
    assert_eq!(app.cursor, "@src/main.rs".len());

    // Mid-sentence: the trigger '@' is still the last char when picked.
    app.input = "see @".to_string();
    app.cursor = app.input.len();
    app.insert_mention("@lib.rs ");
    assert_eq!(app.input, "see @lib.rs ");
}

#[test]
fn test_namespace_picker_skill() {
    let mut app = App::new();
    app.skill_catalog = vec![
        mew_skills::Skill {
            name: "clarify".into(),
            description: "Improve UX copy".into(),
            body: "body".into(),
            path: std::path::PathBuf::from("(test)"),
            template: false,
        },
        mew_skills::Skill {
            name: "polish".into(),
            description: "Final polish".into(),
            body: "body".into(),
            path: std::path::PathBuf::from("(test)"),
            template: false,
        },
    ];

    app.open_namespace_picker("skill", "");
    let picker = app.picker.as_ref().expect("picker should be open");
    assert_eq!(picker.kind, "ns_skill");
    assert_eq!(picker.items.len(), 2);
    assert!(picker.items.iter().any(|i| i.label == "@skill:clarify"));
}

#[test]
fn test_namespace_picker_model() {
    let mut app = App::new();
    app.models = vec![
        ("openai/gpt-4o".into(), "GPT-4o".into()),
        (
            "anthropic/claude-3-5-sonnet".into(),
            "Claude 3.5 Sonnet".into(),
        ),
    ];

    app.open_namespace_picker("model", "");
    let picker = app.picker.as_ref().expect("picker should be open");
    assert_eq!(picker.kind, "ns_model");
    assert_eq!(picker.items.len(), 2);
    assert!(picker
        .items
        .iter()
        .any(|i| i.label == "@model:openai/gpt-4o"));
}

#[test]
fn test_namespace_picker_subagent() {
    let mut app = App::new();
    app.subagent_catalog = vec![mew_subagents::SubagentDef {
        name: "researcher".into(),
        description: "Investigate research questions".into(),
        model: None,
        tools: None,
        max_turns: None,
        max_duration_secs: None,
        body: "body".into(),
        path: std::path::PathBuf::from("(test)"),
        template: false,
        can_spawn: false,
        output_schema: None,
    }];

    app.open_namespace_picker("subagent", "");
    let picker = app.picker.as_ref().expect("picker should be open");
    assert_eq!(picker.kind, "ns_subagent");
    assert_eq!(picker.items.len(), 1);
    assert!(picker
        .items
        .iter()
        .any(|i| i.label == "@subagent:researcher"));
}

#[test]
fn test_insert_namespace_mention() {
    let mut app = App::new();
    // Input ends with `@` (the trigger that opened the picker).
    app.input = "@".to_string();
    app.cursor = 1;
    app.insert_namespace_mention("model", "openai/gpt-4o");
    assert_eq!(app.input, "@model:openai/gpt-4o ");

    app.input = "@".to_string();
    app.cursor = 1;
    app.insert_namespace_mention("subagent", "researcher");
    assert_eq!(app.input, "@subagent:researcher ");
}

#[test]
fn test_skill_catalog_loaded_via_loader() {
    // Verify the mew_skills::Loader populates skill_catalog correctly,
    // matching how tui.rs loads skills at startup.
    let loader = mew_skills::Loader::new(std::env::current_dir().unwrap_or_default());
    let skills = loader.load().unwrap_or_default();
    // Built-in skills (at minimum mew-docs) should be present.
    assert!(
        skills.iter().any(|s| s.name == "mew-docs"),
        "skill_catalog should contain built-in skills"
    );
}

#[test]
fn test_input_visual_line_count_no_wrap() {
    let mut app = App::new();
    app.input = "short\nlines".to_string();
    assert_eq!(app.input_visual_line_count(80), 2);
}

#[test]
fn test_input_visual_line_count_wraps_long_line() {
    let mut app = App::new();
    app.input = "a".repeat(25);
    // 25 chars at width 10 -> ceil(25/10) = 3 rows
    assert_eq!(app.input_visual_line_count(10), 3);
}

#[test]
fn test_input_visual_line_count_empty_line_counts_as_one() {
    let mut app = App::new();
    app.input = "short\n\n".to_string() + &"a".repeat(25);
    // 1 + 1 + 3 = 5 rows at width 10
    assert_eq!(app.input_visual_line_count(10), 5);
}

#[test]
fn test_cursor_visual_row_col_no_wrap() {
    let mut app = App::new();
    app.input = "hello\nworld".to_string();
    app.cursor = 8; // byte 8 is 'r' in "world" -> line 1, col 2
    assert_eq!(app.cursor_visual_row_col(80), (1, 2));
}

#[test]
fn test_cursor_visual_row_col_wraps() {
    let mut app = App::new();
    app.input = "a".repeat(25);
    app.cursor = 22; // char 22, which is on visual row 2 (chars 20-29), col 2
    assert_eq!(app.cursor_visual_row_col(10), (2, 2));
}

#[test]
fn test_visual_to_byte_offset_first_row() {
    let mut app = App::new();
    app.input = "hello world".to_string();
    // visual row 0, visual col 6 -> byte offset 6 (the 'w')
    assert_eq!(app.visual_to_byte_offset(0, 6, 80), 6);
}

#[test]
fn test_visual_to_byte_offset_wrapped_row() {
    let mut app = App::new();
    app.input = "a".repeat(25);
    // visual row 1, visual col 5 -> char 15 -> byte 15
    assert_eq!(app.visual_to_byte_offset(1, 5, 10), 15);
}

#[test]
fn test_visual_to_byte_offset_past_end() {
    let mut app = App::new();
    app.input = "hi".to_string();
    // visual row 5 is past the end -> return input.len()
    assert_eq!(app.visual_to_byte_offset(5, 0, 80), 2);
}

#[test]
fn test_persona_slash_command_with_name_returns_confirm() {
    let mut app = App::new();
    app.personas = vec![("researcher".into(), "read-only".into())];
    let result = app.handle_slash("/persona researcher");
    // Real switches go through the confirm modal so the user sees
    // the model/toolset diff before applying.
    assert!(matches!(
        result,
        crate::app::SlashResult::PersonaSwitchConfirm(ref n) if n == "researcher"
    ));
}

#[test]
fn test_persona_slash_command_default_returns_direct_clear() {
    let app = App::new();
    // "default" / "none" bypass the confirm modal — they're idempotent.
    let result = app.handle_slash("/persona default");
    assert!(matches!(result, crate::app::SlashResult::SwitchPersona(ref n) if n == "default"));
}

#[test]
fn test_persona_slash_command_same_as_active_returns_message() {
    let mut app = App::new();
    app.personas = vec![("researcher".into(), "read-only".into())];
    app.active_persona = Some("researcher".into());
    let result = app.handle_slash("/persona researcher");
    // Switching to the active persona is a no-op; the slash handler
    // returns an info message rather than opening the confirm modal.
    assert!(matches!(result, crate::app::SlashResult::Message(_)));
}

#[test]
fn test_persona_slash_command_no_arg_lists() {
    let mut app = App::new();
    app.personas = vec![
        ("researcher".into(), "read-only".into()),
        ("executor".into(), "writes code".into()),
    ];
    app.active_persona = Some("researcher".into());
    let result = app.handle_slash("/persona");
    assert!(matches!(result, SlashResult::OpenPersonaPicker));
    // Verify the picker is populated.
    app.open_persona_picker();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.kind, "persona");
    assert_eq!(picker.items.len(), 2);
    assert!(picker.items[0].label.contains("researcher"));
    assert!(picker.items[0].label.contains("active"));
}

#[test]
fn test_persona_slash_command_empty_personas() {
    let mut app = App::new();
    let result = app.handle_slash("/persona");
    assert!(matches!(result, SlashResult::OpenPersonaPicker));
    // Empty personas → empty picker (not an error message).
    app.open_persona_picker();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.kind, "persona");
    assert!(picker.items.is_empty());
}

#[test]
fn test_rewind_slash_command_returns_rewind() {
    let app = App::new();
    let result = app.handle_slash("/rewind 2");
    assert!(matches!(result, SlashResult::Rewind(2)));
}

#[test]
fn test_rewind_slash_command_invalid_arg() {
    let app = App::new();
    let result = app.handle_slash("/rewind abc");
    match result {
        SlashResult::Message(msg) => assert!(msg.contains("usage")),
        _ => panic!("expected Message"),
    }
}

#[test]
fn test_rewind_slash_command_no_arg_lists() {
    let mut app = App::new();
    // Add a message so the picker isn't empty.
    let msg = mew_message::Message {
        id: ulid::Ulid::new(),
        session_id: ulid::Ulid::new(),
        role: mew_message::Role::User,
        parts: vec![mew_message::Part::Text(mew_message::TextPart {
            base: mew_message::PartBase {
                id: ulid::Ulid::new(),
                message_id: ulid::Ulid::new(),
                session_id: ulid::Ulid::new(),
            },
            text: "hello world".into(),
            synthetic: false,
        })],
        time: mew_message::Time {
            created: 0,
            completed: None,
        },
        assistant: None,
    };
    app.messages.push(msg);
    let result = app.handle_slash("/rewind");
    assert!(matches!(result, SlashResult::OpenRewindPicker));
    // Verify the picker is populated.
    app.open_rewind_picker();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.kind, "rewind");
    assert_eq!(picker.items.len(), 1);
    assert!(picker.items[0].label.contains("hello world"));
}

#[test]
fn test_rewind_slash_empty_messages() {
    let mut app = App::new();
    let result = app.handle_slash("/rewind");
    assert!(matches!(result, SlashResult::OpenRewindPicker));
    // Empty messages → empty picker.
    app.open_rewind_picker();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.kind, "rewind");
    assert!(picker.items.is_empty());
}

#[test]
fn test_rewind_to_truncates_messages() {
    let mut app = App::new();
    for i in 0..5 {
        let id = ulid::Ulid::new();
        let part_id = ulid::Ulid::new();
        let msg = mew_message::Message {
            id,
            session_id: ulid::Ulid::new(),
            role: mew_message::Role::User,
            parts: vec![mew_message::Part::Text(mew_message::TextPart {
                base: mew_message::PartBase {
                    id: part_id,
                    message_id: id,
                    session_id: ulid::Ulid::new(),
                },
                text: format!("msg {}", i),
                synthetic: false,
            })],
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        };
        app.messages.push(msg);
        app.rendered_md_cache.insert(
            part_id,
            (80, format!("msg {}", i), std::rc::Rc::new(vec![])),
        );
    }
    assert_eq!(app.messages.len(), 5);
    assert_eq!(app.rendered_md_cache.len(), 5);

    app.rewind_to(2);
    assert_eq!(app.messages.len(), 2);
    assert_eq!(app.rendered_md_cache.len(), 2);
}

#[test]
fn test_rewind_to_noop_when_n_too_large() {
    let mut app = App::new();
    app.messages.push(mew_message::Message {
        id: ulid::Ulid::new(),
        session_id: ulid::Ulid::new(),
        role: mew_message::Role::User,
        parts: vec![],
        time: mew_message::Time {
            created: 0,
            completed: None,
        },
        assistant: None,
    });
    app.rewind_to(10);
    assert_eq!(app.messages.len(), 1);
}

#[test]
fn test_undo_redo_basic() {
    let mut app = App::new();
    assert!(app.input.is_empty());

    // Type "hello" — coalesces into one undo entry.
    app.insert_char('h');
    app.insert_char('e');
    app.insert_char('l');
    app.insert_char('l');
    app.insert_char('o');
    assert_eq!(app.input, "hello");

    // Undo restores to empty (coalesced entry).
    app.undo();
    assert_eq!(app.input, "");
    assert!(app.undo_stack.is_empty());

    // Redo restores "hello".
    app.redo();
    assert_eq!(app.input, "hello");
}

#[test]
fn test_undo_after_backspace() {
    let mut app = App::new();
    app.insert_char('a');
    app.insert_char('b');
    // Wait past the coalesce window so backspace is its own entry.
    std::thread::sleep(std::time::Duration::from_millis(600));
    app.backspace(); // removes 'b'
    assert_eq!(app.input, "a");

    app.undo();
    assert_eq!(app.input, "ab");
}

#[test]
fn test_redo_cleared_on_new_edit() {
    let mut app = App::new();
    app.insert_char('x');
    std::thread::sleep(std::time::Duration::from_millis(600));
    app.insert_char('y');
    app.undo();
    app.undo();
    assert_eq!(app.input, "");
    assert!(!app.redo_stack.is_empty());

    // New edit clears redo stack.
    app.insert_char('z');
    assert!(app.redo_stack.is_empty());
    assert_eq!(app.input, "z");
}

#[test]
fn test_undo_paste_single_entry() {
    let mut app = App::new();
    // Simulate a paste by pushing undo once then inserting multiple chars.
    app.push_undo();
    for c in "pasted".chars() {
        app.input.insert(app.cursor, c);
        app.cursor += c.len_utf8();
    }
    assert_eq!(app.input, "pasted");
    assert_eq!(app.undo_stack.len(), 1); // single entry, not 6

    app.undo();
    assert_eq!(app.input, "");
}

#[test]
fn test_toast_queue_pushes_and_expires() {
    let mut app = App::new();
    assert!(app.toasts.is_empty());

    app.set_alert("copied 42 chars");
    assert_eq!(app.toasts.len(), 1);
    assert_eq!(app.toasts[0].0, "copied 42 chars");

    app.set_alert("model switched");
    assert_eq!(app.toasts.len(), 2);

    app.set_alert("third toast");
    app.set_alert("fourth toast");
    // Cap at 3 visible.
    assert_eq!(app.toasts.len(), 3);
    assert_eq!(app.toasts[0].0, "model switched"); // oldest dropped

    // Expiry: simulate passage of time by manually setting old timestamps.
    let old = Instant::now() - Duration::from_secs(5);
    for toast in &mut app.toasts {
        toast.1 = old;
    }
    app.clear_expired_alerts();
    assert!(app.toasts.is_empty());
}

#[test]
fn test_history_search_finds_matches() {
    let mut app = App::new();
    app.history.push("cargo build".into());
    app.history.push("cargo test".into());
    app.history.push("git status".into());

    app.start_history_search();
    assert_eq!(app.mode, Mode::HistorySearch);

    // Search for "cargo" → 2 matches (newest first).
    for c in "cargo".chars() {
        app.history_search_query.push(c);
    }
    app.history_search_index = Some(0);
    let matches = app.history_search_matches();
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0], "cargo test"); // newest first

    // Current match should be "cargo test".
    assert_eq!(
        app.history_search_current_match(),
        Some("cargo test".to_string())
    );

    // Confirm: input should be set to the match.
    app.history_search_confirm();
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.input, "cargo test");
}

#[test]
fn test_history_search_cancel_restores() {
    let mut app = App::new();
    app.input = "partial text".into();
    app.cursor = app.input.len();

    app.start_history_search();
    assert_eq!(app.mode, Mode::HistorySearch);

    // Cancel restores saved input.
    app.history_search_cancel();
    assert_eq!(app.mode, Mode::Normal);
    assert_eq!(app.input, "partial text");
}

#[test]
fn test_history_search_no_match() {
    let mut app = App::new();
    app.history.push("hello world".into());

    app.start_history_search();
    app.history_search_query = "xyz".into();
    app.history_search_index = Some(0);

    let matches = app.history_search_matches();
    assert!(matches.is_empty());
    assert_eq!(app.history_search_current_match(), None);
}

#[test]
fn test_help_opens_shortcuts_overlay() {
    let app = App::new();
    let result = app.handle_slash("/help");
    assert!(matches!(result, SlashResult::OpenHelp));
}

#[test]
fn test_thinking_no_arg_opens_picker() {
    let app = App::new();
    let result = app.handle_slash("/thinking");
    assert!(matches!(result, SlashResult::OpenThinkingVariantPicker));
}

#[test]
fn test_thinking_picker_uses_model_variants() {
    let mut app = App::new();
    app.status.model = "gpt-5.6-sol".into();
    app.thinking_variants.insert(
        "gpt-5.6-sol".into(),
        vec![
            "low".into(),
            "medium".into(),
            "high".into(),
            "xhigh".into(),
            "max".into(),
            "ultra".into(),
        ],
    );
    app.open_thinking_variant_picker();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.kind, "thinking_variant");
    // "Off" + the model's six real variants — not the old hardcoded
    // ["high", "max", "thinking"].
    assert_eq!(picker.items.len(), 7);
    assert_eq!(picker.items[0].id, "off");
    let names: Vec<&str> = picker.items[1..].iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        names,
        vec!["low", "medium", "high", "xhigh", "max", "ultra"]
    );
}

#[test]
fn test_thinking_picker_no_variants_shows_only_off() {
    let mut app = App::new();
    app.status.model = "unknown-model".into();
    app.open_thinking_variant_picker();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.items.len(), 1);
    assert_eq!(picker.items[0].id, "off");
}

#[test]
fn test_thinking_variant_picker_strips_provider_prefix() {
    // The model picker uses "provider/model" IDs (e.g.
    // "opencode-zen/claude-sonnet-4-6"), but thinking_variants is keyed by
    // the bare model id. open_thinking_variant_picker_for must strip the
    // provider prefix to find the variants.
    let mut app = App::new();
    app.status.model = "some-other-model".into();
    app.thinking_variants.insert(
        "claude-sonnet-4-6".into(),
        vec!["low".into(), "high".into(), "max".into()],
    );
    // Pass the full "provider/model" format, as the model picker would.
    app.open_thinking_variant_picker_for(Some("opencode-zen/claude-sonnet-4-6"));
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.kind, "thinking_variant");
    // "Off" + 3 variants.
    assert_eq!(picker.items.len(), 4);
    let names: Vec<&str> = picker.items[1..].iter().map(|i| i.id.as_str()).collect();
    assert_eq!(names, vec!["low", "high", "max"]);
}

#[test]
fn test_thinking_variant_picker_for_bare_model_id() {
    // When passed a bare model id (no provider prefix), it should still work.
    let mut app = App::new();
    app.status.model = "wrong-model".into();
    app.thinking_variants
        .insert("k3".into(), vec!["low".into(), "high".into(), "max".into()]);
    app.open_thinking_variant_picker_for(Some("k3"));
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.items.len(), 4);
    let names: Vec<&str> = picker.items[1..].iter().map(|i| i.id.as_str()).collect();
    assert_eq!(names, vec!["low", "high", "max"]);
}

#[test]
fn test_model_picker_has_thinking_hint() {
    let mut app = App::new();
    app.open_model_picker();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.kind, "model");
    assert!(picker.hint.is_some());
    assert!(picker.hint.as_ref().unwrap().contains("thinking"));
}

#[test]
fn test_model_picker_shows_recent_section() {
    let mut app = App::new();
    app.models = vec![
        ("kimi/k3".into(), "kimi · openai".into()),
        ("opencode-zen/claude-sonnet-4-6".into(), "anthropic".into()),
    ];
    app.recent_models = vec!["kimi/k3".into()];

    app.open_model_picker();
    let picker = app.picker.as_ref().unwrap();

    // First item should be the "Recent" header.
    assert!(picker.items[0].header);
    assert_eq!(picker.items[0].label, "Recent");

    // Second item should be the recent model.
    assert_eq!(picker.items[1].id, "kimi/k3");
    assert!(!picker.items[1].header);

    // Third item should be the "All Models" header.
    assert!(picker.items[2].header);
    assert_eq!(picker.items[2].label, "All Models");

    // Remaining items should be the full model list.
    assert_eq!(picker.items[3].id, "kimi/k3");
    assert_eq!(picker.items[4].id, "opencode-zen/claude-sonnet-4-6");

    // Selection should start on the first recent model (index 1), not the header.
    assert_eq!(picker.selected, 1);
}

#[test]
fn test_model_picker_no_recent_section_when_empty() {
    let mut app = App::new();
    app.models = vec![("kimi/k3".into(), "kimi · openai".into())];
    app.recent_models = vec![];

    app.open_model_picker();
    let picker = app.picker.as_ref().unwrap();

    // No headers should be present.
    assert!(!picker.items.iter().any(|i| i.header));
    assert_eq!(picker.items.len(), 1);
    assert_eq!(picker.selected, 0);
}

#[test]
fn test_model_picker_recent_filters_unknown_models() {
    let mut app = App::new();
    app.models = vec![("kimi/k3".into(), "kimi · openai".into())];
    // Include a model that's no longer available.
    app.recent_models = vec!["kimi/k3".into(), "old-provider/dead-model".into()];

    app.open_model_picker();
    let picker = app.picker.as_ref().unwrap();

    // The dead model should not appear in the Recent section.
    // Count items between the "Recent" header and "All Models" header.
    let in_recent_section = picker
        .items
        .iter()
        .skip_while(|i| !i.header || i.label != "Recent")
        .skip(1) // skip the "Recent" header itself
        .take_while(|i| !i.header) // until "All Models" header
        .collect::<Vec<_>>();
    assert_eq!(in_recent_section.len(), 1);
    assert_eq!(in_recent_section[0].id, "kimi/k3");
}

#[test]
fn test_picker_move_selection_skips_headers() {
    let mut app = App::new();
    app.models = vec![
        ("kimi/k3".into(), "desc1".into()),
        ("opencode-zen/claude".into(), "desc2".into()),
    ];
    app.recent_models = vec!["kimi/k3".into()];
    app.open_model_picker();

    let picker = app.picker.as_ref().unwrap();
    // Items: [Header: Recent, kimi/k3, Header: All Models, kimi/k3, claude]
    assert_eq!(picker.items.len(), 5);

    // Selection starts at index 1 (first recent model).
    assert_eq!(picker.selected, 1);

    // Move down — should skip the "All Models" header and land on kimi/k3 (index 3).
    app.picker_down();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.selected, 3);

    // Move down again — should land on claude (index 4).
    app.picker_down();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.selected, 4);

    // Move down — should wrap to index 1 (skipping header at 0).
    app.picker_down();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.selected, 1);
}

#[test]
fn test_picker_filtered_hides_headers() {
    let mut app = App::new();
    app.models = vec![("kimi/k3".into(), "desc1".into())];
    app.recent_models = vec!["kimi/k3".into()];
    app.open_model_picker();

    // Type a filter — headers should be hidden.
    if let Some(ref mut p) = app.picker {
        p.filter = "kimi".into();
    }

    let picker = app.picker.as_ref().unwrap();
    let filtered = picker.filtered();
    // No headers in filtered results.
    assert!(!filtered.iter().any(|i| i.header));
    // Both kimi/k3 entries should match the filter.
    assert_eq!(filtered.len(), 2);
}

#[test]
fn test_active_thinking_variant_updates_from_daemon_notification() {
    let mut app = App::new();
    assert!(app.active_thinking_variant.is_none());

    // Simulate daemon sending ThinkingVariantChanged { variant: Some("high") }
    let msg = mew_protocol::ServerMessage::ThinkingVariantChanged {
        variant: Some("high".into()),
    };
    app.apply_daemon_notification(&msg);
    assert_eq!(app.active_thinking_variant.as_deref(), Some("high"));

    // Simulate disabling thinking
    let msg = mew_protocol::ServerMessage::ThinkingVariantChanged { variant: None };
    app.apply_daemon_notification(&msg);
    assert!(app.active_thinking_variant.is_none());
}

#[test]
fn test_model_list_populates_picker_state() {
    let mut app = App::new();
    app.status.provider = "kimi".into();
    app.status.model = "k3".into();
    assert!(app.models.is_empty());
    assert!(app.thinking_variants.is_empty());

    let msg = mew_protocol::ServerMessage::ModelList {
        models: vec![
            mew_protocol::ModelInfo {
                id: "kimi/k3".into(),
                provider: "kimi".into(),
                model: "k3".into(),
                description: Some("256k ctx · reasoning".into()),
                thinking_variants: vec![
                    mew_protocol::ThinkingVariantInfo {
                        name: "high".into(),
                    },
                    mew_protocol::ThinkingVariantInfo { name: "max".into() },
                ],
                thinking_budget: None,
                context_window: Some(256_000),
            },
            mew_protocol::ModelInfo {
                id: "deepseek/deepseek-v4-flash".into(),
                provider: "deepseek".into(),
                model: "deepseek-v4-flash".into(),
                description: None,
                thinking_variants: vec![],
                thinking_budget: None,
                context_window: None,
            },
        ],
    };
    app.apply_daemon_notification(&msg);

    assert_eq!(
        app.models,
        vec![
            ("kimi/k3".to_string(), "256k ctx · reasoning".to_string()),
            (
                "deepseek/deepseek-v4-flash".to_string(),
                "deepseek".to_string()
            ),
        ]
    );
    // Thinking variants keyed by bare model id; models without variants
    // are omitted.
    assert_eq!(
        app.thinking_variants.get("k3").map(Vec::as_slice),
        Some(["high".to_string(), "max".to_string()].as_slice())
    );
    assert!(!app.thinking_variants.contains_key("deepseek-v4-flash"));
    // Active model's context window is refreshed.
    assert_eq!(app.status.context_window, 256_000);
}

#[test]
fn test_change_stats_and_flagged_files_reduce() {
    let mut app = App::new();
    app.status.session_id = "sess_1".into();

    // Local mode: FileDelta accumulates per-file stats.
    app.handle_agent_event(mew_agent::AgentEvent::FileDelta {
        path: "src/main.rs".into(),
        added: 10,
        removed: 2,
    });
    app.handle_agent_event(mew_agent::AgentEvent::FileDelta {
        path: "src/lib.rs".into(),
        added: 5,
        removed: 0,
    });
    assert_eq!(app.change_stats.added, 15);
    assert_eq!(app.change_stats.removed, 2);
    assert_eq!(
        app.change_stats.files,
        vec!["src/main.rs".to_string(), "src/lib.rs".to_string()]
    );

    // Local mode: FlaggedFilesChanged replaces the flagged set.
    app.handle_agent_event(mew_agent::AgentEvent::FlaggedFilesChanged {
        files: vec![mew_agent::FlaggedFileInfo {
            path: "src/main.rs".into(),
            reason: Some("important".into()),
        }],
    });
    assert_eq!(app.flagged_files.len(), 1);
    assert_eq!(app.flagged_files[0].path, "src/main.rs");

    // Daemon mode: SessionStatsChanged updates totals for the active
    // session only.
    app.apply_daemon_notification(&mew_protocol::ServerMessage::SessionStatsChanged {
        session_id: "sess_2".into(),
        added: 99,
        removed: 99,
        files_changed: 1,
    });
    assert_eq!(app.change_stats.added, 15, "other session must not apply");
    app.apply_daemon_notification(&mew_protocol::ServerMessage::SessionStatsChanged {
        session_id: "sess_1".into(),
        added: 20,
        removed: 4,
        files_changed: 2,
    });
    assert_eq!(app.change_stats.added, 20);
    assert_eq!(app.change_stats.removed, 4);

    // Daemon mode: FlaggedFilesChanged replaces the flagged set.
    app.apply_daemon_notification(&mew_protocol::ServerMessage::FlaggedFilesChanged {
        session_id: "sess_1".into(),
        files: vec![mew_protocol::FlaggedFileWire {
            path: "README.md".into(),
            reason: None,
        }],
    });
    assert_eq!(app.flagged_files.len(), 1);
    assert_eq!(app.flagged_files[0].path, "README.md");
}

#[test]
fn test_session_list_syncs_active_change_stats() {
    let mut app = App::new();
    app.status.session_id = "sess_1".into();
    app.apply_daemon_notification(&mew_protocol::ServerMessage::SessionList {
        sessions: vec![mew_protocol::SessionInfo {
            session_id: "sess_1".into(),
            state: mew_protocol::SessionState::Active,
            model: None,
            provider: None,
            created_at: 0,
            last_message_at: None,
            summary: None,
            client_count: 1,
            cwd: None,
            last_turn_failed: false,
            archived: false,
            pinned: false,
            group_id: None,
            change_stats: Some(mew_session::ChangeStats {
                added: 7,
                removed: 3,
                files: vec!["a.rs".into()],
            }),
            usage: None,
            context_tokens: None,
            pending_permissions: 0,
            pending_questions: 0,
            first_message: None,
        }],
    });
    assert_eq!(app.change_stats.added, 7);
    assert_eq!(app.change_stats.files, vec!["a.rs".to_string()]);
}

#[test]
fn test_session_list_sorted_by_last_seen() {
    fn info(id: &str, created_at: i64, last_message_at: Option<i64>) -> mew_protocol::SessionInfo {
        mew_protocol::SessionInfo {
            session_id: id.into(),
            state: mew_protocol::SessionState::Idle,
            model: None,
            provider: None,
            created_at,
            last_message_at,
            summary: None,
            client_count: 0,
            cwd: None,
            last_turn_failed: false,
            archived: false,
            pinned: false,
            group_id: None,
            change_stats: None,
            usage: None,
            context_tokens: None,
            pending_permissions: 0,
            pending_questions: 0,
            first_message: None,
        }
    }

    let mut app = App::new();
    // Deliberately out of order: "old" has the newest message, "fresh" has
    // no last_message_at and falls back to created_at, "mid" sits between.
    app.apply_daemon_notification(&mew_protocol::ServerMessage::SessionList {
        sessions: vec![
            info("mid", 100, Some(200)),
            info("old", 10, Some(300)),
            info("fresh", 250, None),
        ],
    });
    let ids: Vec<&str> = app
        .daemon_sessions
        .iter()
        .map(|s| s.session_id.as_str())
        .collect();
    assert_eq!(
        ids,
        vec!["old", "fresh", "mid"],
        "sessions must be newest-first by last_message_at (created_at fallback)"
    );
}

#[test]
fn test_session_list_seeds_active_context_tokens() {
    let mut app = App::new();
    app.status.session_id = "sess_1".into();
    let mut info = mew_protocol::SessionInfo {
        session_id: "sess_1".into(),
        state: mew_protocol::SessionState::Active,
        model: None,
        provider: None,
        created_at: 0,
        last_message_at: None,
        summary: None,
        client_count: 1,
        cwd: None,
        last_turn_failed: false,
        archived: false,
        pinned: false,
        group_id: None,
        change_stats: None,
        usage: None,
        context_tokens: Some(99_000),
        pending_permissions: 0,
        pending_questions: 0,
        first_message: None,
    };
    app.apply_daemon_notification(&mew_protocol::ServerMessage::SessionList {
        sessions: vec![info.clone()],
    });
    assert_eq!(
        app.status.context_tokens, 99_000,
        "attaching must seed the context reading from the session list"
    );

    // A list without a reading (older daemon) leaves the live value alone.
    app.status.context_tokens = 12_345;
    info.context_tokens = None;
    app.apply_daemon_notification(&mew_protocol::ServerMessage::SessionList {
        sessions: vec![info],
    });
    assert_eq!(app.status.context_tokens, 12_345);
}

#[test]
fn test_theme_no_arg_opens_picker() {
    let app = App::new();
    let result = app.handle_slash("/theme");
    assert!(matches!(result, SlashResult::OpenThemePicker));
}

#[test]
fn test_message_end_tracks_current_context_tokens() {
    let mut app = App::new();
    let message_end = |input: u32, output: u32| {
        mew_agent::AgentEvent::Provider(mew_provider::ProviderEvent::MessageEnd {
            finish: mew_message::Finish::Stop,
            usage: mew_message::Tokens {
                input,
                output,
                ..Default::default()
            },
            cost: 0.01,
        })
    };

    // Each MessageEnd reports that request's prompt size, i.e. the current
    // context occupancy. The status must snapshot it, not accumulate a
    // lifetime total.
    app.handle_agent_event(message_end(1_500, 200));
    assert_eq!(app.status.context_tokens, 1_500);
    app.handle_agent_event(message_end(2_100, 300));
    assert_eq!(
        app.status.context_tokens, 2_100,
        "context usage must track the latest request, not sum all requests"
    );

    // Zero usage means a synthetic completion (slash output, harness);
    // it must not clobber the live reading.
    app.handle_agent_event(message_end(0, 0));
    assert_eq!(app.status.context_tokens, 2_100);
}

#[test]
fn test_theme_picker_lists_themes() {
    let mut app = App::new();
    let available = crate::theme::Theme::list_available();
    app.open_theme_picker();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.kind, "theme");
    assert_eq!(picker.items.len(), available.len());
}

#[test]
fn test_persona_no_arg_opens_picker() {
    let mut app = App::new();
    app.personas = vec![("test".into(), "test persona".into())];
    let result = app.handle_slash("/persona");
    assert!(matches!(result, SlashResult::OpenPersonaPicker));
}

#[test]
fn test_persona_picker_lists_personas() {
    let mut app = App::new();
    app.personas = vec![
        ("alpha".into(), "first".into()),
        ("beta".into(), "second".into()),
    ];
    app.open_persona_picker();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.kind, "persona");
    assert_eq!(picker.items.len(), 2);
}

#[test]
fn test_rewind_no_arg_opens_picker() {
    let app = App::new();
    let result = app.handle_slash("/rewind");
    assert!(matches!(result, SlashResult::OpenRewindPicker));
}

#[test]
fn test_rewind_picker_lists_messages() {
    let mut app = App::new();
    for i in 0..3 {
        let msg = mew_message::Message {
            id: ulid::Ulid::new(),
            session_id: ulid::Ulid::new(),
            role: if i % 2 == 0 {
                mew_message::Role::User
            } else {
                mew_message::Role::Assistant
            },
            parts: vec![mew_message::Part::Text(mew_message::TextPart {
                base: mew_message::PartBase {
                    id: ulid::Ulid::new(),
                    message_id: ulid::Ulid::new(),
                    session_id: ulid::Ulid::new(),
                },
                text: format!("msg {}", i),
                synthetic: false,
            })],
            time: mew_message::Time {
                created: 0,
                completed: None,
            },
            assistant: None,
        };
        app.messages.push(msg);
    }
    app.open_rewind_picker();
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.kind, "rewind");
    assert_eq!(picker.items.len(), 3);
}

#[test]
fn test_sessions_no_arg_opens_picker() {
    let app = App::new();
    let result = app.handle_slash("/sessions");
    assert!(matches!(result, SlashResult::OpenSessionPickerFromDisk));
}

#[test]
fn test_resume_no_arg_opens_picker() {
    let app = App::new();
    let result = app.handle_slash("/resume");
    assert!(matches!(result, SlashResult::OpenSessionPickerFromDisk));
}

#[test]
fn test_sessions_no_arg_opens_daemon_picker_in_daemon_mode() {
    let mut app = App::new();
    app.daemon_mode = true;
    let result = app.handle_slash("/sessions");
    assert!(matches!(result, SlashResult::OpenSessionPicker));
}

#[test]
fn test_resume_no_arg_opens_daemon_picker_in_daemon_mode() {
    let mut app = App::new();
    app.daemon_mode = true;
    let result = app.handle_slash("/resume");
    assert!(matches!(result, SlashResult::OpenSessionPicker));
}

#[test]
fn test_command_palette_includes_all_commands() {
    let mut app = App::new();
    app.open_command_palette();
    let picker = app.picker.as_ref().unwrap();
    // The palette has the 5 original items (switch-model, thinking-variant,
    // settings, clear, quit) plus all unique builtin slash commands except
    // /clear, /quit, /model, /thinking, /help (already represented or
    // is the palette itself). Note: /q is a handle_slash alias, not in
    // builtin_slash_commands().
    let builtin = App::builtin_slash_commands();
    let unique_names: std::collections::HashSet<&str> =
        builtin.iter().map(|c| c.name.as_str()).collect();
    let excluded = 5; // /clear, /quit, /model, /thinking, /help
    let expected = 5 + unique_names.len() - excluded;
    assert_eq!(
        picker.items.len(),
        expected,
        "palette has {} items, expected {} (5 original + {} slash commands)",
        picker.items.len(),
        expected,
        unique_names.len() - excluded
    );
}

#[test]
fn test_cost_still_dumps_text() {
    let app = App::new();
    let result = app.handle_slash("/cost");
    assert!(matches!(result, SlashResult::Message(_)));
}

/// AC.12: PartStart on an existing assistant message marks chat dirty.
/// Before the fix, the existing-message branch returned before
/// calling mark_chat_dirty(), causing stale renders.
#[test]
fn test_partstart_existing_message_dirty() {
    let mut app = App::new();

    // Start with a clean dirty state.
    app.chat_dirty = None;

    // Push an initial assistant message so there's an existing one
    // to append to.
    app.messages.push(Message {
        id: ulid::Ulid::new(),
        session_id: ulid::Ulid::new(),
        role: Role::Assistant,
        parts: vec![],
        time: mew_message::Time {
            created: chrono::Utc::now().timestamp_millis(),
            completed: None,
        },
        assistant: None,
    });

    // Simulate PartStart(Text) arriving for the existing message.
    let part = Part::Text(mew_message::TextPart {
        base: mew_message::PartBase {
            id: mew_message::PartId::new(),
            message_id: mew_message::MessageId::new(),
            session_id: mew_message::SessionId::new(),
        },
        text: "hello".into(),
        synthetic: false,
    });
    app.handle_agent_event(mew_agent::AgentEvent::Provider(
        mew_provider::ProviderEvent::PartStart { part },
    ));

    // The chat should be marked dirty — before the fix, this was None.
    assert!(
        app.chat_dirty.is_some(),
        "PartStart on existing assistant message must mark chat dirty"
    );
}

/// AC.10: A message with two text parts does not thrash the markdown cache.
/// The cache is keyed by PartId, so each text part gets its own entry.
/// Verify by inserting cache entries for both PartIds and confirming
/// neither is evicted — if the cache were keyed by MessageId, the second
/// insert would overwrite the first.
#[test]
fn test_multipart_cache_keyed_by_partid() {
    let mut app = App::new();

    let part1_id = mew_message::PartId::new();
    let part2_id = mew_message::PartId::new();
    let msg_id = mew_message::MessageId::new();
    let sess_id = mew_message::SessionId::new();

    let msg = Message {
        id: msg_id,
        session_id: sess_id,
        role: Role::Assistant,
        parts: vec![
            Part::Text(mew_message::TextPart {
                base: mew_message::PartBase {
                    id: part1_id,
                    message_id: msg_id,
                    session_id: sess_id,
                },
                text: "First text".into(),
                synthetic: false,
            }),
            Part::Text(mew_message::TextPart {
                base: mew_message::PartBase {
                    id: part2_id,
                    message_id: msg_id,
                    session_id: sess_id,
                },
                text: "Second text".into(),
                synthetic: false,
            }),
        ],
        time: mew_message::Time {
            created: chrono::Utc::now().timestamp_millis(),
            completed: None,
        },
        assistant: None,
    };
    app.messages.push(msg);

    // Simulate rendering both parts into the cache — each part gets
    // its own cache entry keyed by PartId.
    app.rendered_md_cache.insert(
        part1_id,
        (80, "first".to_string(), std::rc::Rc::new(vec![])),
    );
    app.rendered_md_cache.insert(
        part2_id,
        (80, "second".to_string(), std::rc::Rc::new(vec![])),
    );

    // Both entries must survive — if the cache were keyed by MessageId,
    // the second insert would have evicted the first.
    assert_eq!(
        app.rendered_md_cache.len(),
        2,
        "cache should hold 2 entries for 2 PartIds in one message"
    );
    assert!(
        app.rendered_md_cache.contains_key(&part1_id),
        "first PartId should have a cache entry"
    );
    assert!(
        app.rendered_md_cache.contains_key(&part2_id),
        "second PartId should have a cache entry"
    );
}

/// AC.13: Verify that the render cache coalesces multiple deltas between
/// renders. This tests the render-cache invariant (dirty flag batches
/// multiple deltas into one rebuild), which is the mechanism the drain loop
/// relies on. The drain loop itself (event_rx batching in run_tui) is not
/// tested here — that would require simulating the event channel.
#[test]
fn test_render_cache_batches_deltas() {
    let mut app = App::new();
    app.streaming = true;

    // Create an assistant message with a text part to stream into.
    let part_id = mew_message::PartId::new();
    let msg_id = mew_message::MessageId::new();
    let sess_id = mew_message::SessionId::new();

    app.messages.push(Message {
        id: msg_id,
        session_id: sess_id,
        role: Role::Assistant,
        parts: vec![Part::Text(mew_message::TextPart {
            base: mew_message::PartBase {
                id: part_id,
                message_id: msg_id,
                session_id: sess_id,
            },
            text: String::new(),
            synthetic: false,
        })],
        time: mew_message::Time {
            created: chrono::Utc::now().timestamp_millis(),
            completed: None,
        },
        assistant: None,
    });

    // Initialize the markdown stream for the text part.
    app.md_stream = Some(mdstream::MdStream::new(mdstream::Options::default()));
    app.md_state = mdstream::DocumentState::new();

    // Reset render_count to start fresh.
    app.render_count = 0;

    // Simulate 10 char-granularity deltas arriving in drain batches.
    // The drain cap is 4, so we process 4 deltas, render, repeat.
    const STREAMING_DRAIN_LIMIT: usize = 4;
    let deltas: Vec<char> = "helloworld".chars().collect();

    for chunk in deltas.chunks(STREAMING_DRAIN_LIMIT) {
        // Feed up to STREAMING_DRAIN_LIMIT deltas (simulating one drain batch).
        for ch in chunk {
            app.handle_agent_event(mew_agent::AgentEvent::Provider(
                mew_provider::ProviderEvent::PartDelta {
                    part_id,
                    field: "text",
                    delta: ch.to_string(),
                },
            ));
        }
        // Render after the drain batch (simulates one frame).
        app.ensure_chat_rendered(80, 80, 24);
    }

    // With batching, we expect exactly ceil(10/4) = 3 renders.
    // A broken cache that re-renders on every delta would give 10.
    assert_eq!(
        app.render_count, 3,
        "expected exactly 3 render rebuilds for 10 deltas in batches of 4 \
             (got {} — broken would be 10)",
        app.render_count
    );
}

// --- thinking token-budget row ---

fn qwen_budget() -> mew_protocol::ThinkingBudgetInfo {
    mew_protocol::ThinkingBudgetInfo {
        min: 0,
        max: 262_144,
        step: 1024,
        default: 131_072,
        by_effort: vec![
            ("low".to_owned(), 4096),
            ("medium".to_owned(), 16_384),
            ("xhigh".to_owned(), 262_144),
        ],
    }
}

fn open_qwen_thinking_picker(app: &mut App) {
    app.status.model = "qwen3.8-max".into();
    app.thinking_variants.insert(
        "qwen3.8-max".into(),
        vec!["low".into(), "medium".into(), "xhigh".into()],
    );
    app.thinking_budget
        .insert("qwen3.8-max".into(), qwen_budget());
    app.open_thinking_variant_picker_for(None);
}

fn select_budget_row(app: &mut App) {
    let picker = app.picker.as_mut().expect("picker open");
    picker.selected = picker
        .items
        .iter()
        .position(|i| i.id == "budget")
        .expect("budget row present");
}

fn budget_draft(app: &App) -> String {
    app.picker
        .as_ref()
        .and_then(|p| p.budget.as_ref())
        .map(|b| b.draft.clone())
        .expect("budget draft present")
}

/// Extract the variant string from a SetThinkingVariant action (for
/// assertion ergonomics; `Action` doesn't implement PartialEq).
fn thinking_action(action: Option<crate::events::Action>) -> Option<String> {
    match action {
        Some(crate::events::Action::SetThinkingVariant(variant)) => Some(variant),
        _ => None,
    }
}

fn key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
}

#[test]
fn ctrl_up_guides_oldest_queued_message() {
    use crate::events::Action;
    let mut app = App::new();
    app.streaming = true;
    app.queued_messages = vec!["first".into(), "second".into()];

    let ctrl_up = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Up,
        crossterm::event::KeyModifiers::CONTROL,
    );
    let action = crate::events::handle_key_event(&mut app, ctrl_up);
    assert!(matches!(action, Some(Action::GuideQueued(text)) if text == "first"));
    // The guided message is popped from the queue; the rest remain.
    assert_eq!(app.queued_messages, vec!["second".to_string()]);
}

#[test]
fn ctrl_up_without_queued_message_is_noop() {
    let mut app = App::new();
    app.streaming = true;
    app.queued_messages.clear();
    let ctrl_up = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Up,
        crossterm::event::KeyModifiers::CONTROL,
    );
    let action = crate::events::handle_key_event(&mut app, ctrl_up);
    assert!(action.is_none());
}

#[test]
fn test_thinking_picker_budget_row_only_with_metadata() {
    let mut app = App::new();
    app.status.model = "qwen3.8-max".into();
    // No budget metadata → no budget row.
    app.open_thinking_variant_picker_for(None);
    let picker = app.picker.as_ref().unwrap();
    assert!(!picker.items.iter().any(|i| i.id == "budget"));
    assert!(picker.budget.is_none());

    // With budget metadata → budget row appended after the variants.
    open_qwen_thinking_picker(&mut app);
    let picker = app.picker.as_ref().unwrap();
    let ids: Vec<&str> = picker.items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids, vec!["off", "low", "medium", "xhigh", "budget"]);
    assert!(picker.budget.is_some());
}

#[test]
fn test_thinking_picker_budget_draft_seeding() {
    // Active budget:<n> variant wins.
    let mut app = App::new();
    app.active_thinking_variant = Some("budget:4096".into());
    open_qwen_thinking_picker(&mut app);
    assert_eq!(budget_draft(&app), "4096");

    // Active effort maps through by_effort.
    let mut app = App::new();
    app.active_thinking_variant = Some("medium".into());
    open_qwen_thinking_picker(&mut app);
    assert_eq!(budget_draft(&app), "16384");

    // Nothing set → metadata default.
    let mut app = App::new();
    open_qwen_thinking_picker(&mut app);
    assert_eq!(budget_draft(&app), "131072");
}

#[test]
fn test_budget_keys_step_type_and_backspace() {
    let mut app = App::new();
    open_qwen_thinking_picker(&mut app);
    select_budget_row(&mut app);

    // First digit replaces the seeded value, later digits append.
    assert!(
        crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Char('8')))
            .is_none()
    );
    assert_eq!(budget_draft(&app), "8");
    assert!(
        crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Char('1')))
            .is_none()
    );
    assert_eq!(budget_draft(&app), "81");

    // Right steps up by one step, snapped; Left steps back down.
    crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Right));
    assert_eq!(budget_draft(&app), "1024");
    crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Left));
    assert_eq!(budget_draft(&app), "0");

    // Backspace pops digits; an emptied draft reseeds to the default.
    crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Right));
    assert_eq!(budget_draft(&app), "1024");
    crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Backspace));
    assert_eq!(budget_draft(&app), "102");
    crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Backspace));
    assert_eq!(budget_draft(&app), "10");
    crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Backspace));
    assert_eq!(budget_draft(&app), "1");
    crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Backspace));
    assert_eq!(budget_draft(&app), "131072");
}

#[test]
fn test_budget_clamps_at_range_edges() {
    let mut app = App::new();
    open_qwen_thinking_picker(&mut app);
    select_budget_row(&mut app);

    // Left at the default never goes below min.
    for _ in 0..200 {
        crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Left));
    }
    assert_eq!(budget_draft(&app), "0");

    // Right never goes above max.
    for _ in 0..300 {
        crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Right));
    }
    assert_eq!(budget_draft(&app), "262144");
}

#[test]
fn test_budget_enter_commits_budget_variant() {
    let mut app = App::new();
    open_qwen_thinking_picker(&mut app);
    select_budget_row(&mut app);

    // Type an on-step value, then Enter commits `budget:<snapped>` and closes.
    crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Char('8')));
    crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Char('1')));
    crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Char('9')));
    crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Char('2')));
    let action = crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Enter));
    assert_eq!(thinking_action(action), Some("budget:8192".into()));
    assert!(app.picker.is_none());

    // Reopen: Enter on an off-step typed value commits the snapped value.
    open_qwen_thinking_picker(&mut app);
    select_budget_row(&mut app);
    crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Char('1')));
    crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Char('9')));
    let action = crate::events::handle_key_event(&mut app, key(crossterm::event::KeyCode::Enter));
    assert_eq!(thinking_action(action), Some("budget:0".into()));
}

#[test]
fn test_budget_mouse_click_drag_commit() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

    let mut app = App::new();
    open_qwen_thinking_picker(&mut app);
    let rect = ratatui::layout::Rect::new(10, 5, 20, 1);
    app.picker
        .as_mut()
        .unwrap()
        .budget
        .as_mut()
        .unwrap()
        .track_rect = Some(rect);

    let mouse = |kind: MouseEventKind, column: u16, row: u16| MouseEvent {
        kind,
        column,
        row,
        modifiers: crossterm::event::KeyModifiers::NONE,
    };

    // Click on the track sets the draft (row/col are 1-based; the handler
    // subtracts 1). Column 15 → frac (14-10)/19 ≈ 0.21 → snap → 55296.
    assert!(crate::events::handle_mouse_event(
        &mut app,
        mouse(MouseEventKind::Down(MouseButton::Left), 15, 6)
    )
    .is_none());
    assert_eq!(budget_draft(&app), "55296");
    // The click also selects the budget row so arrows keep working.
    assert_eq!(
        app.picker.as_ref().unwrap().selected_item().unwrap().id,
        "budget"
    );

    // Drag to the right end.
    crate::events::handle_mouse_event(
        &mut app,
        mouse(MouseEventKind::Drag(MouseButton::Left), 30, 6),
    );
    assert_eq!(budget_draft(&app), "262144");

    // Release commits `budget:<n>` and keeps the picker open.
    let action = crate::events::handle_mouse_event(
        &mut app,
        mouse(MouseEventKind::Up(MouseButton::Left), 30, 6),
    );
    assert_eq!(thinking_action(action), Some("budget:262144".into()));
    assert!(app.picker.is_some());
}

#[test]
fn test_budget_mouse_wheel_nudges_and_commits() {
    use crossterm::event::{MouseEvent, MouseEventKind};

    let mut app = App::new();
    open_qwen_thinking_picker(&mut app);
    let rect = ratatui::layout::Rect::new(10, 5, 20, 1);
    app.picker
        .as_mut()
        .unwrap()
        .budget
        .as_mut()
        .unwrap()
        .track_rect = Some(rect);

    let mouse = |kind: MouseEventKind| MouseEvent {
        kind,
        column: 15,
        row: 6,
        modifiers: crossterm::event::KeyModifiers::NONE,
    };

    // Wheel down over the track nudges down one step and commits.
    let action = crate::events::handle_mouse_event(&mut app, mouse(MouseEventKind::ScrollDown));
    assert_eq!(thinking_action(action), Some("budget:130048".into()));
    // Wheel up nudges up.
    let action = crate::events::handle_mouse_event(&mut app, mouse(MouseEventKind::ScrollUp));
    assert_eq!(thinking_action(action), Some("budget:131072".into()));
    // Wheel outside the track is ignored.
    let off = MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 2,
        row: 2,
        modifiers: crossterm::event::KeyModifiers::NONE,
    };
    assert!(crate::events::handle_mouse_event(&mut app, off).is_none());
}

#[test]
fn test_model_list_populates_thinking_budget() {
    let mut app = App::new();
    assert!(app.thinking_budget.is_empty());

    let msg = mew_protocol::ServerMessage::ModelList {
        models: vec![
            mew_protocol::ModelInfo {
                id: "qwen/qwen3.8-max".into(),
                provider: "qwen".into(),
                model: "qwen3.8-max".into(),
                description: None,
                thinking_variants: vec![],
                thinking_budget: Some(qwen_budget()),
                context_window: None,
            },
            mew_protocol::ModelInfo {
                id: "deepseek/deepseek-v4-flash".into(),
                provider: "deepseek".into(),
                model: "deepseek-v4-flash".into(),
                description: None,
                thinking_variants: vec![],
                thinking_budget: None,
                context_window: None,
            },
        ],
    };
    app.apply_daemon_notification(&msg);

    assert!(app.thinking_budget.contains_key("qwen3.8-max"));
    assert!(!app.thinking_budget.contains_key("deepseek-v4-flash"));
    let budget = &app.thinking_budget["qwen3.8-max"];
    assert_eq!(
        (budget.min, budget.max, budget.step, budget.default),
        (0, 262_144, 1024, 131_072)
    );
}

#[test]
fn test_ctrl_1_maps_to_environment_toggle() {
    use crate::events::Action;
    let mut app = App::new();
    let key = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('1'),
        crossterm::event::KeyModifiers::CONTROL,
    );
    let action = crate::events::handle_key_event(&mut app, key);
    assert!(matches!(action, Some(Action::ToggleSidebarEnvironment)));
}

#[test]
fn test_environment_toggle_first_press_expands() {
    // Environment defaults to collapsed, so the first toggle must expand
    // it (regression guard: the toggle must know the section's default).
    let mut app = App::new();
    assert!(App::sidebar_default_collapsed("environment"));
    app.toggle_sidebar_section("environment");
    assert_eq!(app.sidebar_collapsed.get("environment"), Some(&false));
    app.toggle_sidebar_section("environment");
    assert_eq!(app.sidebar_collapsed.get("environment"), Some(&true));
}
