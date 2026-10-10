use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use mew_tui::{
    events::{handle_input_event, Action},
    harness::Harness,
};
use std::time::{Duration, Instant};

fn key(h: &mut Harness, code: KeyCode) {
    handle_input_event(
        &mut h.app,
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE)),
    );
}

fn open_job(h: &mut Harness) {
    h.app.status.session_id = "owner".into();
    h.app.handle_agent_event(mew_agent::AgentEvent::JobUpdate {
        job_id: "job".into(),
        command: "cargo test".into(),
        state: "running".into(),
    });
    h.app.open_job("job".into());
}

fn request(h: &mut Harness, now: Instant) -> (String, u64) {
    let Action::ReadJobOutput {
        session_id,
        job_id,
        request_id,
        offset,
    } = h.app.take_activity_request(now).expect("output request")
    else {
        panic!("expected output read")
    };
    assert_eq!(session_id, "owner");
    assert_eq!(job_id, "job");
    (request_id, offset)
}

#[test]
fn job_view_reads_chunks_and_rejects_stale_owners_ids_and_offsets() {
    let mut h = Harness::new(128, 40);
    open_job(&mut h);
    let now = Instant::now();
    let (rid, offset) = request(&mut h, now);
    assert_eq!(offset, 0);
    for (owner, job, id, start, end) in [
        ("other", "job", rid.as_str(), 0, 4),
        ("owner", "other", rid.as_str(), 0, 4),
        ("owner", "job", "wrong", 0, 4),
        ("owner", "job", rid.as_str(), 1, 5),
        ("owner", "job", rid.as_str(), 0, 1),
    ] {
        h.app
            .apply_job_output(owner, job, id, start, end, "bad\n", false, false);
        assert!(!h.render().contains("bad"));
    }
    h.app
        .apply_job_output("owner", "job", &rid, 0, 4, "one\n", false, false);
    let (next, offset) = request(&mut h, now + Duration::from_secs(1));
    assert_eq!(offset, 4);
    h.app
        .apply_job_output("owner", "job", &rid, 0, 4, "duplicate\n", true, false);
    h.app
        .apply_job_output("owner", "job", &next, 4, 11, "日本\n", true, true);
    let frame = h.render();
    assert!(
        frame.contains("one")
            && frame.lines().any(|line| line
                .chars()
                .collect::<Vec<_>>()
                .windows(3)
                .any(|cells| cells[0] == '日' && cells[2] == '本'))
            && frame.contains("Output incomplete")
            && frame.contains("r refresh")
            && frame.contains("Esc close"),
        "{frame}"
    );
    assert!(!frame.contains("duplicate"));
    assert_eq!(
        h.app
            .activity_view
            .as_ref()
            .unwrap()
            .job_output
            .as_ref()
            .unwrap()
            .output,
        "one\n日本\n"
    );
    assert!(h
        .app
        .take_activity_request(now + Duration::from_secs(4))
        .is_none());
    assert!(h.app.messages().is_empty());
    key(&mut h, KeyCode::Char('r'));
    let (fresh, offset) = request(&mut h, now + Duration::from_secs(4));
    assert_eq!(
        offset, 11,
        "refresh must continue from the collected offset"
    );
    h.app
        .apply_job_output_error("owner", "job", &fresh, "job no longer available");
    assert!(h.render().contains("job no longer available"));
    key(&mut h, KeyCode::Esc);
    h.app
        .apply_job_output("owner", "job", &fresh, 11, 15, "late", true, false);
    assert!(h.app.activity_view.is_none());
}

#[test]
fn job_view_timeout_retains_collected_output_and_refresh_retries() {
    let mut h = Harness::new(80, 24);
    open_job(&mut h);
    let now = Instant::now();
    let (rid, _) = request(&mut h, now);
    h.app
        .apply_job_output("owner", "job", &rid, 0, 5, "kept\n", false, false);
    request(&mut h, now + Duration::from_secs(1));
    assert!(h
        .app
        .take_activity_request(now + Duration::from_secs(5))
        .is_none());
    let frame = h.render();
    assert!(
        frame.contains("kept") && frame.contains("not confirmed"),
        "{frame}"
    );
    key(&mut h, KeyCode::Char('r'));
    assert_eq!(request(&mut h, now + Duration::from_secs(6)).1, 5);
    h.app.attaching_session = true;
    assert!(h
        .app
        .take_activity_request(now + Duration::from_secs(10))
        .is_none());
}

#[test]
fn job_end_scroll_reaches_output_beyond_u16_rows_after_resize() {
    let mut h = Harness::new(128, 40);
    open_job(&mut h);
    let mut output = "row\n".repeat(70_000);
    output.push_str("final-output-tail\n");
    let mut offset = 0;
    let now = Instant::now();
    for chunk in output.as_bytes().chunks(65_536) {
        let (rid, start) = request(&mut h, now + Duration::from_secs(1));
        assert_eq!(start, offset);
        let text = std::str::from_utf8(chunk).unwrap();
        let next = offset + chunk.len() as u64;
        h.app.apply_job_output(
            "owner",
            "job",
            &rid,
            offset,
            next,
            text,
            next == output.len() as u64,
            false,
        );
        offset = next;
    }
    for (w, height) in [(128, 40), (30, 14), (80, 24)] {
        h.resize(w, height);
        h.render();
        key(&mut h, KeyCode::End);
        let frame = h.render();
        assert!(
            frame.contains("final-output-tail") && frame.contains("Esc close"),
            "{frame}"
        );
        assert!(h.app.activity_view.as_ref().unwrap().scroll > 65_535);
    }
}

fn tool_message(output: &str, error: Option<&str>) -> mew_message::Message {
    let state = if let Some(error) = error {
        serde_json::json!({"status":"error","input":{"command":"run full-command-tail"},"error":error,"time":{"start":0,"end":1}})
    } else {
        serde_json::json!({"status":"completed","input":{"command":"run full-command-tail"},"output":output,"time":{"start":0,"end":1}})
    };
    serde_json::from_value(serde_json::json!({"id":"01ARZ3NDEKTSV4RRFFQ69G5FA1","session_id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","role":"assistant","time":{"created":0},"parts":[{"type":"tool_call","id":"01ARZ3NDEKTSV4RRFFQ69G5FA2","message_id":"01ARZ3NDEKTSV4RRFFQ69G5FA1","session_id":"01ARZ3NDEKTSV4RRFFQ69G5FAV","tool_name":"bash","call_id":"call","state":state}]})).unwrap()
}

#[test]
fn tool_picker_opens_full_canonical_args_results_and_errors() {
    let mut h = Harness::new(128, 40);
    let message = tool_message("full-result-tail", None);
    h.app.push_message(message);
    h.app.open_tool_picker();
    assert!(h.render().contains("bash"));
    key(&mut h, KeyCode::Enter);
    let frame = h.render();
    assert!(
        frame.contains("full-command-tail")
            && frame.contains("full-result-tail")
            && frame.contains("Completed")
            && frame.contains("Esc close"),
        "{frame}"
    );
    assert!(h.app.take_activity_request(Instant::now()).is_none());
    key(&mut h, KeyCode::Esc);
    h.app.clear_messages();
    let message = tool_message("", Some("actual full failure details"));
    let id = message.id;
    let part = message.parts[0].id();
    h.app.push_message(message);
    h.app.open_tool(id, part);
    assert!(h.render().contains("actual full failure details"));
    h.app.status.session_id = "changed".into();
    let frame = h.render();
    assert!(
        frame.contains("Session changed") && !frame.contains("actual full failure details"),
        "{frame}"
    );
}

#[test]
fn review_requests_pin_owner_and_request_and_render_hunks_with_refresh() {
    let mut h = Harness::new(128, 40);
    h.app.status.session_id = "owner".into();
    h.app.open_review();
    let now = Instant::now();
    let Action::ReadSessionDiff {
        session_id,
        request_id,
    } = h.app.take_activity_request(now).unwrap()
    else {
        panic!("expected diff read")
    };
    assert_eq!(session_id, "owner");
    let files: Vec<_>=serde_json::from_value(serde_json::json!([{ "path":"src/session.rs","old_path":null,"status":"Modified","added":1,"removed":1,"hunks":[{"old_start":1,"old_count":1,"new_start":1,"new_count":1,"lines":[{"kind":"Deletion","old_line":1,"new_line":null,"text":"old owner"},{"kind":"Addition","old_line":null,"new_line":1,"text":"retained owner"}]}]}])).unwrap();
    h.app.apply_session_diff("other", &request_id, &files, None);
    h.app.apply_session_diff("owner", "stale", &files, None);
    assert!(!h.render().contains("retained owner"));
    h.app.apply_session_diff("owner", &request_id, &files, None);
    let frame = h.render();
    for text in [
        "Review",
        "src/session.rs",
        "old owner",
        "retained owner",
        "r refresh",
        "Esc close",
    ] {
        assert!(frame.contains(text), "missing {text}: {frame}");
    }
    assert!(h
        .app
        .take_activity_request(now + Duration::from_secs(3))
        .is_none());
    h.app.invalidate_review("other");
    assert!(h
        .app
        .take_activity_request(now + Duration::from_secs(3))
        .is_none());
    h.app.invalidate_review("owner");
    let Action::ReadSessionDiff {
        request_id: fresh, ..
    } = h
        .app
        .take_activity_request(now + Duration::from_secs(3))
        .unwrap()
    else {
        panic!()
    };
    assert_ne!(fresh, request_id);
    h.app.apply_session_diff("owner", &request_id, &[], None);
    assert!(h.render().contains("retained owner"));
    h.app.apply_session_diff("owner", &fresh, &[], None);
    assert!(h.render().contains("No workspace changes"));
    key(&mut h, KeyCode::Char('r'));
    let Action::ReadSessionDiff { request_id, .. } = h
        .app
        .take_activity_request(now + Duration::from_secs(4))
        .unwrap()
    else {
        panic!()
    };
    h.app
        .apply_session_diff("owner", &request_id, &[], Some("not a git repository"));
    assert!(h.render().contains("not a git repository"));
}

#[test]
fn job_unicode_redraw_clears_wide_continuation_cells_in_the_actual_frame() {
    let mut h = Harness::new(128, 40);
    open_job(&mut h);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(128, 40)).unwrap();
    terminal
        .draw(|frame| mew_tui::ui::draw(frame, &mut h.app))
        .unwrap();
    let (rid, offset) = request(&mut h, Instant::now());
    h.app
        .apply_job_output("owner", "job", &rid, offset, 7, "日本\n", true, false);
    let frame = terminal
        .draw(|frame| mew_tui::ui::draw(frame, &mut h.app))
        .unwrap();
    let index = frame
        .buffer
        .content
        .iter()
        .position(|cell| cell.symbol() == "日")
        .unwrap();
    assert_eq!(frame.buffer.content[index + 1].symbol(), " ");
    assert_eq!(frame.buffer.content[index + 2].symbol(), "本");
    assert_eq!(frame.buffer.content[index + 3].symbol(), " ");
    let surface = h.app.theme.resolve("panel.overlay");
    for cell in &frame.buffer.content[index..index + 4] {
        assert_eq!(cell.bg, surface);
    }
}

#[test]
fn job_output_renders_ansi_text_without_escape_control_codes() {
    let mut h = Harness::new(128, 40);
    open_job(&mut h);
    let (rid, offset) = request(&mut h, Instant::now());
    let output = "\x1b[31mred output\x1b[0m\n";
    h.app.apply_job_output(
        "owner",
        "job",
        &rid,
        offset,
        output.len() as u64,
        output,
        true,
        false,
    );
    let frame = h.render();
    assert!(frame.contains("red output"), "{frame}");
    assert!(!frame.contains("[31m") && !frame.contains("[0m"), "{frame}");
    assert_eq!(
        h.app
            .activity_view
            .as_ref()
            .unwrap()
            .job_output
            .as_ref()
            .unwrap()
            .output,
        output
    );
}

#[test]
fn unicode_safe_full_chunks_continue_immediately_at_each_byte_boundary() {
    for byte_count in 65_533..=65_536 {
        let mut h = Harness::new(80, 24);
        open_job(&mut h);
        let (rid, offset) = request(&mut h, Instant::now());
        let chunk = format!("{}日", "x".repeat(byte_count - 3));
        h.app.apply_job_output(
            "owner",
            "job",
            &rid,
            offset,
            byte_count as u64,
            &chunk,
            false,
            false,
        );
        assert!(
            h.app.take_activity_request(Instant::now()).is_some(),
            "{byte_count}-byte UTF-8-safe full chunk was delayed"
        );
    }
}
