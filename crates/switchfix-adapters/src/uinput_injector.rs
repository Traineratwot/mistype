//! Text injection via Linux uinput virtual keyboard.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use evdev::uinput::{VirtualDevice, VirtualDeviceBuilder};
use evdev::{AttributeSet, InputEvent, KeyCode};
use switchfix_app::ports::{InjectError, TextInjector};
use switchfix_domain::{en_to_ru_map, KeyChord, LayoutId};
use tracing::debug;

/// Character → (KeyCode, needs_shift) for a physical layout.
/// Built from the US QWERTY base + layout map so that typing under RU layout
/// produces the intended Cyrillic characters.
struct KeyMap {
    /// char → (KeyCode, shift)
    map: HashMap<char, (KeyCode, bool)>,
}

impl KeyMap {
    fn for_layout(layout: &LayoutId) -> Self {
        let mut map: HashMap<char, (KeyCode, bool)> = HashMap::new();

        // US QWERTY physical keys → chars (lowercase / uppercase).
        let us: &[(char, char, KeyCode)] = &[
            ('q', 'Q', KeyCode::KEY_Q), ('w', 'W', KeyCode::KEY_W), ('e', 'E', KeyCode::KEY_E),
            ('r', 'R', KeyCode::KEY_R), ('t', 'T', KeyCode::KEY_T), ('y', 'Y', KeyCode::KEY_Y),
            ('u', 'U', KeyCode::KEY_U), ('i', 'I', KeyCode::KEY_I), ('o', 'O', KeyCode::KEY_O),
            ('p', 'P', KeyCode::KEY_P), ('a', 'A', KeyCode::KEY_A), ('s', 'S', KeyCode::KEY_S),
            ('d', 'D', KeyCode::KEY_D), ('f', 'F', KeyCode::KEY_F), ('g', 'G', KeyCode::KEY_G),
            ('h', 'H', KeyCode::KEY_H), ('j', 'J', KeyCode::KEY_J), ('k', 'K', KeyCode::KEY_K),
            ('l', 'L', KeyCode::KEY_L), ('z', 'Z', KeyCode::KEY_Z), ('x', 'X', KeyCode::KEY_X),
            ('c', 'C', KeyCode::KEY_C), ('v', 'V', KeyCode::KEY_V), ('b', 'B', KeyCode::KEY_B),
            ('n', 'N', KeyCode::KEY_N), ('m', 'M', KeyCode::KEY_M),
            ('1', '!', KeyCode::KEY_1), ('2', '@', KeyCode::KEY_2), ('3', '#', KeyCode::KEY_3),
            ('4', '$', KeyCode::KEY_4), ('5', '%', KeyCode::KEY_5), ('6', '^', KeyCode::KEY_6),
            ('7', '&', KeyCode::KEY_7), ('8', '*', KeyCode::KEY_8), ('9', '(', KeyCode::KEY_9),
            ('0', ')', KeyCode::KEY_0),
            ('-', '_', KeyCode::KEY_MINUS), ('=', '+', KeyCode::KEY_EQUAL),
            ('[', '{', KeyCode::KEY_LEFTBRACE), (']', '}', KeyCode::KEY_RIGHTBRACE),
            (';', ':', KeyCode::KEY_SEMICOLON), ('\'', '"', KeyCode::KEY_APOSTROPHE),
            ('`', '~', KeyCode::KEY_GRAVE), ('\\', '|', KeyCode::KEY_BACKSLASH),
            (',', '<', KeyCode::KEY_COMMA), ('.', '>', KeyCode::KEY_DOT),
            ('/', '?', KeyCode::KEY_SLASH), (' ', ' ', KeyCode::KEY_SPACE),
        ];

        for &(lo, up, key) in us {
            map.insert(lo, (key, false));
            map.insert(up, (key, true));
        }

        // For RU layout: chars we want to type come from en→ru map values.
        // Physical key that produces them = reverse lookup in en_to_ru_map.
        if layout.as_str() == "ru" {
            let en_ru = en_to_ru_map();
            // Build reverse: ru_char → en_char, then use US key of en_char.
            let mut ru_map: HashMap<char, (KeyCode, bool)> = HashMap::new();
            for (en_ch, ru_ch) in &en_ru {
                if let Some((key, shift)) = map.get(en_ch).copied() {
                    ru_map.insert(*ru_ch, (key, shift));
                }
            }
            // Space and digits that stay the same
            if let Some(v) = map.get(&' ') {
                ru_map.insert(' ', *v);
            }
            return Self { map: ru_map };
        }

        // "us" and unknown layouts: use US map
        Self { map }
    }

    fn lookup(&self, c: char) -> Option<(KeyCode, bool)> {
        self.map.get(&c).copied()
    }
}

/// uinput virtual keyboard injector.
pub struct UinputInjector {
    device: Mutex<VirtualDevice>,
    maps: Mutex<HashMap<String, KeyMap>>,
}

impl UinputInjector {
    pub fn open() -> Result<Self, InjectError> {
        let mut keys = AttributeSet::<KeyCode>::new();
        for k in ALL_KEYS {
            keys.insert(*k);
        }

        #[allow(deprecated)]
        let device = VirtualDeviceBuilder::new()
            .map_err(|e| InjectError::Device(e.to_string()))?
            .name("switchfix-virtual-keyboard")
            .with_keys(&keys)
            .map_err(|e| InjectError::Device(e.to_string()))?
            .build()
            .map_err(|e| InjectError::Device(e.to_string()))?;

        debug!("uinput device created");
        Ok(Self {
            device: Mutex::new(device),
            maps: Mutex::new(HashMap::new()),
        })
    }

    fn emit(&self, events: &[InputEvent]) -> Result<(), InjectError> {
        let mut dev = self
            .device
            .lock()
            .map_err(|_| InjectError::Failed("device lock poisoned".into()))?;
        dev.emit(events)
            .map_err(|e| InjectError::Failed(e.to_string()))
    }

    fn key_down(&self, key: KeyCode) -> Result<(), InjectError> {
        self.emit(&[
            InputEvent::new(evdev::EventType::KEY.0, key.0, 1),
            InputEvent::new(evdev::EventType::SYNCHRONIZATION.0, 0, 0),
        ])
    }

    fn key_up(&self, key: KeyCode) -> Result<(), InjectError> {
        self.emit(&[
            InputEvent::new(evdev::EventType::KEY.0, key.0, 0),
            InputEvent::new(evdev::EventType::SYNCHRONIZATION.0, 0, 0),
        ])
    }

    fn tap(&self, key: KeyCode) -> Result<(), InjectError> {
        self.key_down(key)?;
        self.key_up(key)
    }

    fn chord_combo(&self, keys: &[KeyCode]) -> Result<(), InjectError> {
        for k in keys {
            self.key_down(*k)?;
        }
        for k in keys.iter().rev() {
            self.key_up(*k)?;
        }
        Ok(())
    }

    fn map_for(&self, layout: &LayoutId) -> Result<KeyMap, InjectError> {
        let mut maps = self
            .maps
            .lock()
            .map_err(|_| InjectError::Failed("map lock poisoned".into()))?;
        if !maps.contains_key(layout.as_str()) {
            maps.insert(layout.as_str().to_string(), KeyMap::for_layout(layout));
        }
        Ok(maps.get(layout.as_str()).unwrap().clone_shim())
    }
}

// KeyMap is cheap to rebuild; avoid Clone derive on KeyCode type issues.
impl KeyMap {
    fn clone_shim(&self) -> KeyMap {
        KeyMap {
            map: self.map.clone(),
        }
    }
}

const ALL_KEYS: &[KeyCode] = &[
    KeyCode::KEY_A, KeyCode::KEY_B, KeyCode::KEY_C, KeyCode::KEY_D, KeyCode::KEY_E, KeyCode::KEY_F,
    KeyCode::KEY_G, KeyCode::KEY_H, KeyCode::KEY_I, KeyCode::KEY_J, KeyCode::KEY_K, KeyCode::KEY_L,
    KeyCode::KEY_M, KeyCode::KEY_N, KeyCode::KEY_O, KeyCode::KEY_P, KeyCode::KEY_Q, KeyCode::KEY_R,
    KeyCode::KEY_S, KeyCode::KEY_T, KeyCode::KEY_U, KeyCode::KEY_V, KeyCode::KEY_W, KeyCode::KEY_X,
    KeyCode::KEY_Y, KeyCode::KEY_Z,
    KeyCode::KEY_1, KeyCode::KEY_2, KeyCode::KEY_3, KeyCode::KEY_4, KeyCode::KEY_5, KeyCode::KEY_6,
    KeyCode::KEY_7, KeyCode::KEY_8, KeyCode::KEY_9, KeyCode::KEY_0,
    KeyCode::KEY_MINUS, KeyCode::KEY_EQUAL, KeyCode::KEY_LEFTBRACE, KeyCode::KEY_RIGHTBRACE,
    KeyCode::KEY_SEMICOLON, KeyCode::KEY_APOSTROPHE, KeyCode::KEY_GRAVE, KeyCode::KEY_BACKSLASH,
    KeyCode::KEY_COMMA, KeyCode::KEY_DOT, KeyCode::KEY_SLASH, KeyCode::KEY_SPACE,
    KeyCode::KEY_ENTER, KeyCode::KEY_BACKSPACE, KeyCode::KEY_TAB, KeyCode::KEY_ESC,
    KeyCode::KEY_LEFTCTRL, KeyCode::KEY_RIGHTCTRL, KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_RIGHTSHIFT,
    KeyCode::KEY_LEFTALT, KeyCode::KEY_RIGHTALT, KeyCode::KEY_LEFTMETA, KeyCode::KEY_RIGHTMETA,
    KeyCode::KEY_INSERT, KeyCode::KEY_PAUSE,
];

#[async_trait]
impl TextInjector for UinputInjector {
    async fn backspace(&self, count: usize) -> Result<(), InjectError> {
        for _ in 0..count {
            self.tap(KeyCode::KEY_BACKSPACE)?;
        }
        Ok(())
    }

    async fn type_text(&self, text: &str, layout: &LayoutId) -> Result<(), InjectError> {
        let map = self.map_for(layout)?;
        for c in text.chars() {
            let (key, shift) = map.lookup(c).ok_or_else(|| InjectError::UnknownChar {
                layout: layout.as_str().to_string(),
                ch: c,
            })?;
            if shift {
                self.key_down(KeyCode::KEY_LEFTSHIFT)?;
                self.tap(key)?;
                self.key_up(KeyCode::KEY_LEFTSHIFT)?;
            } else {
                self.tap(key)?;
            }
        }
        Ok(())
    }

    async fn chord(&self, combo: KeyChord) -> Result<(), InjectError> {
        let keys: &[KeyCode] = match combo {
            KeyChord::CtrlInsert => &[KeyCode::KEY_LEFTCTRL, KeyCode::KEY_INSERT],
            KeyChord::CtrlV => &[KeyCode::KEY_LEFTCTRL, KeyCode::KEY_V],
            KeyChord::CtrlShiftV => &[
                KeyCode::KEY_LEFTCTRL,
                KeyCode::KEY_LEFTSHIFT,
                KeyCode::KEY_V,
            ],
            KeyChord::CtrlA => &[KeyCode::KEY_LEFTCTRL, KeyCode::KEY_A],
            KeyChord::CtrlK => &[KeyCode::KEY_LEFTCTRL, KeyCode::KEY_K],
            KeyChord::CtrlF1 => &[KeyCode::KEY_LEFTCTRL, KeyCode::KEY_F1],
            KeyChord::CtrlF2 => &[KeyCode::KEY_LEFTCTRL, KeyCode::KEY_F2],
            KeyChord::CtrlShiftF1 => &[
                KeyCode::KEY_LEFTCTRL,
                KeyCode::KEY_LEFTSHIFT,
                KeyCode::KEY_F1,
            ],
            KeyChord::CtrlPause => &[KeyCode::KEY_LEFTCTRL, KeyCode::KEY_PAUSE],
            KeyChord::MetaSpace => &[KeyCode::KEY_LEFTMETA, KeyCode::KEY_SPACE],
        };
        self.chord_combo(keys)
    }
}

// KeyCode doesn't implement Clone in all evdev versions — store Copy keys.
impl Clone for KeyMap {
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
        }
    }
}
