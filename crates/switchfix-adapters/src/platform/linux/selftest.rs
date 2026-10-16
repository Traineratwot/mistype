//! Live self-test: inject a synthetic hotkey via uinput and verify evdev sees it.
//!
//! Proves the interception path works without the user pressing physical keys.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use evdev::uinput::{VirtualDevice, VirtualDeviceBuilder};
use evdev::{AttributeSet, EventType, InputEvent, KeyCode};
use switchfix_app::hotkey::HotkeyBinding;
use switchfix_app::ports::{HotkeyEvent, HotkeySource};
use tracing::info;

use super::evdev_hotkey::{EvdevHotkeySource, EvdevOptions};
use crate::platform::portable::typing_buffer::SharedTypingBuffer;

const TEST_KB_NAME: &str = "switchfix-selftest-kb";
/// Unused by desktops — safe to inject.
const TEST_COMBO: &str = "ctrl+f20";

/// Result of one self-test step.
#[derive(Debug)]
pub struct SelfTestReport {
    pub steps: Vec<(String, bool, String)>,
}

impl SelfTestReport {
    pub fn all_ok(&self) -> bool {
        self.steps.iter().all(|(_, ok, _)| *ok)
    }

    pub fn print(&self) {
        for (name, ok, detail) in &self.steps {
            let mark = if *ok { "OK " } else { "FAIL" };
            println!("[{mark}] {name}: {detail}");
        }
    }
}

/// Inject `ctrl+f20` through a private uinput keyboard and wait for the
/// matching hotkey event from [`EvdevHotkeySource`].
pub async fn run_hotkey_selftest() -> SelfTestReport {
    let mut steps = Vec::new();

    // 1. Can we list keyboard devices at all?
    let can_list = std::fs::read_dir("/dev/input").is_ok();
    steps.push((
        "list /dev/input".into(),
        can_list,
        if can_list {
            "readable".into()
        } else {
            "cannot read /dev/input".into()
        },
    ));

    // 2. Open at least one real event node (needs group `input` on the session).
    let mut opened = 0usize;
    if let Ok(rd) = std::fs::read_dir("/dev/input") {
        for e in rd.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("event") {
                continue;
            }
            if evdev::Device::open(e.path()).is_ok() {
                opened += 1;
            }
        }
    }
    steps.push((
        "open /dev/input/event*".into(),
        opened > 0,
        format!("{opened} device(s) opened"),
    ));

    if opened == 0 {
        steps.push((
            "hotkey intercept".into(),
            false,
            "skipped — cannot open keyboards (need group 'input' in THIS session; \
             log out/in or: sg input -c switchfix selftest)"
                .into(),
        ));
        return SelfTestReport { steps };
    }

    // 3. Private virtual keyboard. Create it FIRST so the evdev scan sees it.
    let inject = match open_test_keyboard() {
        Ok(d) => d,
        Err(e) => {
            steps.push((
                "uinput inject device".into(),
                false,
                format!("cannot create virtual keyboard: {e}"),
            ));
            return SelfTestReport { steps };
        }
    };
    // Give udev / evdev a moment to register the new event node.
    tokio::time::sleep(Duration::from_millis(200)).await;
    steps.push(("uinput inject device".into(), true, "created".into()));

    // 4. Evdev source bound to ctrl+f20.
    let buffer = SharedTypingBuffer::new(64, true);
    let source = match EvdevHotkeySource::open_with(EvdevOptions {
        bindings: vec![(
            HotkeyBinding::parse(TEST_COMBO).expect("valid"),
            HotkeyEvent::ConvertSelection,
        )],
        dynamic_bindings: Vec::new(),
        bar_visible: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        // Passive: do not grab keyboards (self-test must not break typing).
        consume: false,
        buffer,
        mapper: Arc::new(crate::platform::portable::LayoutCharMapper::us_only()),
    }) {
        Ok(s) => s,
        Err(e) => {
            steps.push((
                "evdev hotkey source".into(),
                false,
                format!("open failed: {e}"),
            ));
            return SelfTestReport { steps };
        }
    };
    steps.push(("evdev hotkey source".into(), true, "opened".into()));

    // Does the source track our virtual keyboard? (Best-effort diagnostic.)
    let test_kb_seen = find_event_path(TEST_KB_NAME).is_some();
    steps.push((
        "selftest keyboard visible".into(),
        test_kb_seen,
        if test_kb_seen {
            format!("found event node for '{TEST_KB_NAME}'")
        } else {
            format!("no event node named '{TEST_KB_NAME}' after 200ms")
        },
    ));

    let mut rx = source.events();

    // Give the evdev worker thread time to start its poll loop.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 5. Inject ctrl+f20 down/up.
    inject.tap(KeyCode::KEY_LEFTCTRL, true);
    inject.tap(KeyCode::KEY_F20, true);
    inject.tap(KeyCode::KEY_F20, false);
    inject.tap(KeyCode::KEY_LEFTCTRL, false);
    info!("selftest: injected {TEST_COMBO}");

    // 6. Wait for the event (evdev poll loop is 3ms).
    let got = tokio::time::timeout(Duration::from_secs(2), rx.recv()).await;
    let (ok, detail) = match got {
        Ok(Some(HotkeyEvent::ConvertSelection)) => (true, "received ConvertSelection".into()),
        Ok(Some(other)) => (false, format!("unexpected event: {other:?}")),
        Ok(None) => (false, "channel closed".into()),
        Err(_) => {
            // Fallback: read the test device directly to tell "inject broken"
            // from "source missed our device".
            let direct = direct_read_match(TEST_KB_NAME, Duration::from_millis(500));
            (
                false,
                format!(
                    "timeout after 2s — evdev source did not see the injected key \
                     (direct read on test kb: {direct})"
                ),
            )
        }
    };
    steps.push(("hotkey intercept".into(), ok, detail));

    SelfTestReport { steps }
}

/// Open the named uinput keyboard's event node and try to match a pre-injected
/// key sequence. Used only as a diagnostic when the main source times out.
fn direct_read_match(name: &str, wait: Duration) -> String {
    let Some(path) = find_event_path(name) else {
        return "no event node".into();
    };
    match evdev::Device::open(&path) {
        Ok(mut dev) => {
            let start = std::time::Instant::now();
            let mut ctrl = false;
            let mut saw_f20 = false;
            while start.elapsed() < wait {
                match dev.fetch_events() {
                    Ok(evs) => {
                        for ev in evs {
                            if ev.event_type() != EventType::KEY {
                                continue;
                            }
                            let code = ev.code();
                            let val = ev.value();
                            if code == KeyCode::KEY_LEFTCTRL.0 {
                                ctrl = val != 0;
                            }
                            if code == KeyCode::KEY_F20.0 && val == 1 && ctrl {
                                saw_f20 = true;
                            }
                        }
                        if saw_f20 {
                            return "matched ctrl+f20".into();
                        }
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(5)),
                }
            }
            if saw_f20 {
                "matched ctrl+f20".into()
            } else {
                format!("opened {path:?} but never saw ctrl+f20")
            }
        }
        Err(e) => format!("open {path:?} failed: {e}"),
    }
}

/// Find `/dev/input/event*` whose device name matches (via /proc/bus/input/devices).
fn find_event_path(name: &str) -> Option<std::path::PathBuf> {
    let proc = std::fs::read_to_string("/proc/bus/input/devices").ok()?;
    let mut current_name = String::new();
    for line in proc.lines() {
        if let Some(n) = line.strip_prefix("N: Name=\"") {
            current_name = n.trim_end_matches('"').to_string();
        }
        if line.starts_with("H: Handlers=") && current_name == name {
            for tok in line.split_whitespace() {
                if let Some(ev) = tok.strip_prefix("event") {
                    if ev.chars().all(|c| c.is_ascii_digit()) {
                        return Some(std::path::PathBuf::from(format!("/dev/input/event{ev}")));
                    }
                }
            }
        }
    }
    None
}

fn open_test_keyboard() -> std::io::Result<TestKeyboard> {
    let mut keys = AttributeSet::<KeyCode>::new();
    for i in 0..=255u16 {
        keys.insert(KeyCode(i));
    }
    #[allow(deprecated)]
    let device = VirtualDeviceBuilder::new()
        .map_err(|e| std::io::Error::other(e.to_string()))?
        .name(TEST_KB_NAME)
        .with_keys(&keys)
        .map_err(|e| std::io::Error::other(e.to_string()))?
        .build()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(TestKeyboard {
        device: Mutex::new(device),
    })
}

struct TestKeyboard {
    device: Mutex<VirtualDevice>,
}

impl TestKeyboard {
    fn tap(&self, key: KeyCode, down: bool) {
        let value = if down { 1 } else { 0 };
        let events = [
            InputEvent::new(EventType::KEY.0, key.0, value),
            InputEvent::new(EventType::SYNCHRONIZATION.0, 0, 0),
        ];
        if let Ok(mut dev) = self.device.lock() {
            let _ = dev.emit(&events);
        }
    }
}
