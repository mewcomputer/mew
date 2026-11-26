use mew_agy::transcript::{project, prompt_blocks, system_instructions};
use mew_message::{
    CompactionPart, FilePart, Message, Part, PartBase, ReasoningPart, Role, TextPart, Time,
    ToolCallPart, ToolImage, ToolResultPart, ToolState, ToolStateCompleted, ToolStateError,
    ToolStateRunning, ToolTime,
};
use mew_provider::Request;
use serde_json::{json, Value};
use ulid::Ulid;

fn base() -> PartBase {
    PartBase {
        id: Ulid::new(),
        message_id: Ulid::new(),
        session_id: Ulid::new(),
    }
}
fn message(role: Role, parts: Vec<Part>) -> Message {
    Message {
        id: Ulid::new(),
        session_id: Ulid::new(),
        role,
        parts,
        time: Time {
            created: 0,
            completed: None,
        },
        assistant: None,
    }
}
fn text(s: &str) -> Part {
    Part::Text(TextPart {
        base: base(),
        text: s.into(),
        synthetic: false,
    })
}
fn call(id: &str, state: ToolState) -> Part {
    Part::ToolCall(ToolCallPart {
        base: base(),
        tool_name: "write".into(),
        call_id: id.into(),
        state,
        raw_input: "private raw input".into(),
    })
}
fn completed(output: &str) -> ToolState {
    ToolState::Completed(ToolStateCompleted {
        input: json!({"path":"a\"\\b","value":"one\ntwo"}),
        output: output.into(),
        metadata: Some(json!({"internal":"SECRET"})),
        diff: Some("secret diff".into()),
        images: vec![],
        time: ToolTime {
            start: 0,
            end: Some(1),
        },
    })
}
fn result(id: &str) -> Part {
    Part::ToolResult(ToolResultPart {
        base: base(),
        call_id: id.into(),
    })
}

#[test]
fn preserves_order_and_exact_tool_input_and_output_but_not_private_metadata() {
    let messages = vec![
        message(Role::User, vec![text("hello\n<end> \"€")]),
        message(
            Role::Assistant,
            vec![
                text("doing it"),
                call("call-1", completed("line1\n\"line2\"")),
            ],
        ),
        message(Role::User, vec![result("call-1"), text("next")]),
    ];
    let req = Request {
        system: "system\n</system>".into(),
        messages,
        ..Default::default()
    };
    let encoded = project(&req).unwrap();
    let v: Value = serde_json::from_str(&encoded).unwrap();
    assert!(v.get("system").is_none());
    assert_eq!(v["messages"][0]["content"][0]["text"], "hello\n<end> \"€");
    assert_eq!(
        v["messages"][1]["content"][1]["input"],
        json!({"path":"a\"\\b","value":"one\ntwo"})
    );
    assert_eq!(
        v["messages"][2]["content"][0]["content"],
        "line1\n\"line2\""
    );
    assert!(!encoded.contains("SECRET"));
    assert!(!encoded.contains("private raw input"));
    assert!(!encoded.contains("secret diff"));
}

#[test]
fn oversized_tool_results_are_replaced_only_in_the_projected_history() {
    for (output, was_error) in [
        ("x".repeat(1_048_577), false),
        ("\u{0000}".repeat(180_000), true),
    ] {
        let state = if was_error {
            ToolState::Error(ToolStateError {
                input: json!({}),
                error: output.clone(),
                time: ToolTime {
                    start: 0,
                    end: Some(1),
                },
            })
        } else {
            completed(&output)
        };
        let req = Request {
            messages: vec![
                message(Role::Assistant, vec![call("large", state)]),
                message(Role::User, vec![result("large")]),
            ],
            ..Default::default()
        };
        let projected: Value = serde_json::from_str(&project(&req).unwrap()).unwrap();
        let result = &projected["messages"][1]["content"][0];
        assert_eq!(result["is_error"], true);
        let message = result["content"].as_str().unwrap();
        assert!(message.contains("offset") && message.contains("limit"));
        match &req.messages[0].parts[0] {
            Part::ToolCall(call) => match &call.state {
                ToolState::Completed(state) => assert_eq!(state.output, output),
                ToolState::Error(state) => assert_eq!(state.error, output),
                _ => panic!("unexpected state"),
            },
            _ => panic!("expected tool call"),
        }
    }
}

#[test]
fn historical_result_limit_matches_encoded_mcp_response_boundary() {
    for is_error in [false, true] {
        let overhead =
            serde_json::to_vec(&json!({"content":[{"type":"text","text":""}],"isError":is_error}))
                .unwrap()
                .len();
        for extra in [0, 1] {
            let output = "x".repeat(1_048_576 - overhead + extra);
            let state = if is_error {
                ToolState::Error(ToolStateError {
                    input: json!({}),
                    error: output.clone(),
                    time: ToolTime {
                        start: 0,
                        end: Some(1),
                    },
                })
            } else {
                completed(&output)
            };
            let req = Request {
                messages: vec![
                    message(Role::Assistant, vec![call("limit", state)]),
                    message(Role::User, vec![result("limit")]),
                ],
                ..Default::default()
            };
            let projected: Value = serde_json::from_str(&project(&req).unwrap()).unwrap();
            let item = &projected["messages"][1]["content"][0];
            if extra == 0 {
                assert_eq!(item["content"], output);
                assert_eq!(item["is_error"], is_error);
            } else {
                assert_eq!(item["is_error"], true);
                assert!(item["content"].as_str().unwrap().contains("offset"));
            }
        }
    }
}

#[test]
fn tool_errors_and_compaction_are_preserved_once_without_reasoning() {
    let err = ToolState::Error(ToolStateError {
        input: json!({}),
        error: "denied\nby user".into(),
        time: ToolTime {
            start: 0,
            end: Some(1),
        },
    });
    let summary="Earlier context was compacted. This record contains a limited selection of earlier user messages. The full conversation remains in the transcript.\n\nold text";
    let compact = Part::Compaction(CompactionPart {
        base: base(),
        auto: true,
        overflow: false,
        tail_start_id: None,
        summary: Some(summary.into()),
        removed_count: Some(3),
        preserved_context: vec![],
    });
    let reasoning = Part::Reasoning(ReasoningPart {
        base: base(),
        text: "INTERNAL_THOUGHT".into(),
        signature: Some("SIGNATURE".into()),
        encrypted_content: Some("CIPHERTEXT".into()),
        provider_item_id: None,
    });
    let req = Request {
        messages: vec![
            message(Role::User, vec![compact, text(summary)]),
            message(Role::Assistant, vec![reasoning, call("x", err)]),
            message(Role::User, vec![result("x")]),
        ],
        ..Default::default()
    };
    let encoded = project(&req).unwrap();
    let v: Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(encoded.matches("old text").count(), 1);
    assert_eq!(v["messages"][2]["content"][0]["content"], "denied\nby user");
    assert_eq!(v["messages"][2]["content"][0]["is_error"], true);
    assert!(!encoded.contains("INTERNAL_THOUGHT"));
    assert!(!encoded.contains("CIPHERTEXT"));
    assert!(!encoded.contains("SIGNATURE"));
}

#[test]
fn rejects_orphans_duplicate_ids_unfinished_calls_and_invalid_attachments() {
    let time = ToolTime {
        start: 0,
        end: None,
    };
    let cases = vec![
        vec![message(Role::User, vec![result("orphan")])],
        vec![
            message(
                Role::Assistant,
                vec![call("x", completed("ok")), call("x", completed("ok"))],
            ),
            message(Role::User, vec![result("x")]),
        ],
        vec![message(Role::Assistant, vec![call("x", completed("ok"))])],
        vec![message(
            Role::Assistant,
            vec![call(
                "x",
                ToolState::Running(ToolStateRunning {
                    input: json!({}),
                    output: String::new(),
                    time,
                }),
            )],
        )],
        vec![message(
            Role::System,
            vec![Part::File(FilePart {
                base: base(),
                mime: "image/png".into(),
                filename: None,
                url: "data:image/png;base64,AAAA".into(),
            })],
        )],
    ];
    for messages in cases {
        assert!(project(&Request {
            messages,
            ..Default::default()
        })
        .is_err())
    }
    let bad_url = Request {
        messages: vec![message(
            Role::User,
            vec![Part::File(FilePart {
                base: base(),
                mime: "image/png".into(),
                filename: None,
                url: "https://example.invalid/secret.png".into(),
            })],
        )],
        ..Default::default()
    };
    assert!(prompt_blocks(&bad_url).is_err());
}

#[test]
fn rejects_non_object_inputs_and_duplicate_results_while_projecting_tool_images() {
    let mut with_image = match completed("ok") {
        ToolState::Completed(value) => value,
        _ => unreachable!(),
    };
    with_image.images.push(ToolImage {
        mime: "image/png".into(),
        data: "c2VjcmV0".into(),
    });
    let with_image_req = Request {
        messages: vec![
            message(
                Role::Assistant,
                vec![call("x", ToolState::Completed(with_image))],
            ),
            message(Role::User, vec![result("x")]),
        ],
        ..Default::default()
    };
    let projected: Value = serde_json::from_str(&project(&with_image_req).unwrap()).unwrap();
    assert_eq!(
        projected["messages"][1]["content"][0]["images"][0],
        json!({"mime": "image/png", "data": "c2VjcmV0"})
    );

    let mut malformed = match completed("ok") {
        ToolState::Completed(value) => value,
        _ => unreachable!(),
    };
    malformed.input = json!(null);
    let cases = vec![
        vec![
            message(
                Role::Assistant,
                vec![call("x", ToolState::Completed(malformed))],
            ),
            message(Role::User, vec![result("x")]),
        ],
        vec![
            message(Role::Assistant, vec![call("x", completed("ok"))]),
            message(Role::User, vec![result("x"), result("x")]),
        ],
    ];
    for messages in cases {
        assert!(project(&Request {
            messages,
            ..Default::default()
        })
        .is_err());
    }
}

#[test]
fn prompt_blocks_emit_user_images_and_text_resources_in_order() {
    let note = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(note.path(), "hello file").unwrap();
    let file_url = url::Url::from_file_path(note.path()).unwrap().to_string();
    let bare_path = note.path().to_string_lossy().into_owned();
    let req = Request {
        messages: vec![message(
            Role::User,
            vec![
                text("check these"),
                Part::File(FilePart {
                    base: base(),
                    mime: "image/png".into(),
                    filename: Some("dot.png".into()),
                    url: "data:image/png;base64,iVBORw0KGgo=".into(),
                }),
                Part::File(FilePart {
                    base: base(),
                    mime: "text/plain".into(),
                    filename: Some("note.txt".into()),
                    url: file_url.clone(),
                }),
                Part::File(FilePart {
                    base: base(),
                    mime: "text/plain".into(),
                    filename: Some("bare.txt".into()),
                    url: bare_path,
                }),
            ],
        )],
        ..Default::default()
    };
    let blocks = prompt_blocks(&req).unwrap();
    assert_eq!(blocks.len(), 4);
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(
        blocks[1],
        json!({"type": "image", "mimeType": "image/png", "data": "iVBORw0KGgo="})
    );
    assert_eq!(
        blocks[2],
        json!({
            "type": "resource",
            "resource": {
                "uri": file_url,
                "mimeType": "text/plain",
                "text": "hello file",
            }
        })
    );
    assert_eq!(
        blocks[3],
        json!({
            "type": "resource",
            "resource": {
                "uri": "file:///bare.txt",
                "mimeType": "text/plain",
                "text": "hello file",
            }
        })
    );
}

#[test]
fn retains_compaction_summary_and_preserved_context_without_leaking_headers() {
    let compact = Part::Compaction(CompactionPart {
        base: base(),
        auto: false,
        overflow: false,
        tail_start_id: None,
        summary: Some("summary\nverbatim".into()),
        removed_count: None,
        preserved_context: vec![message(Role::User, vec![text("preserved file content")])],
    });
    let mut req = Request {
        messages: vec![message(Role::System, vec![compact])],
        ..Default::default()
    };
    req.headers.insert(
        "authorization",
        "Bearer PRIVATE_CREDENTIAL".parse().unwrap(),
    );
    let encoded = project(&req).unwrap();
    let v: Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(
        v["messages"][0]["content"][0]["summary"],
        "summary\nverbatim"
    );
    assert_eq!(
        v["messages"][0]["content"][1]["messages"][0]["content"][0]["text"],
        "preserved file content"
    );
    assert!(!encoded.contains("PRIVATE_CREDENTIAL"));
}

#[test]
fn system_messages_are_in_agent_instructions_once_not_in_history() {
    let req = Request {
        system: "base <literal>".into(),
        messages: vec![
            message(Role::System, vec![text("first\n---\nΩ")]),
            message(Role::User, vec![text("ordinary")]),
            message(Role::System, vec![text("second")]),
        ],
        ..Default::default()
    };
    assert_eq!(
        system_instructions(&req).unwrap(),
        "base <literal>\n\nfirst\n---\nΩ\n\nsecond"
    );
    let encoded = project(&req).unwrap();
    let v: Value = serde_json::from_str(&encoded).unwrap();
    assert!(!encoded.contains("base <literal>"));
    assert!(!encoded.contains("first"));
    assert!(!encoded.contains("second"));
    assert_eq!(v["messages"][0]["content"][0]["text"], "ordinary");
    assert_eq!(system_instructions(&Request::default()).unwrap(), "");
}

#[test]
fn decodes_data_urls_and_rejects_malformed_non_regular_or_oversized_file_attachments() {
    let data_req = Request {
        messages: vec![message(
            Role::User,
            vec![
                Part::File(FilePart {
                    base: base(),
                    mime: "text/plain".into(),
                    filename: Some("encoded.txt".into()),
                    url: "data:text/plain;base64,aGVsbG8gd29ybGQ=".into(),
                }),
                Part::File(FilePart {
                    base: base(),
                    mime: "text/plain".into(),
                    filename: Some("percent.txt".into()),
                    url: "data:text/plain,hello%20world".into(),
                }),
            ],
        )],
        ..Default::default()
    };
    let blocks = prompt_blocks(&data_req).unwrap();
    assert_eq!(blocks[1]["resource"]["text"], "hello world");
    assert_eq!(blocks[2]["resource"]["text"], "hello world");

    let dir = tempfile::tempdir().unwrap();
    let dir_url = url::Url::from_directory_path(dir.path())
        .unwrap()
        .to_string();
    let oversized_path = dir.path().join("huge.txt");
    let oversized = std::fs::File::create(&oversized_path).unwrap();
    oversized.set_len(16 * 1024 * 1024 + 1).unwrap();
    let oversized_url = url::Url::from_file_path(&oversized_path)
        .unwrap()
        .to_string();

    for url in [
        "file://remote-host/etc/passwd".to_string(),
        dir_url,
        oversized_url,
    ] {
        let bad_req = Request {
            messages: vec![message(
                Role::User,
                vec![Part::File(FilePart {
                    base: base(),
                    mime: "text/plain".into(),
                    filename: None,
                    url,
                })],
            )],
            ..Default::default()
        };
        assert!(prompt_blocks(&bad_req).is_err());
    }
}

#[test]
fn prefixed_rendered_compaction_summary_is_not_duplicated() {
    let mut rendered = match text("Handoff context follows:\n\nConcise checkpoint summary.") {
        Part::Text(part) => part,
        _ => unreachable!(),
    };
    rendered.synthetic = true;
    let compact = Part::Compaction(CompactionPart {
        base: base(),
        auto: true,
        overflow: false,
        tail_start_id: None,
        summary: Some("Concise checkpoint summary.".into()),
        removed_count: Some(4),
        preserved_context: vec![],
    });
    let req = Request {
        messages: vec![message(Role::User, vec![Part::Text(rendered), compact])],
        ..Default::default()
    };
    let encoded = project(&req).unwrap();
    let v: Value = serde_json::from_str(&encoded).unwrap();
    let content = v["messages"][0]["content"].as_array().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "text");
    assert_eq!(
        content[0]["text"],
        "Handoff context follows:\n\nConcise checkpoint summary."
    );
}

#[test]
fn prompt_blocks_places_latest_user_message_after_historical_attachments() {
    let req = Request {
        messages: vec![
            message(
                Role::User,
                vec![
                    text("controls are going off screen"),
                    Part::File(FilePart {
                        base: base(),
                        mime: "image/png".into(),
                        filename: Some("old.png".into()),
                        url: "data:image/png;base64,iVBORw0KGgo=".into(),
                    }),
                ],
            ),
            message(Role::Assistant, vec![text("fixed the portrait layout")]),
            message(Role::User, vec![text("can you deploy on surge.sh?")]),
        ],
        ..Default::default()
    };

    let blocks = prompt_blocks(&req).unwrap();
    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[0]["type"], "text");
    assert!(blocks[0]["text"]
        .as_str()
        .unwrap()
        .contains("controls are going off screen"));
    assert!(!blocks[0]["text"]
        .as_str()
        .unwrap()
        .contains("can you deploy on surge.sh?"));
    assert_eq!(blocks[1]["type"], "image");
    assert_eq!(blocks[2]["type"], "text");
    assert_eq!(
        blocks[2]["text"],
        "Latest user message:\ncan you deploy on surge.sh?"
    );
}
