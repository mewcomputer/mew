//! Session persistence round-trip tests.
//!
//! Exercises the public API: write messages via `Writer`, load them back
//! via `Reader`, and assert they're equal. Catches breakage in the JSONL
//! serialization, the meta sidecar, or the directory layout.

use mew_message::{Message, MessageId, Part, PartBase, Role, SessionId, TextPart, Time};
use mew_session::{Meta, Reader, Writer};
use tempfile::TempDir;

fn sample_user_message(id: MessageId, sid: SessionId, text: &str) -> Message {
    Message {
        id,
        session_id: sid,
        role: Role::User,
        parts: vec![Part::Text(TextPart {
            base: PartBase {
                id: ulid::Ulid::new(),
                message_id: id,
                session_id: sid,
            },
            text: text.into(),
            synthetic: false,
        })],
        time: Time {
            created: chrono::Utc::now().timestamp_millis(),
            completed: None,
        },
        assistant: None,
    }
}

#[tokio::test]
async fn write_then_load_round_trip_preserves_messages() {
    let dir = TempDir::new().unwrap();
    let sid = SessionId::new();
    let mut writer = Writer::open_at(dir.path(), "sess-1").await.unwrap();

    let m1 = sample_user_message(MessageId::new(), sid, "hello");
    let m2 = sample_user_message(MessageId::new(), sid, "world");
    writer.write_message(&m1).await.unwrap();
    writer.write_message(&m2).await.unwrap();
    writer.flush().await.unwrap();

    let loaded = Reader::load_from(dir.path(), "sess-1").await.unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].id, m1.id);
    assert_eq!(loaded[1].id, m2.id);
    assert_eq!(loaded[0].role, Role::User);
}

#[tokio::test]
async fn empty_session_loads_as_empty_vec() {
    let dir = TempDir::new().unwrap();
    let _writer = Writer::open_at(dir.path(), "sess-empty").await.unwrap();
    // No messages written.
    let loaded = Reader::load_from(dir.path(), "sess-empty").await.unwrap();
    assert!(loaded.is_empty());
}

#[tokio::test]
async fn meta_is_persisted_with_session() {
    let dir = TempDir::new().unwrap();
    let mut meta = Meta::new("sess-meta");
    meta.model = Some("opencode-zen".into());
    meta.subagent_name = Some("explorer".into());
    let mut writer = Writer::open_at_with_meta(dir.path(), "sess-meta", meta.clone())
        .await
        .unwrap();
    writer
        .write_message(&sample_user_message(
            MessageId::new(),
            SessionId::new(),
            "hi",
        ))
        .await
        .unwrap();
    writer.flush().await.unwrap();

    let reader_meta = Reader::load_meta_from(dir.path(), "sess-meta")
        .await
        .expect("meta load should succeed")
        .expect("meta should exist");
    assert_eq!(reader_meta.model.as_deref(), Some("opencode-zen"));
    assert_eq!(reader_meta.subagent_name.as_deref(), Some("explorer"));
    assert_eq!(reader_meta.id, "sess-meta");
}

#[tokio::test]
async fn multiple_sessions_in_one_dir_are_independent() {
    let dir = TempDir::new().unwrap();

    let mut w1 = Writer::open_at(dir.path(), "a").await.unwrap();
    let mut w2 = Writer::open_at(dir.path(), "b").await.unwrap();
    w1.write_message(&sample_user_message(
        MessageId::new(),
        SessionId::new(),
        "from-a",
    ))
    .await
    .unwrap();
    w2.write_message(&sample_user_message(
        MessageId::new(),
        SessionId::new(),
        "from-b",
    ))
    .await
    .unwrap();
    w1.flush().await.unwrap();
    w2.flush().await.unwrap();

    let a = Reader::load_from(dir.path(), "a").await.unwrap();
    let b = Reader::load_from(dir.path(), "b").await.unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 1);

    // Each session loaded its own message — no cross-talk.
    let a_text = a[0].parts[0].id(); // any unique signal is enough
    let b_text = b[0].parts[0].id();
    assert_ne!(a_text, b_text);
}

#[tokio::test]
async fn loading_unknown_session_returns_error() {
    let dir = TempDir::new().unwrap();
    // No session was ever written.
    let result = Reader::load_from(dir.path(), "does-not-exist").await;
    assert!(
        result.is_err(),
        "loading an unknown session must surface an error"
    );
}

#[tokio::test]
async fn reopen_existing_session_appends_without_truncating() {
    let dir = TempDir::new().unwrap();
    let mut w1 = Writer::open_at(dir.path(), "append").await.unwrap();
    w1.write_message(&sample_user_message(
        MessageId::new(),
        SessionId::new(),
        "first",
    ))
    .await
    .unwrap();
    w1.flush().await.unwrap();

    // Reopen and append — must NOT lose the first message.
    let mut w2 = Writer::open_at(dir.path(), "append").await.unwrap();
    w2.write_message(&sample_user_message(
        MessageId::new(),
        SessionId::new(),
        "second",
    ))
    .await
    .unwrap();
    w2.flush().await.unwrap();

    let loaded = Reader::load_from(dir.path(), "append").await.unwrap();
    assert_eq!(loaded.len(), 2, "reopen must append, not truncate");
}

#[tokio::test]
async fn meta_context_tokens_round_trips() {
    let dir = TempDir::new().unwrap();
    let mut meta = Meta::new("sess-ctx");
    assert_eq!(
        meta.context_tokens, None,
        "fresh meta has no context reading"
    );
    meta.context_tokens = Some(12_345);
    let mut writer = Writer::open_at_with_meta(dir.path(), "sess-ctx", meta.clone())
        .await
        .unwrap();
    writer
        .write_message(&sample_user_message(
            MessageId::new(),
            SessionId::new(),
            "hi",
        ))
        .await
        .unwrap();
    writer.flush().await.unwrap();

    let loaded = Reader::load_meta_from(dir.path(), "sess-ctx")
        .await
        .expect("meta load should succeed")
        .expect("meta should exist");
    assert_eq!(loaded.context_tokens, Some(12_345));
}

#[tokio::test]
async fn appending_preserves_metadata_updated_after_the_writer_opened() {
    let dir = TempDir::new().unwrap();
    let mut writer = Writer::open_at(dir.path(), "edited").await.unwrap();
    Meta::update(dir.path(), "edited", |meta| {
        meta.custom_title = Some("Saved title".into());
        meta.pinned = true;
    })
    .await
    .unwrap()
    .unwrap();
    let child = Writer::open_subagent_at(dir.path(), "edited", "worker", "reader")
        .await
        .unwrap();
    child.close().await.unwrap();

    writer
        .write_message(&sample_user_message(
            MessageId::new(),
            SessionId::new(),
            "continue",
        ))
        .await
        .unwrap();

    let meta = Meta::read(dir.path(), "edited").await.unwrap().unwrap();
    assert_eq!(meta.custom_title.as_deref(), Some("Saved title"));
    assert!(meta.pinned);
    assert_eq!(meta.children_session_ids, ["worker"]);
    assert!(meta.last_message_at.is_some());
}

#[tokio::test]
async fn interrupted_final_record_is_recovered_and_the_session_stays_appendable() {
    let dir = TempDir::new().unwrap();
    let first = sample_user_message(MessageId::new(), SessionId::new(), "first");
    let interrupted = sample_user_message(
        MessageId::new(),
        first.session_id,
        &format!("{}🙂", "x".repeat(5000)),
    );
    let next = sample_user_message(MessageId::new(), first.session_id, "next");
    let encoded = serde_json::to_vec(&interrupted).unwrap();
    let emoji_start = encoded.iter().position(|byte| *byte == 0xf0).unwrap();

    for tail in [&encoded[..emoji_start], &encoded[..emoji_start + 2]] {
        let mut writer = Writer::open_at(dir.path(), "recovery").await.unwrap();
        let path = writer.path().clone();
        writer.close().await.unwrap();
        let mut records = serde_json::to_vec(&first).unwrap();
        records.push(b'\n');
        records.extend_from_slice(tail);
        tokio::fs::write(&path, &records).await.unwrap();

        assert_eq!(
            Reader::load_from(dir.path(), "recovery").await.unwrap(),
            [first.clone()]
        );
        assert_eq!(tokio::fs::read(&path).await.unwrap(), records);

        writer = Writer::open_at(dir.path(), "recovery").await.unwrap();
        writer.write_message(&next).await.unwrap();
        writer.close().await.unwrap();
        assert_eq!(
            Reader::load_from(dir.path(), "recovery").await.unwrap(),
            [first.clone(), next.clone()]
        );
    }
}

#[tokio::test]
async fn reopening_retains_a_valid_final_record_without_a_newline() {
    let dir = TempDir::new().unwrap();
    let writer = Writer::open_at(dir.path(), "final-record").await.unwrap();
    let path = writer.path().clone();
    writer.close().await.unwrap();
    let first = sample_user_message(MessageId::new(), SessionId::new(), "committed JSON");
    tokio::fs::write(&path, serde_json::to_vec(&first).unwrap())
        .await
        .unwrap();
    assert_eq!(
        Reader::load_from(dir.path(), "final-record").await.unwrap(),
        [first.clone()]
    );
    let next = sample_user_message(MessageId::new(), first.session_id, "next");

    let mut writer = Writer::open_at(dir.path(), "final-record").await.unwrap();
    writer.write_message(&next).await.unwrap();
    writer.close().await.unwrap();

    assert_eq!(
        Reader::load_from(dir.path(), "final-record").await.unwrap(),
        [first, next]
    );
}

#[tokio::test]
async fn malformed_complete_or_middle_records_are_reported() {
    let dir = TempDir::new().unwrap();
    let session_id = "corrupt";
    let writer = Writer::open_at(dir.path(), session_id).await.unwrap();
    let path = writer.path().clone();
    writer.close().await.unwrap();
    let good = sample_user_message(MessageId::new(), SessionId::new(), "good");
    let good = serde_json::to_string(&good).unwrap();

    for corrupted in [
        "{\"id\":\n".to_owned(),
        format!("{{\"id\":\n{good}\n"),
        format!("{good}\n{{broken"),
    ] {
        tokio::fs::write(&path, &corrupted).await.unwrap();
        assert!(Reader::load_from(dir.path(), session_id).await.is_err());
        assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), corrupted);
    }
}

#[tokio::test]
async fn concurrent_child_sessions_keep_all_links_in_parent_metadata() {
    let dir = TempDir::new().unwrap();
    let mut parent = Writer::open_at(dir.path(), "parent").await.unwrap();
    let (first, second) = tokio::join!(
        Writer::open_subagent_at(dir.path(), "parent", "first-worker", "first"),
        Writer::open_subagent_at(dir.path(), "parent", "second-worker", "second"),
    );
    first.unwrap().close().await.unwrap();
    second.unwrap().close().await.unwrap();
    parent
        .write_message(&sample_user_message(
            MessageId::new(),
            SessionId::new(),
            "continue after workers",
        ))
        .await
        .unwrap();

    let mut children = parent.meta().await.unwrap().unwrap().children_session_ids;
    children.sort_unstable();
    assert_eq!(children, ["first-worker", "second-worker"]);
}

#[tokio::test]
async fn metadata_edits_leave_corrupt_records_intact() {
    let dir = TempDir::new().unwrap();
    let writer = Writer::open_at(dir.path(), "corrupt").await.unwrap();
    let path = writer.path().with_file_name("meta.json");
    tokio::fs::write(&path, "{").await.unwrap();

    assert!(
        Meta::update(dir.path(), "corrupt", |meta| meta.pinned = true)
            .await
            .is_err()
    );
    assert_eq!(tokio::fs::read_to_string(&path).await.unwrap(), "{");
}

#[tokio::test]
async fn prompt_commit_keeps_receipt_and_message_together_after_a_torn_tail() {
    let dir = TempDir::new().unwrap();
    let id = "prompt-recovery";
    let mut writer = Writer::open_at(dir.path(), id).await.unwrap();
    let message = sample_user_message(MessageId::new(), SessionId::new(), "accepted input");
    tokio::fs::write(writer.path(), b"{\"id\":").await.unwrap();

    writer
        .write_prompt(&message, "recorded-request")
        .await
        .unwrap();

    assert_eq!(Reader::load_from(dir.path(), id).await.unwrap(), [message]);
    assert!(
        Reader::has_prompt_receipt(dir.path(), id, "recorded-request")
            .await
            .unwrap()
    );
    assert!(!Reader::has_prompt_receipt(dir.path(), id, "other-request")
        .await
        .unwrap());
}

#[tokio::test]
async fn cold_receipt_reads_wait_for_a_pending_append_to_commit_or_roll_back() {
    use std::io::Write;

    let dir = TempDir::new().unwrap();
    let id = "uncommitted";
    let writer = Writer::open_at(dir.path(), id).await.unwrap();
    let mut transcript = std::fs::OpenOptions::new()
        .read(true)
        .append(true)
        .open(writer.path())
        .unwrap();
    transcript.lock().unwrap();

    let message = sample_user_message(MessageId::new(), SessionId::new(), "pending");
    let mut record = serde_json::to_value(&message).unwrap();
    record["prompt_request_id"] = serde_json::json!("pending-request");
    writeln!(transcript, "{record}").unwrap();

    let root = dir.path().to_path_buf();
    let mut lookup =
        tokio::spawn(async move { Reader::has_prompt_receipt(&root, id, "pending-request").await });
    let in_flight = tokio::time::timeout(std::time::Duration::from_millis(200), &mut lookup).await;

    transcript.set_len(0).unwrap();
    transcript.unlock().unwrap();
    assert!(
        in_flight.is_err(),
        "receipt became observable before commit"
    );
    assert!(!lookup.await.unwrap().unwrap());
}
