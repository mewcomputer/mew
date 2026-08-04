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
