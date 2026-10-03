use super::*;

fn page(profile: &str, session: &str, generation: &str) -> BrowserPage {
    BrowserPage {
        owner: BrowserOwner {
            connection_profile: profile.into(),
            session_id: session.into(),
        },
        page_instance: generation.into(),
    }
}

#[test]
fn late_callbacks_keep_the_page_that_created_them() {
    let mut state = BrowserState::default();
    let first = page("local", "a", "first");
    let second = page("local", "b", "second");
    state.insert_page(first.clone());
    state.insert_page(second.clone());
    state.set_visible(&second.owner, true);
    enqueue_event(
        &mut state,
        &first,
        CefBrowserEvent::AddressChanged {
            url: "https://a.example/late".into(),
        },
    );
    assert!(
        matches!(state.events.front(), Some(BrowserEvent::AddressChanged { page, .. }) if page == &first)
    );
    assert_eq!(state.pages[&second.owner].url, "about:blank");
}

#[test]
fn hiding_a_retained_page_keeps_its_callbacks_routable() {
    let mut state = BrowserState::default();
    let first = page("local", "a", "first");
    state.insert_page(first.clone());
    state.set_visible(&first.owner, true);
    state.set_visible(&first.owner, false);
    enqueue_event(
        &mut state,
        &first,
        CefBrowserEvent::TitleChanged {
            title: "finished while hidden".into(),
            url: "https://a.example".into(),
        },
    );
    assert!(
        matches!(state.events.front(), Some(BrowserEvent::TitleChanged { page, .. }) if page == &first)
    );
    assert!(!state.pages[&first.owner].visible);
    assert_eq!(state.pages.len(), 1);
}

#[test]
fn recreated_pages_reject_old_readiness_and_old_removal() {
    let mut state = BrowserState::default();
    let first = page("local", "a", "first");
    let replacement = page("local", "a", "replacement");
    state.insert_page(first.clone());
    assert!(state.remove_page(&first));
    state.insert_page(replacement.clone());
    enqueue_event(
        &mut state,
        &first,
        CefBrowserEvent::Ready {
            cdp_endpoint: "http://127.0.0.1:9223".into(),
            target_id: "old-target".into(),
        },
    );
    assert!(!state.remove_page(&first));
    assert!(state.pages[&replacement.owner].ready.is_none());
    assert!(state.events.is_empty());
    enqueue_event(
        &mut state,
        &replacement,
        CefBrowserEvent::Ready {
            cdp_endpoint: "http://127.0.0.1:9223".into(),
            target_id: "real-target".into(),
        },
    );
    assert_eq!(
        state.pages[&replacement.owner]
            .ready
            .as_ref()
            .unwrap()
            .target_id,
        "real-target"
    );
}

#[test]
fn pages_with_the_same_session_on_different_profiles_are_independent() {
    let mut state = BrowserState::default();
    let first = page("local-one", "a", "first");
    let second = page("local-two", "a", "second");
    state.insert_page(first.clone());
    state.insert_page(second.clone());
    state.set_visible(&first.owner, true);
    enqueue_event(
        &mut state,
        &first,
        CefBrowserEvent::AddressChanged {
            url: "https://retained.example/state".into(),
        },
    );
    state.set_visible(&second.owner, true);
    state.set_visible(&first.owner, true);
    assert_eq!(
        state.pages[&first.owner].url,
        "https://retained.example/state"
    );
    assert_eq!(state.pages[&second.owner].url, "about:blank");
    assert!(!state.pages[&second.owner].visible);
}

#[test]
fn closing_a_generation_discards_its_queued_events_and_late_callbacks() {
    let mut state = BrowserState::default();
    let first = page("local", "a", "first");
    let second = page("local", "b", "second");
    state.insert_page(first.clone());
    state.insert_page(second.clone());
    for page in [&first, &second] {
        enqueue_event(
            &mut state,
            page,
            CefBrowserEvent::AddressChanged {
                url: "https://example.com".into(),
            },
        );
    }
    assert!(state.remove_page(&first));
    enqueue_event(
        &mut state,
        &first,
        CefBrowserEvent::Error {
            message: "late failure".into(),
        },
    );
    assert_eq!(state.events.len(), 1);
    assert!(
        matches!(state.events.front(), Some(BrowserEvent::AddressChanged { page, .. }) if page == &second)
    );
}

#[test]
fn native_close_revokes_ready_state_and_cannot_close_a_replacement() {
    let mut state = BrowserState::default();
    let first = page("local", "a", "first");
    let replacement = page("local", "a", "replacement");
    state.insert_page(first.clone());
    enqueue_event(
        &mut state,
        &first,
        CefBrowserEvent::Ready {
            cdp_endpoint: "http://127.0.0.1:9223".into(),
            target_id: "first-target".into(),
        },
    );
    enqueue_event(&mut state, &first, CefBrowserEvent::Closed);
    assert!(state.pages[&first.owner].closed);
    assert!(state.pages[&first.owner].ready.is_none());
    assert!(matches!(state.events.front(), Some(BrowserEvent::Closed { page }) if page == &first));
    assert!(state.remove_page(&first));
    state.insert_page(replacement.clone());
    enqueue_event(&mut state, &first, CefBrowserEvent::Closed);
    assert!(!state.pages[&replacement.owner].closed);
}

#[test]
fn registration_ack_cannot_clear_a_closed_or_replaced_page_error() {
    let mut state = BrowserState::default();
    let first = page("local", "a", "first");
    let replacement = page("local", "a", "replacement");
    state.insert_page(first.clone());
    assert!(state.set_error(&first, Some("incompatible CLI".into())));
    assert_eq!(
        state.pages[&first.owner].error.as_deref(),
        Some("incompatible CLI")
    );
    enqueue_event(&mut state, &first, CefBrowserEvent::Closed);
    assert!(!state.set_error(&first, None));
    assert!(state.pages[&first.owner].error.is_some());
    state.remove_page(&first);
    state.insert_page(replacement.clone());
    assert!(!state.set_error(&first, Some("late error".into())));
    assert!(state.pages[&replacement.owner].error.is_none());
}

#[cfg(not(target_os = "macos"))]
#[test]
fn unsupported_platform_does_not_advertise_a_native_browser() {
    assert!(BrowserPortal::initialize().unwrap().is_none());
}

#[test]
fn unchanged_visibility_produces_no_native_visibility_changes() {
    let mut state = BrowserState::default();
    let first = page("local", "a", "first");
    let second = page("local", "b", "second");
    state.insert_page(first.clone());
    state.insert_page(second.clone());
    assert!(state
        .visibility_changes(Some(&first.owner))
        .eq([(&first.owner, true)]));
    state.set_visible(&first.owner, true);
    assert!(state
        .visibility_changes(Some(&first.owner))
        .next()
        .is_none());
    assert_eq!(
        state
            .visibility_changes(Some(&second.owner))
            .collect::<Vec<_>>(),
        vec![(&first.owner, false), (&second.owner, true)]
    );
    state.set_visible(&second.owner, true);
    assert!(state.visibility_changes(None).eq([(&second.owner, false)]));
}

#[test]
fn native_closed_pages_stay_hidden_when_visibility_is_requested() {
    let mut state = BrowserState::default();
    let first = page("local", "a", "first");
    state.insert_page(first.clone());
    enqueue_event(&mut state, &first, CefBrowserEvent::Closed);
    state.set_visible(&first.owner, true);
    assert!(!state.pages[&first.owner].visible);
    assert!(state
        .visibility_changes(Some(&first.owner))
        .next()
        .is_none());
}
