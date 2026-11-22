use mew_agy::stream::{CliEvent, CliStream};

#[test]
fn streams_fragmented_text_and_preserves_unicode() {
    let mut stream = CliStream::new(4096);
    let input = "{\"event\":\"step_update\",\"step_update\":{\"step_type\":\"agent_response\",\"text_delta\":\"café\"}}\n".as_bytes();
    let split = input.iter().position(|b| *b == 0xc3).unwrap() + 1;
    assert!(stream.push(&input[..split]).unwrap().is_empty());
    assert_eq!(
        stream.push(&input[split..]).unwrap(),
        vec![CliEvent::Text("café".into())]
    );
}

#[test]
fn terminal_success_requires_clean_result_and_emits_usage() {
    let mut stream = CliStream::new(4096);
    let events = stream.push(br#"{"event":"step_update","step_update":{"state":"DONE","usage":{"input_tokens":17,"output_tokens":5,"cache_read_tokens":3}}}
{"event":"result","result":{"status":"SUCCESS","denied_actions":[]}}
"#).unwrap();
    assert_eq!(
        events,
        vec![
            CliEvent::Usage {
                input: 17,
                output: 5,
                cache_read: 3
            },
            CliEvent::Finished
        ]
    );
    stream.finish().unwrap();
}

#[test]
fn denied_actions_and_error_results_are_not_success() {
    for result in [
        r#"{"status":"SUCCESS","denied_actions":["read_file"]}"#,
        r#"{"status":"ERROR","error":"quota reached"}"#,
    ] {
        let mut stream = CliStream::new(4096);
        let input = format!("{{\"event\":\"result\",\"result\":{result}}}\n");
        let error = stream.push(input.as_bytes()).unwrap_err();
        if result.contains("quota reached") {
            assert!(error.to_string().contains("quota reached"));
        }
    }
}

#[test]
fn rejects_truncated_oversized_and_post_result_streams() {
    let mut truncated = CliStream::new(4096);
    truncated.push(b"{\"event\":\"init\"}").unwrap();
    assert!(truncated.finish().is_err());
    let mut oversized = CliStream::new(8);
    assert!(oversized.push(b"123456789").is_err());
    let mut finished = CliStream::new(4096);
    finished
        .push(b"{\"event\":\"result\",\"result\":{\"status\":\"SUCCESS\"}}\n")
        .unwrap();
    assert!(finished.push(b"{\"event\":\"init\"}\n").is_err());
}
