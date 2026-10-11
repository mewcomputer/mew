use mew_tui::{app::*, harness::Harness};
use std::time::{Duration, Instant};

pub fn run(gallery: &mut super::Gallery) {
    let mut h = Harness::new(120, 30);
    h.say("The task is running. Click a todo, agent, or background job to inspect it.");
    h.app.streaming = true;
    h.app.status.session_id = "capturetask".into();
    h.app
        .session_titles
        .insert("capturetask".into(), "Improve TUI task flow".into());
    h.app.todos = vec![
        mew_agent::Todo {
            id: 1,
            content: "Rework popovers and make current work inspectable".into(),
            status: mew_agent::TodoStatus::InProgress,
            depends_on: vec![],
        },
        mew_agent::Todo {
            id: 2,
            content: "Verify task switching".into(),
            status: mew_agent::TodoStatus::Blocked,
            depends_on: vec![1],
        },
    ];
    h.app.subagents=vec![SubagentState{task_id:"review".into(),child_session_id:"01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),name:"reviewer".into(),started_at:Instant::now()-Duration::from_secs(127),status:SubagentStatus::Running,last_progress:Some("Checking that sidebar clicks open the correct activity after scrolling and resizing.\nReviewing narrow popovers and completion transitions.".into()),display_name:Some("Curie".into()),finished_at:None}];
    h.app.background_jobs.push(BackgroundJobState {
        job_id: "tests".into(),
        command:
            "cargo test --workspace --all-features -- --nocapture activity_flow long-command-tail"
                .into(),
        started_at: Instant::now() - Duration::from_secs(42),
        status: BackgroundJobStatus::Running,
        finished_at: None,
    });
    gallery.capture(&mut h.app, "desktop-sidebar-clickable-120", 128, 40);
    h.app.open_subagent("review".into());
    gallery.capture(&mut h.app, "desktop-subagent-120", 128, 40);
    h.resize(40, 24);
    gallery.capture(&mut h.app, "desktop-subagent-40", 40, 24);
    h.resize(30, 14);
    gallery.capture(&mut h.app, "desktop-subagent-30", 30, 14);
    h.app.activity_view.as_mut().unwrap().scroll = usize::MAX;
    gallery.capture(&mut h.app, "desktop-subagent-30-end", 30, 14);
    h.app.close_activity();
    h.resize(120, 30);
    h.app.open_job("tests".into());
    gallery.capture(&mut h.app, "desktop-job-120", 128, 40);
    h.resize(40, 24);
    gallery.capture(&mut h.app, "desktop-job-40", 40, 24);
    h.app.close_activity();
    h.app.open_job_picker();
    gallery.capture(&mut h.app, "desktop-jobs-picker-40", 40, 24);
    h.app.close_picker();
    h.app.open_todos(Some(1));
    h.resize(120, 30);
    gallery.capture(&mut h.app, "desktop-todo-selected-120", 128, 40);
}
