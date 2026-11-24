use mew_agy::transcript::{project, system_instructions};
use mew_message::{
    CompactionPart, FilePart, Message, Part, PartBase, ReasoningPart, Role, TextPart, Time,
    ToolCallPart, ToolResultPart, ToolState, ToolStateCompleted, ToolStateError, ToolStateRunning,
    ToolTime,
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
fn rejects_orphans_duplicate_ids_unfinished_calls_and_attachments() {
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
            Role::User,
            vec![Part::File(FilePart {
                base: base(),
                mime: "image/png".into(),
                filename: None,
                url: "file:///private".into(),
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
}

#[test]
fn rejects_non_object_inputs_duplicate_results_and_tool_images() {
    let mut image = match completed("ok") {
        ToolState::Completed(value) => value,
        _ => unreachable!(),
    };
    image.images.push(mew_message::ToolImage {
        mime: "image/png".into(),
        data: "c2VjcmV0".into(),
    });
    let mut malformed = match completed("ok") {
        ToolState::Completed(value) => value,
        _ => unreachable!(),
    };
    malformed.input = json!(null);
    let cases = vec![
        vec![
            message(
                Role::Assistant,
                vec![call("x", ToolState::Completed(image))],
            ),
            message(Role::User, vec![result("x")]),
        ],
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
