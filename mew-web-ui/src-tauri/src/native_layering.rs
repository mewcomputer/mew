//! Keep the embedded CEF browser composited above the Tauri WKWebView.
//!
//! The layering inversion this module originally implemented (CEF below a
//! transparent WKWebView with a transparent React "hole") is not viable on
//! this WebKit build: the WebContent process composites an opaque background
//! layer into the webview's remote layer tree, and no AppKit-level
//! transparency on the view punches through it. Verified by dumping the
//! hierarchy — CEF reorders below the webview correctly, yet stays invisible
//! until the webview is moved below it.
//!
//! So CEF stays on top, and HTML overlays that must cover the browser are
//! handled by a separate native overlay surface (not by transparency). What
//! remains here is the steady-state assertion that CEF is the content view's
//! topmost subview, since CEF adds new views on top of anything already
//! there and nothing else in the app reorders it back.

use std::sync::atomic::{AtomicUsize, Ordering};

use objc2_app_kit::{NSView, NSWindowOrderingMode};
use tauri::Manager;

/// Remembers which CEF native view has been asserted as topmost. CEF can
/// recreate its view on close/reopen, so "already handled" is keyed by the
/// view handle rather than a one-shot flag: an unchanged handle means the
/// current ordering is still in effect, a new handle needs another pass.
#[derive(Default)]
pub struct NativeLayeringGuard {
    last_ordered_handle: AtomicUsize,
}

impl NativeLayeringGuard {
    /// Returns the handle that still needs a layering pass, or `None` when
    /// the current handle was already handled (or no view exists yet).
    pub fn needs_ordering(&self, current_handle: usize) -> Option<usize> {
        if current_handle == 0 {
            return None;
        }
        if self.last_ordered_handle.load(Ordering::Acquire) == current_handle {
            return None;
        }
        Some(current_handle)
    }

    /// Marks `handle` as handled. Called after the pass is scheduled;
    /// scheduling is main-thread only, so a stale mark is harmless (the next
    /// `needs_ordering` call with a new handle runs another pass).
    pub fn mark_ordered(&self, handle: usize) {
        self.last_ordered_handle.store(handle, Ordering::Release);
    }
}

/// Asserts the CEF view is the content view's topmost subview, above the
/// WKWebView. Idempotent and cheap; the CEF claim path runs it on every
/// visible bounds update. No-op when the CEF view isn't a direct child of
/// the Tauri content view (a mismatch is logged, never fatal).
pub fn ensure_cef_on_top(app: &tauri::AppHandle, cef_view_handle: usize) {
    if cef_view_handle == 0 {
        return;
    }
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let content_view = match window.ns_view() {
        Ok(view) => view as usize,
        Err(error) => {
            tracing::warn!(%error, "failed to get the Tauri content view for CEF ordering");
            return;
        }
    };
    let result = window.with_webview(move |webview| {
        let content_view = unsafe { &*(content_view as *const NSView) };
        let wk_webview = unsafe { &*(webview.inner() as *const NSView) };
        let cef_view = unsafe { &*(cef_view_handle as *const NSView) };
        let Some(cef_superview) = (unsafe { cef_view.superview() }) else {
            return;
        };
        if !std::ptr::eq(cef_superview.as_ref(), content_view) {
            tracing::warn!("CEF view is not a direct child of the content view; skipping ordering");
            return;
        }
        content_view.addSubview_positioned_relativeTo(
            cef_view,
            NSWindowOrderingMode::Above,
            Some(wk_webview),
        );
    });
    if let Err(error) = result {
        tracing::warn!(%error, "failed to order the CEF view above the WKWebView");
    }
}

#[cfg(test)]
mod tests {
    use super::NativeLayeringGuard;

    #[test]
    fn an_unordered_handle_needs_ordering_exactly_once() {
        let guard = NativeLayeringGuard::default();

        assert_eq!(guard.needs_ordering(42), Some(42));
        guard.mark_ordered(42);
        assert_eq!(guard.needs_ordering(42), None);
    }

    #[test]
    fn a_recreated_cef_view_is_ordered_again() {
        let guard = NativeLayeringGuard::default();
        guard.mark_ordered(42);

        assert_eq!(guard.needs_ordering(77), Some(77));
        guard.mark_ordered(77);
        assert_eq!(guard.needs_ordering(77), None);
    }

    #[test]
    fn a_missing_native_view_never_needs_ordering() {
        let guard = NativeLayeringGuard::default();

        assert_eq!(guard.needs_ordering(0), None);
    }
}
