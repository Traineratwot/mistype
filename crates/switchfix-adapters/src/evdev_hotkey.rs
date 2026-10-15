//! Global hotkeys + keystroke buffer via evdev.
//!
//! Modes:
//! - **passive** (default when `hotkey_consume = false`): listen only; the OS
//!   still delivers the hotkey to the focused app.
//! - **consume** (`hotkey_consume = true`): EVIOCGRAB the keyboards so the hotkey
//!   never reaches other apps. Non-hotkey keys are re-injected through uinput so
//!   normal typing keeps working.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use evdev::{Device, EventType, InputEvent, KeyCode};
use switchfix_app::hotkey::HotkeyBinding;
use switchfix_app::ports::{HotkeyEvent, HotkeySource, TypingBufferPort};
use switchfix_domain::WordSpan;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

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
        c.is_whitespace()
            || matches!(
                c,
                ',' | '.' | ';'
                    | ':'
                    | '!'
                    | '?'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '"'
                    | '\''
                    | '/'
                    | '\\'
            )
    }

    pub fn take_last_word(&self) -> Option<WordSpan> {
        if !self.enabled.load(Ordering::SeqCst) {
            return None;
        }
        let mut g = self.chars.lock().unwrap();

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

// ----- key name ↔ evdev KeyCode -----

/// Config key name → evdev KeyCode (Linux input event codes).
pub fn key_name_to_code(name: &str) -> Option<KeyCode> {
    let code = match name {
        "a" => KeyCode::KEY_A,
        "b" => KeyCode::KEY_B,
        "c" => KeyCode::KEY_C,
        "d" => KeyCode::KEY_D,
        "e" => KeyCode::KEY_E,
        "f" => KeyCode::KEY_F,
        "g" => KeyCode::KEY_G,
        "h" => KeyCode::KEY_H,
        "i" => KeyCode::KEY_I,
        "j" => KeyCode::KEY_J,
        "k" => KeyCode::KEY_K,
        "l" => KeyCode::KEY_L,
        "m" => KeyCode::KEY_M,
        "n" => KeyCode::KEY_N,
        "o" => KeyCode::KEY_O,
        "p" => KeyCode::KEY_P,
        "q" => KeyCode::KEY_Q,
        "r" => KeyCode::KEY_R,
        "s" => KeyCode::KEY_S,
        "t" => KeyCode::KEY_T,
        "u" => KeyCode::KEY_U,
        "v" => KeyCode::KEY_V,
        "w" => KeyCode::KEY_W,
        "x" => KeyCode::KEY_X,
        "y" => KeyCode::KEY_Y,
        "z" => KeyCode::KEY_Z,
        "0" => KeyCode::KEY_0,
        "1" => KeyCode::KEY_1,
        "2" => KeyCode::KEY_2,
        "3" => KeyCode::KEY_3,
        "4" => KeyCode::KEY_4,
        "5" => KeyCode::KEY_5,
        "6" => KeyCode::KEY_6,
        "7" => KeyCode::KEY_7,
        "8" => KeyCode::KEY_8,
        "9" => KeyCode::KEY_9,
        "f1" => KeyCode::KEY_F1,
        "f2" => KeyCode::KEY_F2,
        "f3" => KeyCode::KEY_F3,
        "f4" => KeyCode::KEY_F4,
        "f5" => KeyCode::KEY_F5,
        "f6" => KeyCode::KEY_F6,
        "f7" => KeyCode::KEY_F7,
        "f8" => KeyCode::KEY_F8,
        "f9" => KeyCode::KEY_F9,
        "f10" => KeyCode::KEY_F10,
        "f11" => KeyCode::KEY_F11,
        "f12" => KeyCode::KEY_F12,
        "f13" => KeyCode::KEY_F13,
        "f14" => KeyCode::KEY_F14,
        "f15" => KeyCode::KEY_F15,
        "f16" => KeyCode::KEY_F16,
        "f17" => KeyCode::KEY_F17,
        "f18" => KeyCode::KEY_F18,
        "f19" => KeyCode::KEY_F19,
        "f20" => KeyCode::KEY_F20,
        "pause" | "break" => KeyCode::KEY_PAUSE,
        "space" => KeyCode::KEY_SPACE,
        "enter" | "return" => KeyCode::KEY_ENTER,
        "tab" => KeyCode::KEY_TAB,
        "escape" | "esc" => KeyCode::KEY_ESC,
        "backspace" => KeyCode::KEY_BACKSPACE,
        "delete" => KeyCode::KEY_DELETE,
        "insert" => KeyCode::KEY_INSERT,
        "home" => KeyCode::KEY_HOME,
        "end" => KeyCode::KEY_END,
        "pageup" => KeyCode::KEY_PAGEUP,
        "pagedown" => KeyCode::KEY_PAGEDOWN,
        "up" => KeyCode::KEY_UP,
        "down" => KeyCode::KEY_DOWN,
        "left" => KeyCode::KEY_LEFT,
        "right" => KeyCode::KEY_RIGHT,
        "minus" => KeyCode::KEY_MINUS,
        "equal" => KeyCode::KEY_EQUAL,
        "leftbrace" => KeyCode::KEY_LEFTBRACE,
        "rightbrace" => KeyCode::KEY_RIGHTBRACE,
        "semicolon" => KeyCode::KEY_SEMICOLON,
        "apostrophe" => KeyCode::KEY_APOSTROPHE,
        "grave" => KeyCode::KEY_GRAVE,
        "backslash" => KeyCode::KEY_BACKSLASH,
        "comma" => KeyCode::KEY_COMMA,
        "dot" => KeyCode::KEY_DOT,
        "slash" => KeyCode::KEY_SLASH,
        _ => return None,
    };
    Some(code)
}

fn key_to_char(code: u16, shift: bool) -> Option<char> {
    let c = match code {
        16 => 'q',
        17 => 'w',
        18 => 'e',
        19 => 'r',
        20 => 't',
        21 => 'y',
        22 => 'u',
        23 => 'i',
        24 => 'o',
        25 => 'p',
        30 => 'a',
        31 => 's',
        32 => 'd',
        33 => 'f',
        34 => 'g',
        35 => 'h',
        36 => 'j',
        37 => 'k',
        38 => 'l',
        44 => 'z',
        45 => 'x',
        46 => 'c',
        47 => 'v',
        48 => 'b',
        49 => 'n',
        50 => 'm',
        2 => '1',
        3 => '2',
        4 => '3',
        5 => '4',
        6 => '5',
        7 => '6',
        8 => '7',
        9 => '8',
        10 => '9',
        11 => '0',
        57 => ' ',
        51 => ',',
        52 => '.',
        53 => '/',
        12 => '-',
        13 => '=',
        26 => '[',
        27 => ']',
        39 => ';',
        40 => '\'',
        41 => '`',
        43 => '\\',
        _ => return None,
    };
    if shift && c.is_ascii_alphabetic() {
        Some(c.to_ascii_uppercase())
    } else {
        Some(c)
    }
}

// Modifier key codes (left + right).
const MOD_CTRL: &[u16] = &[29, 97];
const MOD_SHIFT: &[u16] = &[42, 54];
const MOD_ALT: &[u16] = &[56, 100];
const MOD_META: &[u16] = &[125, 126];

/// Compiled hotkey + which event it should produce.
struct Binding {
    parsed: HotkeyBinding,
    code: KeyCode,
    event: HotkeyEvent,
}

/// Passive / consuming evdev listener.
pub struct EvdevHotkeySource {
    rx: Mutex<Option<mpsc::Receiver<HotkeyEvent>>>,
}

/// Options for the evdev hotkey source.
pub struct EvdevOptions {
    pub bindings: Vec<(HotkeyBinding, HotkeyEvent)>,
    /// EVIOCGRAB + uinput pass-through so the hotkey does not reach apps.
    pub consume: bool,
    /// Typing buffer (for last-word mode).
    pub buffer: Arc<SharedTypingBuffer>,
}

impl EvdevHotkeySource {
    pub fn open(buffer: Arc<SharedTypingBuffer>) -> std::io::Result<Self> {
        let defaults = vec![
            (HotkeyBinding::parse("ctrl+f1").unwrap(), HotkeyEvent::ConvertSelection),
            (HotkeyBinding::parse("ctrl+f2").unwrap(), HotkeyEvent::ConvertLastWord),
            (
                HotkeyBinding::parse("ctrl+shift+f1").unwrap(),
                HotkeyEvent::ConvertTerminal,
            ),
            (HotkeyBinding::parse("ctrl+pause").unwrap(), HotkeyEvent::ToggleEnabled),
        ];
        Self::open_with(EvdevOptions {
            bindings: defaults,
            consume: false,
            buffer,
        })
    }

    pub fn open_with(opts: EvdevOptions) -> std::io::Result<Self> {
        let (tx, rx) = mpsc::channel(32);

        let mut compiled: Vec<Binding> = Vec::new();
        for (parsed, event) in opts.bindings {
            match key_name_to_code(&parsed.key) {
                Some(code) => compiled.push(Binding {
                    parsed,
                    code,
                    event,
                }),
                None => {
                    warn!("hotkey key '{key}' has no evdev code — skipped", key = parsed.key);
                }
            }
        }

        // Open the pass-through uinput device BEFORE grabbing, so a failure
        // never leaves the keyboards grabbed with no way to type.
        let passthrough = if opts.consume {
            match Passthrough::open() {
                Ok(p) => {
                    info!("passthrough uinput ready");
                    Some(p)
                }
                Err(e) => {
                    warn!("passthrough uinput failed ({e}); hotkeys stay passive");
                    None
                }
            }
        } else {
            None
        };
        let consume = opts.consume && passthrough.is_some();

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
                Ok(mut dev) => {
                    if !dev.supported_keys().is_some_and(|k| k.contains(KeyCode::KEY_A)) {
                        continue;
                    }
                    debug!("evdev keyboard: {:?} ({:?})", path, dev.name());
                    if consume {
                        match dev.grab() {
                            Ok(()) => info!("evdev grabbed {:?} (hotkey_consume)", path),
                            Err(e) => warn!("EVIOCGRAB failed on {:?}: {e}", path),
                        }
                    }
                    devices.push(dev);
                }
                Err(e) => debug!("skip {:?}: {e}", path),
            }
        }

        if devices.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no keyboard devices (need group 'input')",
            ));
        }

        info!(
            "evdev: tracking {} keyboard(s), consume={}, bindings={}",
            devices.len(),
            consume,
            compiled.len()
        );

        let buffer = opts.buffer;
        std::thread::spawn(move || {
            run_loop(devices, compiled, buffer, tx, passthrough, consume);
        });

        Ok(Self {
            rx: Mutex::new(Some(rx)),
        })
    }
}

/// Raw uinput handle used to re-inject keys when consume mode is on.
struct Passthrough {
    device: Mutex<evdev::uinput::VirtualDevice>,
}

impl Passthrough {
    fn open() -> std::io::Result<Self> {
        use evdev::uinput::VirtualDeviceBuilder;
        let mut keys = evdev::AttributeSet::<KeyCode>::new();
        for i in 0..=255u16 {
            keys.insert(KeyCode(i));
        }
        #[allow(deprecated)]
        let device = VirtualDeviceBuilder::new()
            .map_err(|e| std::io::Error::other(e.to_string()))?
            .name("switchfix-passthrough")
            .with_keys(&keys)
            .map_err(|e| std::io::Error::other(e.to_string()))?
            .build()
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        Ok(Self {
            device: Mutex::new(device),
        })
    }

    fn emit_key(&self, code: u16, value: i32) {
        let events = [
            InputEvent::new(EventType::KEY.0, code, value),
            InputEvent::new(EventType::SYNCHRONIZATION.0, 0, 0),
        ];
        if let Ok(mut dev) = self.device.lock() {
            let _ = dev.emit(&events);
        }
    }
}

fn run_loop(
    mut devices: Vec<Device>,
    bindings: Vec<Binding>,
    buffer: Arc<SharedTypingBuffer>,
    tx: mpsc::Sender<HotkeyEvent>,
    passthrough: Option<Passthrough>,
    consume: bool,
) {
    // When consuming, we must re-inject everything that is not a hotkey.
    let consuming = consume && passthrough.is_some();

    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut meta = false;

    loop {
        for dev in devices.iter_mut() {
            let events = match dev.fetch_events() {
                Ok(e) => e,
                Err(e) => {
                    warn!("evdev read error: {e}");
                    continue;
                }
            };

            for ev in events {
                if ev.event_type() != EventType::KEY {
                    // In consume mode forward non-key events? Usually LED/REP — drop.
                    continue;
                }
                let code = ev.code();
                let value = ev.value(); // 0=up 1=down 2=repeat

                // Track modifiers (left+right).
                if MOD_CTRL.contains(&code) {
                    ctrl = value != 0;
                    if consuming {
                        if let Some(p) = &passthrough {
                            p.emit_key(code, value);
                        }
                    }
                    continue;
                }
                if MOD_SHIFT.contains(&code) {
                    shift = value != 0;
                    if consuming {
                        if let Some(p) = &passthrough {
                            p.emit_key(code, value);
                        }
                    }
                    continue;
                }
                if MOD_ALT.contains(&code) {
                    alt = value != 0;
                    if consuming {
                        if let Some(p) = &passthrough {
                            p.emit_key(code, value);
                        }
                    }
                    continue;
                }
                if MOD_META.contains(&code) {
                    meta = value != 0;
                    if consuming {
                        if let Some(p) = &passthrough {
                            p.emit_key(code, value);
                        }
                    }
                    continue;
                }

                // Hotkey match on key-down (key-up/rep of the same key is swallowed
                // too so the app never sees a half-press).
                let matched = bindings.iter().find(|b| {
                    b.code.0 == code
                        && b.parsed.ctrl == ctrl
                        && b.parsed.alt == alt
                        && b.parsed.shift == shift
                        && b.parsed.meta == meta
                });

                if let Some(b) = matched {
                    if value == 1 {
                        let _ = tx.try_send(b.event.clone());
                    }
                    // Consume the key: do not forward, do not put in buffer.
                    continue;
                }

                // Typing buffer (only for real character keys on key-down).
                if value == 1 {
                    if code == 14 {
                        buffer.pop();
                    } else if let Some(c) = key_to_char(code, shift) {
                        buffer.push(c);
                    }
                }

                // Consume mode: re-inject everything else so typing still works.
                if consuming {
                    if let Some(p) = &passthrough {
                        p.emit_key(code, value);
                    }
                }
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(3));
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
