//! PLAN.md §5 — integration scenarios with fakes (order-sensitive).

#[path = "fakes/mod.rs"]
mod fakes;

use fakes::*;
use switchfix_app::ports::HotkeyEvent;
use switchfix_domain::{CorrectionOutcome, CorrectionRequest};

// Re-export so tests can use HotkeyEvent etc. without extra imports.
#[allow(unused_imports)]
use switchfix_app as _app;

/// Convert selection happy path (PLAN.md §5):
/// capture → convert → put → chord(Ctrl+V) → restore → set(layout)
#[tokio::test]
async fn convert_selection_happy_order() {
    let h = harness(HarnessOptions {
        selected: Some("ghbdtn".into()),
        clipboard_text: Some("user-clip".into()),
        ..Default::default()
    });

    let out = h.service.handle(CorrectionRequest::ConvertSelection).await;
    assert!(out.is_success(), "{out:?}");
    match &out {
        CorrectionOutcome::Converted {
            original, converted, ..
        } => {
            assert_eq!(original, "ghbdtn");
            assert_eq!(converted, "привет");
        }
        other => panic!("unexpected {other:?}"),
    }

    // Cross-component order (PLAN.md §5 happy path).
    h.events.assert_order("capture_selection", "put:привет");
    h.events.assert_order("put:привет", "chord:CtrlV");
    h.events.assert_order("chord:CtrlV", "restore");
    h.events.assert_order("restore", "set_layout:1");

    // Clipboard is back to the original snapshot.
    assert_eq!(
        h.clipboard.text.lock().unwrap().as_deref(),
        Some("user-clip"),
        "clipboard not restored"
    );
}

/// No selection → fallback to last word; clipboard untouched by the word path.
#[tokio::test]
async fn no_selection_falls_back_to_word() {
    let h = harness(HarnessOptions {
        selected: None,
        word: Some("ghbdtn".into()),
        ..Default::default()
    });

    let out = h.service.handle(CorrectionRequest::ConvertSelection).await;
    assert!(out.is_success(), "{out:?}");
    // Word path: backspace + type, no put/restore.
    h.injector.log.assert_order("backspace:6", "type:привет@ru");
    assert!(!h.clipboard.log.contains("put:"), "clipboard must stay clean");
    assert!(!h.clipboard.log.contains("restore"), "no restore needed");
}

/// No selection and no word → NoInputFound, clipboard untouched.
#[tokio::test]
async fn no_selection_no_word_is_no_input() {
    let h = harness(HarnessOptions::default());
    let out = h.service.handle(CorrectionRequest::ConvertSelection).await;
    assert_eq!(out, CorrectionOutcome::NoInputFound);
    assert!(!h.clipboard.log.contains("put:"));
}

/// Convert last word: backspace×N → type_text → set(layout); clipboard not touched.
#[tokio::test]
async fn convert_last_word_order() {
    let h = harness(HarnessOptions {
        word: Some("руддщ".into()),
        clipboard_text: Some("keep-me".into()),
        ..Default::default()
    });

    let out = h.service.handle(CorrectionRequest::ConvertLastWord).await;
    assert!(out.is_success(), "{out:?}");
    h.events.assert_order("take_last_word", "backspace:5");
    h.events.assert_order("backspace:5", "type:hello@us");
    h.events.assert_order("type:hello@us", "set_layout:0");
    assert!(!h.clipboard.log.contains("save"), "clipboard must not be touched");
    assert!(!h.clipboard.log.contains("put:"), "clipboard must not be touched");
    assert_eq!(
        h.clipboard.text.lock().unwrap().as_deref(),
        Some("keep-me")
    );
}

/// Terminal mode: chord(Ctrl+A), (Ctrl+K), (Ctrl+Shift+V).
#[tokio::test]
async fn terminal_chords_order() {
    let h = harness(HarnessOptions {
        word: Some("ghbdtn".into()),
        ..Default::default()
    });

    let out = h.service.handle(CorrectionRequest::ConvertTerminal).await;
    assert!(out.is_success(), "{out:?}");
    h.injector.log.assert_order("chord:CtrlA", "chord:CtrlK");
    h.injector.log.assert_order("chord:CtrlK", "chord:CtrlShiftV");
    // Result was put into the clipboard before paste.
    h.events.assert_order("put:привет", "chord:CtrlShiftV");
}

/// Layout switch fail: text is NOT rolled back; warning is emitted.
#[tokio::test]
async fn layout_fail_keeps_text() {
    let h = harness(HarnessOptions {
        word: Some("ghbdtn".into()),
        layout_fail: true,
        ..Default::default()
    });

    let out = h.service.handle(CorrectionRequest::ConvertLastWord).await;
    assert!(out.is_success(), "conversion must succeed even if layout fails: {out:?}");
    // Text was retyped despite layout failure.
    assert!(h.injector.log.contains("type:привет@ru"));
    // Warning was sent.
    assert!(
        h.notifier.log.contains("раскладка"),
        "expected layout warning, log={:?}",
        h.notifier.log.all()
    );
}

/// Clipboard restore fail: warning; core conversion is not rolled back.
#[tokio::test]
async fn clipboard_restore_fail_keeps_text() {
    let h = harness(HarnessOptions {
        selected: Some("ghbdtn".into()),
        clipboard_restore_fail: true,
        ..Default::default()
    });

    let out = h.service.handle(CorrectionRequest::ConvertSelection).await;
    assert!(out.is_success(), "{out:?}");
    assert!(h.injector.log.contains("chord:CtrlV"), "paste must have happened");
    assert!(h.clipboard.log.contains("restore"), "restore was attempted");
    assert!(
        h.notifier.log.contains("буфер"),
        "expected clipboard warning, log={:?}",
        h.notifier.log.all()
    );
}

/// Toggle disabled: injector is not called.
#[tokio::test]
async fn toggle_disabled_skips_injector() {
    let h = harness(HarnessOptions {
        word: Some("ghbdtn".into()),
        enabled: false,
        ..Default::default()
    });

    let out = h.service.handle(CorrectionRequest::ConvertLastWord).await;
    assert_eq!(out, CorrectionOutcome::Disabled);
    assert!(h.injector.log.all().is_empty(), "injector must stay silent");
    assert!(h.buffer.word.lock().unwrap().is_some(), "buffer not cleared");
}

/// Toggle clears the typing buffer (privacy).
#[tokio::test]
async fn toggle_clears_buffer() {
    let h = harness(HarnessOptions {
        word: Some("secret".into()),
        ..Default::default()
    });
    assert!(h.service.is_enabled());
    let now = h.service.toggle_enabled().await;
    assert!(!now);
    assert!(h.buffer.word.lock().unwrap().is_none(), "buffer must be cleared");
}

/// Unchanged text (digits only) → NoInputFound and clipboard restored.
#[tokio::test]
async fn unchanged_selection_restores_clipboard() {
    let h = harness(HarnessOptions {
        selected: Some("123".into()),
        clipboard_text: Some("orig".into()),
        ..Default::default()
    });

    let out = h.service.handle(CorrectionRequest::ConvertSelection).await;
    assert_eq!(out, CorrectionOutcome::NoInputFound);
    assert!(h.clipboard.log.contains("restore"));
    assert!(!h.injector.log.contains("chord:CtrlV"), "no paste");
    assert_eq!(h.clipboard.text.lock().unwrap().as_deref(), Some("orig"));
}

/// restore_clipboard=false leaves the converted text in the clipboard.
#[tokio::test]
async fn restore_disabled_keeps_converted() {
    let h = harness(HarnessOptions {
        selected: Some("ghbdtn".into()),
        clipboard_text: Some("orig".into()),
        restore_clipboard: false,
        ..Default::default()
    });

    let out = h.service.handle(CorrectionRequest::ConvertSelection).await;
    assert!(out.is_success());
    assert!(!h.clipboard.log.contains("restore"));
    assert_eq!(
        h.clipboard.text.lock().unwrap().as_deref(),
        Some("привет"),
        "converted text stays in clipboard when restore is off"
    );
}

// Silence unused import warning for HotkeyEvent (kept for future hotkey-order tests).
#[allow(dead_code)]
fn _touch(_: HotkeyEvent) {}
