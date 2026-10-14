//! Global hotkeys + keystroke buffer via evdev (passive, non-consuming).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use evdev::{Device, EventType, KeyCode};
use switchfix_app::ports::{HotkeyEvent, HotkeySource, TypingBufferPort};
use switchfix_domain::WordSpan;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Key codes for default hotkeys (Linux input event codes).
const KEY_F1: u16 = 59;
const KEY_F2: u16 = 60;
const KEY_PAUSE: u16 = 119;
const KEY_LEFTCTRL: u16 = 29;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_BACKSPACE: u16 = 14;
const KEY_SPACE: u16 = 57;

/// Shared keystroke buffer (RAM only).
#[derive(Debug)]
pub struct SharedTypingBuffer {
    chars: Mutex<VecDeque<char>>,
    max: usize,
    enabled: AtomicBool,
}

impl SharedTypingBuffer {
    pub fn new(max: usize, enabled: bool) -> Arc<Self> {
        Arc::new(Self {
            chars: Mutex::new(VecDeque::with_capacity(max.min(256))),
            max: max.max(1),
            enabled: AtomicBool::new(enabled),
        })
    }

    pub fn push(&self, c: char) {
        if !self.enabled.load(Ordering::SeqCst) {
            return;
        }
        let mut g = self.chars.lock().unwrap();
        if g.len() >= self.max {
            g.pop_front();
        }
        g.push_back(c);
    }

    pub fn pop(&self) {
        self.chars.lock().unwrap().pop_back();
    }

    pub fn clear(&self) {
        self.chars.lock().unwrap().clear();
    }

    fn is_sep(c: char) -> bool {
        c.is_whitespace() || matches!(c, ',' | '.' | ';' | ':' | '!' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\'' | '/' | '\\')
    }

    pub fn take_last_word(&self) -> Option<WordSpan> {
        if !self.enabled.load(Ordering::SeqCst) {
            return None;
        }
        let mut g = self.chars.lock().unwrap();

        // Trailing separators are ignored.
        while g.back().is_some_and(|c| Self::is_sep(*c)) {
            g.pop_back();
        }
        if g.is_empty() {
            return None;
        }

        let mut word = String::new();
        while let Some(c) = g.back() {
            if Self::is_sep(*c) {
                break;
            }
            word.insert(0, *c);
            g.pop_back();
        }

        if word.is_empty() {
            return None;
        }

        let char_len = word.chars().count();
        g.clear();
        Some(WordSpan { text: word, char_len })
    }
}

impl TypingBufferPort for SharedTypingBuffer {
    fn take_last_word(&self) -> Option<WordSpan> {
        SharedTypingBuffer::take_last_word(self)
    }
    fn clear(&self) {
        SharedTypingBuffer::clear(self);
    }
    fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }
    fn push_char(&self, c: char) {
        self.push(c);
    }
    fn backspace(&self) {
        self.pop();
    }
}

/// Minimal US-QWERTY key → char map for buffer tracking under active layout.
/// We track the *intended* character after our own conversion; for raw typing
/// we store the US char when layout is unknown. The conversion layer maps later.
fn key_to_char(code: u16, shift: bool) -> Option<char> {
    let c = match code {
        16 => 'q', 17 => 'w', 18 => 'e', 19 => 'r', 20 => 't',
        21 => 'y', 22 => 'u', 23 => 'i', 24 => 'o', 25 => 'p',
        30 => 'a', 31 => 's', 32 => 'd', 33 => 'f', 34 => 'g',
        35 => 'h', 36 => 'j', 37 => 'k', 38 => 'l',
        44 => 'z', 45 => 'x', 46 => 'c', 47 => 'v', 48 => 'b',
        49 => 'n', 50 => 'm',
        2 => '1', 3 => '2', 4 => '3', 5 => '4', 6 => '5',
        7 => '6', 8 => '7', 9 => '8', 10 => '9', 11 => '0',
        KEY_SPACE => ' ',
        51 => ',', 52 => '.', 53 => '/',
        12 => '-', 13 => '=',
        26 => '[', 27 => ']',
        39 => ';', 40 => '\'',
        41 => '`', 43 => '\\',
        _ => return None,
    };
    if shift && c.is_ascii_alphabetic() {
        Some(c.to_ascii_uppercase())
    } else {
        Some(c)
    }
}

/// Passive evdev listener: global hotkeys + typing buffer.
pub struct EvdevHotkeySource {
    rx: Mutex<Option<mpsc::Receiver<HotkeyEvent>>>,
}

impl EvdevHotkeySource {
    /// Open all keyboard-like event devices and spawn a reader thread.
    pub fn open(buffer: Arc<SharedTypingBuffer>) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel(32);
        let mut devices = Vec::new();

        for entry in std::fs::read_dir("/dev/input")? {
            let Ok(entry) = entry else { continue };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("event") {
                continue;
            }
            let path = entry.path();
            match Device::open(&path) {
                Ok(dev) => {
                    // Only keyboards: has KEY_A capability.
                    if dev.supported_keys().is_some_and(|k| k.contains(KeyCode::KEY_A)) {
                        debug!("evdev keyboard: {:?} ({:?})", path, dev.name());
                        devices.push(dev);
                    }
                }
                Err(e) => {
                    debug!("skip {:?}: {e}", path);
                }
            }
        }

        if devices.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no keyboard devices (need group 'input')",
            ));
        }

        info!("evdev: tracking {} keyboard(s)", devices.len());

        std::thread::spawn(move || {
            run_loop(devices, buffer, tx);
        });

        Ok(Self {
            rx: Mutex::new(Some(rx)),
        })
    }
}

fn run_loop(
    mut devices: Vec<Device>,
    buffer: Arc<SharedTypingBuffer>,
    tx: mpsc::Sender<HotkeyEvent>,
) {
    let mut ctrl = false;
    let mut shift = false;

    loop {
        for dev in devices.iter_mut() {
            let events = match dev.fetch_events() {
                Ok(e) => e,
                Err(e) => {
                    // ENODEV = device unplugged
                    warn!("evdev read error: {e}");
                    continue;
                }
            };

            for ev in events {
                if ev.event_type() != EventType::KEY {
                    continue;
                }
                let code = ev.code();
                let value = ev.value(); // 0=up 1=down 2=repeat

                // Track modifiers.
                if code == KEY_LEFTCTRL {
                    ctrl = value != 0;
                    continue;
                }
                if code == KEY_LEFTSHIFT {
                    shift = value != 0;
                    continue;
                }

                if value == 1 {
                    // Hotkeys (on key-down).
                    if ctrl && !shift && code == KEY_F1 {
                        let _ = tx.try_send(HotkeyEvent::ConvertSelection);
                        continue;
                    }
                    if ctrl && !shift && code == KEY_F2 {
                        let _ = tx.try_send(HotkeyEvent::ConvertLastWord);
                        continue;
                    }
                    if ctrl && shift && code == KEY_F1 {
                        let _ = tx.try_send(HotkeyEvent::ConvertTerminal);
                        continue;
                    }
                    if ctrl && code == KEY_PAUSE {
                        let _ = tx.try_send(HotkeyEvent::ToggleEnabled);
                        continue;
                    }

                    // Typing buffer.
                    if code == KEY_BACKSPACE {
                        buffer.pop();
                        continue;
                    }
                    if let Some(c) = key_to_char(code, shift) {
                        buffer.push(c);
                    }
                }
            }
        }

        // Small sleep to avoid busy-loop on fetch_events (it already blocks per-device
        // but we cycle devices; use a tiny timeout via sleep when all empty).
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

impl HotkeySource for EvdevHotkeySource {
    fn events(&self) -> mpsc::Receiver<HotkeyEvent> {
        let mut g = self.rx.lock().unwrap();
        g.take().unwrap_or_else(|| {
            let (_tx, rx) = mpsc::channel(1);
            rx
        })
    }
}

/// In-memory hotkey source for tests / fallback when evdev is unavailable.
pub struct ManualHotkeySource {
    tx: mpsc::Sender<HotkeyEvent>,
    rx: Mutex<Option<mpsc::Receiver<HotkeyEvent>>>,
}

impl ManualHotkeySource {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(16);
        Self {
            tx,
            rx: Mutex::new(Some(rx)),
        }
    }

    pub fn send(&self, ev: HotkeyEvent) {
        let _ = self.tx.try_send(ev);
    }
}

impl Default for ManualHotkeySource {
    fn default() -> Self {
        Self::new()
    }
}

impl HotkeySource for ManualHotkeySource {
    fn events(&self) -> mpsc::Receiver<HotkeyEvent> {
        self.rx.lock().unwrap().take().unwrap_or_else(|| {
            let (_tx, rx) = mpsc::channel(1);
            rx
        })
    }
}

