# SwitchFix — план разработки

Tray-утилита для **Kubuntu (Debian + KDE Plasma 6, Wayland-only MVP)**: конвертация текста «не той раскладкой» + переключение системной раскладки.

| Решение | Значение |
|---------|----------|
| Рабочее имя | `switchfix` (переименуемо) |
| Стек | **Rust** + iced (settings) + ksni (tray) + zbus + evdev/uinput |
| Режим MVP | **Горячая клавиша** (не auto-Punto) |
| Scope MVP | Раскладка EN↔RU + layout switch + tray + settings |
| Не в MVP | Спеллинг, сниппеты, auto-mode, X11/GNOME (фазы 2–5) |

---

## 1. Аналоги

### Прямые (исправление раскладки)

| Проект | Платформа / стек | Что делает | Берём / избегаем |
|--------|------------------|------------|------------------|
| **Punto Switcher** (Яндекс) | Windows | Авто-детект раскладки, замена, спеллчек | UX-эталон. **Избегаем** auto в MVP |
| **Keyboop** | macOS | Аналог Punto | Подтверждает спрос |
| **XNeur / gXNeur / kdeNeur** | Linux X11, C/GTK/Qt | Авто-переключение по словарям | Идея словарей (phase 4). **Избегаем** X11, заброшенности |
| **xswitcher** (ds-voix) | X11, Go | Keylogger + vkeyboard, backspace+retype | Паттерн retype без чтения текста. **Избегаем** X11/AGPL |
| **puny-switcher** (roadkell) | X11+GNOME, bash+kanata | Хоткей → clipboard → convert → paste | Разделение «конверт» / «switch layout». **Избегаем** kanata, GNOME-only |
| **WordWrap / sgk-wordwrap** (sgk-dev) | GNOME Wayland, Python+PyQt6 | **Главный UX-референс:** hotkey, clipboard-marker, convert, paste, switch layout, tray, terminal mode | Marker-протокол, fallback last word, privacy stance, settle-задержки. **Избегаем** `gsettings`/GNOME |
| **xlayout-switcher** (sngvy) | X11/Wayland | Конверт раскладки | Простая утилита |
| **kb-layout-daemon** (aydiler) | **KDE Plasma**, **Rust**+evdev+uinput+zbus | Layout по физ. клавиатуре, grab/passive | **Точный D-Bus API KDE** (§3.4), systemd unit. **Избегаем** grab по умолчанию |
| **g3kb-switch** (lyokha) | GNOME, C | Layout switch через D-Bus | Идея «LayoutSwitcher-адаптер на DE» |

### Смежные

| Проект | Стек | Что делает | Урок |
|--------|------|------------|------|
| **Espanso** | Rust | Text expander | Разделение detect/inject/clipboard/engine — наша §4 |
| **AutoKey** | Python, X11 | Автоматизация / snippets | X11-only, не ядро |
| **kbcut** (hhpmacedo) | evdev+uinput | Буфер → backspaces+retype, без clipboard | Primary path для last word (phase 3 snippets) |
| **kanata / keyd / kmonad** | Rust/C/HS | Low-level remap | Слишком низко; паттерны evdev |
| **im-select** | C++ | CLI switch IME | Маленький helper |

### Спеллинг (фаза 2)

**zspell** (pure Rust, Hunspell-формат) · системные словари `/usr/share/hunspell` · user.dic · one-step neighbors для очевидных опечаток · LanguageTool (опц., тяжёлый) · aspell (legacy)

### Выводы

1. Wayland: инъекция **только uinput** (`wtype`/`ydotool`/`xdotool` — dead-end).
2. Selection без AT-SPI — **clipboard-marker** (wordwrap) или **keystroke-буфер** (kbcut).
3. Layout switch в KDE — **D-Bus `org.kde.KeyboardLayouts`**, не `setxkbmap`/`gsettings`.
4. Архитектура espanso (порты/адаптеры).
5. MVP = hotkey-first (приватность, проще тесты); auto — phase 4.

---

## 2. Стек

| Слой | Выбор | Почему | Не берём |
|------|-------|--------|----------|
| Язык | **Rust** | Один бинарь, evdev/uinput крейты, espanso/как референс | C++/Qt, Python, Go |
| Settings GUI | **iced** (fallback egui) | Тестируемые редьюсеры, без webview | tauri (тяжёлый webview) |
| Tray | **ksni** | Нативный SNI для KDE, async | tray-icon (запасной для GNOME) |
| D-Bus | **zbus** | Async, безопасный | dbus-rs |
| Capture | **evdev** | Глобальный буфер + хоткеи под Wayland | AT-SPI (хрупко) |
| Inject | **uinput** (`evdev` crate) | Ядерная vkeyboard, не блокируется Wayland | wtype/ydotool/XTest |
| Clipboard | `wl-clipboard-rs` (+ `arboard`) | save/put/get/restore | — |
| Конфиг | `serde`+`toml` `~/.config/switchfix/` | Человекочитаемо | KConfig |
| Спеллинг (P2) | **zspell** (pure Rust, hunspell-формат) | Без FFI, легко паковать | LanguageTool, hunspell FFI |
| Упаковка | `.deb` + systemd user | Kubuntu-first | Flatpak (усложняет /dev) |

**Паттерн процессов (снимает конфликт iced ↔ ksni):**
- `switchfix daemon` — tray + hotkeys + конвертация (long-running)
- `switchfix settings` — отдельный процесс окна настроек

**Crate-лист MVP:** `zbus`, `tokio`, `ksni`, `iced`, `evdev`, `tokio-udev`, `wl-clipboard-rs`, `arboard`, `serde`, `toml`, `dirs`, `thiserror`, `tracing`, `clap`.

---

## 3. Взаимодействие с вводом

### 3.1 Триггер (hotkey)

| Подход | Плюсы | Минусы | Роль |
|--------|-------|--------|------|
| XDG GlobalShortcuts portal | Без `input` group, user consent | Диалог, задержки, не все chords | Fallback / opt-in |
| **evdev passive** | Полный контроль + keystroke-буфер | Группа `input` | ✅ **Primary** |
| KDE KGlobalAccel D-Bus | Виден в System Settings | KDE-only, сложнее API | Fallback |

**Дефолтные хоткеи:** `Ctrl+F1` selection · `Ctrl+Pause` toggle.

Pass-through (не consume) + settle-delay; опция `hotkey_consume` (EVIOCGRAB) — не дефолт.

**Fallback без `input`:** portal → tray menu / `switchfix convert` CLI.

### 3.2 Захват текста

| Режим | Механизм | Роль |
|-------|----------|------|
| Selection | Clipboard-marker: save → маркер → `Ctrl+Insert` → если маркер **не** пережил → читаем selection | ✅ Primary selection |
| Last word | Keystroke-буфер (evdev, RAM, N=256): `take_last_word()` | ✅ Primary word, без clipboard |
| AT-SPI | Чтение focused widget | ❌ не MVP |

**Порядок:** selection (marker) → fallback word (если `fallback_to_word`) → toast «Нечего конвертировать».

### 3.3 Замена текста

| Способ | Механика | Когда |
|--------|----------|-------|
| **A. Без clipboard** | N× Backspace + uinput type_text под **целевой** раскладкой | last word / terminal |
| **B. Clipboard+paste** | put(result) → `Ctrl+V` / `Ctrl+Shift+V` → restore | selection |
| C. Hybrid | ASCII/кириллица и len≤32 → A; иначе B | опция `replace_strategy=auto` |

**Terminal mode:** `Ctrl+A`, `Ctrl+K`, `Ctrl+Shift+V` (опция `terminal_erase=line|backspaces`).

### 3.4 Layout switch (KDE Plasma 6)

**Primary API** (из `aydiler/kb-layout-daemon`):

| Поле | Значение |
|------|----------|
| Destination | `org.kde.keyboard` |
| Path | `/Layouts` |
| Interface | `org.kde.KeyboardLayouts` |
| `setLayout(u) → b` | индекс 0-based = порядок в System Settings |
| `getLayout() → u` | текущий индекс |
| Signal `layoutChanged(u)` | внешняя смена |

Poll `getLayout()` до совпадения (~50 ms). После hot-plug — перечитывать (Plasma сбрасывает layout).

| Fallback | Метод |
|----------|-------|
| 1 | `kwriteconfig6` (kxkbrc) + `org.kde.KWin.reconfigure` (**to verify** на Plasma 6) |
| 2 | Синтетика `Meta+Space` через uinput |
| 3 (out of MVP) | X11 `setxkbmap` |

### 3.5 Свод primary/fallback

| Задача | Primary | Fallback |
|--------|---------|----------|
| Hotkey | evdev passive | portal → tray/CLI |
| Selection | clipboard-marker | last word |
| Last word | keystroke buffer | clipboard-marker word |
| Replace word | uinput backspace+type | clipboard+V |
| Replace selection | clipboard+V | uinput type (ASCII) |
| Replace terminal | Ctrl+A/K + Ctrl+Shift+V | backspaces |
| Layout switch | `org.kde.KeyboardLayouts` | kwriteconfig6 → Meta+Space |
| Clipboard | wl-clipboard-rs | arboard |

---

## 4. Архитектура (SOLID)

### Слои

```
UI/CLI (iced, ksni, clap)  →  App (use-cases, ports=traits)  →  Domain (pure)
                                      ↑ implements
                                 Adapters (evdev, uinput, zbus, clipboard, toml)
```

Domain — без I/O. App — только traits. Adapters реализуют порты. Зависимости вниз; DI через `Arc<dyn Port>` в `AppContext`.

### Workspace

```
switchfix/
├── Cargo.toml
├── crates/
│   ├── switchfix-domain/src/
│   │   ├── conversion.rs      # ConversionTable, convert(), detect_direction()
│   │   ├── buffer.rs          # TypingBuffer, WordSpan
│   │   ├── model.rs           # LayoutId, CorrectionRequest, CorrectionOutcome
│   │   ├── state.rs           # CorrectionState
│   │   └── error.rs
│   ├── switchfix-app/src/
│   │   ├── ports.rs           # traits
│   │   ├── services/{correction,layout,config}.rs
│   │   ├── commands.rs
│   │   └── context.rs         # AppContext
│   ├── switchfix-adapters/src/
│   │   ├── evdev_hotkey.rs    # EvdevHotkeySource
│   │   ├── evdev_buffer.rs    # EvdevKeystrokeBuffer
│   │   ├── uinput_injector.rs # UinputInjector
│   │   ├── kde_layout.rs      # KdeLayoutSwitcher (zbus)
│   │   ├── wl_clipboard.rs    # WaylandClipboard
│   │   ├── portal_hotkey.rs   # PortalHotkeySource
│   │   └── toml_config.rs
│   ├── switchfix-ui/src/
│   │   ├── tray.rs            # ksni
│   │   ├── settings_app.rs    # iced
│   │   └── pages/             # General, Hotkeys, Layouts, Privacy, About
│   └── switchfix-daemon/src/
│       ├── main.rs            # clap: daemon|settings|convert|doctor|toggle
│       ├── wiring.rs          # composition root
│       └── doctor.rs
├── assets/{icons,maps}/
├── packaging/{deb,systemd,desktop,install.sh}
├── tests/{integration,fakes}/
└── docs/{PRIVACY,SECURITY,MANUAL-TEST-MATRIX}.md
```

### Порты (`switchfix-app/src/ports.rs`)

```rust
TextInjector:     backspace(n), type_text(text, layout), chord(combo)
ClipboardPort:    save(), put_text(), get_text(), restore()
SelectionCapture: capture_selection() -> Option<String>   // marker protocol
TypingBufferPort: take_last_word(), clear(), is_enabled()
LayoutSwitcher:   current(), set(index), layouts(), subscribe()
HotkeySource:     events() -> Receiver<HotkeyEvent>
ConfigStore:      load(), save()
Notifier:         notify(msg, level)
```

### Ключевые типы (domain)

```rust
ConversionTable { from, to, map: HashMap<char,char> }
detect_direction(text) -> Option<(LayoutId, LayoutId)>
convert(text, tables) -> Option<ConvertedText>
WordSpan { text, char_len }

CorrectionRequest::{ ConvertSelection, ConvertTerminal }
CorrectionOutcome::{ Converted{..}, NoInputFound, Disabled, Failed(..) }
CorrectionState::{ Idle, Capturing, Converting, Replacing,
                   SwitchingLayout, RestoringClipboard, Error{recoverable} }
```

### Composition root

```rust
AppContext { correction, config, hotkeys, tray_model, shutdown }
build(): Config → EvdevHotkey|PortalHotkey → UinputInjector
       → KdeLayoutSwitcher → WaylandClipboard → EvdevKeystrokeBuffer
```

SOLID: **S** — узкие модули · **O** — новые LayoutSwitcher/Injector без правки core · **L** — фейки в тестах · **I** — узкие traits · **D** — CorrectionService зависит от портов.

---

## 5. Тест-кейсы

### Unit (domain)

| Модуль | Кейсы |
|--------|-------|
| conversion | `ghbdtn`→`привет`, `руддщ`→`hello`, bidirectional, detect_direction (EN/RU/mixed/empty/цифры-знаки), регистр, неизвестные символы, custom JSON map |
| buffer | take_last_word (пробелы/пунктуация), backspace, Ctrl+Backspace, empty→None, overflow, unicode, opt-out, clear после convert |
| state | переходы Idle→…→Idle; NoInputFound; Error(recoverable)→Idle; toggle |
| config | TOML round-trip, дефолты, валидация hotkey/layout_index |

### Integration (фейки)

| Сценарий | Проверяем |
|----------|-----------|
| Convert selection happy | порядок: save→put(marker)→chord(Ctrl+Insert)→get→convert→put→chord(Ctrl+V)→restore→set(layout) |
| Нет выделения | NoInputFound / fallback word; clipboard восстановлен |
| Convert last word | backspace×N → type_text → set(layout); clipboard **не** трогали |
| Terminal | chord(Ctrl+A), (Ctrl+K), (Ctrl+Shift+V) |
| Layout switch fail | текст **не** откатываем; warning |
| Clipboard restore fail | warning; core не откатывается |
| Toggle disabled | injector не вызван |
| EACCES `/dev/input` | doctor hint `usermod -aG input` |

### Manual matrix (Kate, Firefox, Konsole, Telegram, VS Code)

| # | Сценарий | Ожидание |
|---|----------|----------|
| M1 | Выделить `ghbdtn`, Ctrl+F1 | `привет`, layout=RU |
| M2 | Без выделения | NoInputFound (нет пути last word) |
| M3 | Пустое выделение | NoInputFound, clipboard ok |
| M4 | Mixed `hello мир` | convert по detect_direction |
| M5 | Password-поле | skip / exclude list |
| M6 | Konsole + Ctrl+Shift+F1 | clear line + paste |
| M7 | Toggle paused | no-op, серая иконка |
| M8 | Пользовательский Meta+Space | current синхронизирован |
| M9 | USB keyboard hot-plug | переподключение |
| M10–M11 | VS Code search / Firefox contenteditable | selection path |
| M12 | Добавить UA layout | EN↔RU ok, UA ignore |
| M13 | Без `input` group | doctor fail, settings/tray живы |
| M14 | Clipboard = картинка | restore skip (doc) |

---

## 6. Логика приложения

### Convert selection (`Ctrl+F1`)

```
Capturing → enabled? → save clipboard → put(marker) → chord(Ctrl+Insert)
→ wait(copy_settle_ms) → selected = get()
→ if selected == marker: fallback word | NoInputFound
→ Converting: convert(selected)  // detect direction
→ if unchanged: restore, NoInputFound
→ Replacing: put(converted) → chord(Ctrl+V|Ctrl+Shift+V) → wait(paste_settle_ms)
→ RestoringClipboard: restore(saved)   // best-effort
→ SwitchingLayout: set(target)         // best-effort, текст не откатываем
→ Idle
```

### Convert last word

**Removed** — selection-only convert. No Backspace fallback (it would wipe an
active selection). Terminal apps use ConvertTerminal automatically.

### Terminal (auto, no hotkey)

When the focused app matches `behavior.terminal_apps`, ConvertSelection uses
`Ctrl+A`, `Ctrl+K`, `Ctrl+Shift+V` (опция `terminal_erase`).

### Toggle / Settings / Quit

- **Toggle:** `enabled=!enabled`, icon update, `buffer.clear()` (privacy), notify.
- **Settings:** spawn `switchfix settings` (iced, отдельный процесс); Save → daemon auto-restart (re-exec).
- **Quit:** cancel token → join hotkey → drop uinput → D-Bus unsubscribe → exit.

### State machine

```
Idle → Capturing → Converting → Replacing → SwitchingLayout
  → RestoringClipboard → Idle
  ↘ NoInputFound | Disabled | Error(recoverable) → Idle
```

### Ошибки

| Ошибка | Реакция |
|--------|---------|
| Нет ввода | NoInputFound, тихо |
| EACCES `/dev/input` | portal fallback, doctor hint |
| EACCES `/dev/uinput` | fatal для convert; tray/settings живы |
| D-Bus layout fail | fallback kwriteconfig6 / Meta+Space; warning |
| Layout switch fail | текст **не** откатываем; «Текст исправлен, раскладка — нет» |
| Clipboard restore fail | warning; не откатываем |
| Password-поля | exclude list + эвристика; док |
| Panic в адаптере | catch_unwind → Error(recoverable) |

---

## 7. Дорожная карта

| Фаза | Объём | Exit |
|------|-------|------|
| **0 Spike** (1–2 дн.) | Подтвердить D-Bus `setLayout/getLayout`; uinput печатает в Kate; clipboard-marker читает selection; iced vs egui | Прототипы работают |
| **1 MVP** (2–4 нед.) | domain EN↔RU, adapters, CorrectionService, tray, settings, CLI (`daemon\|settings\|convert\|doctor\|toggle`), .deb, tests | M1, M2, M6, M7 зелёные в 5 app |
| **2 Spellcheck** | SpellPort + zspell, хоткей «исправить слово», полоса подсказок, completions, layout-convert candidates, user.dic, сигнал | unit + manual M15–M23 |
| **3 Snippets** | SnippetStore, expand по триггеру (keystroke-буфер), `{date}`/`{time}` | expand tests |
| **4 Auto mode** | AutoDetector по словарям; default **off**; privacy | opt-in |
| **5 Другие DE** | X11 (XTest/setxkbmap), GNOME (portal, gsettings), AUR/.rpm/Flatpak | adapter matrix |

---

## 8. Упаковка (Kubuntu)

**.deb:** `/usr/bin/switchfix`, desktop entry, icons, `/usr/lib/systemd/user/switchfix.service`, maps, PRIVACY/SECURITY.

**postinst:** подсказка `sudo usermod -aG input $USER` + `systemctl --user enable --now switchfix` (не включать автоматически).

**udev (опц.):** `KERNEL=="uinput", GROUP="input", MODE="0660"`.

**systemd user unit:** `PartOf=graphical-session.target`, `Restart=on-failure`, `NoNewPrivileges`, `ProtectSystem=strict`, `RestrictAddressFamilies=AF_UNIX AF_NETLINK`.

**Dev:** `cargo build --release && cargo test && ./packaging/install.sh && switchfix doctor`.

---

## 9. Безопасность и приватность

| Принцип | Реализация |
|---------|------------|
| Не keylogger | Keystroke-буфер только в RAM (≤256), opt-in, drop на exit; **не** пишем текст в логи |
| Локально | Нет сети/телеметрии; systemd sandbox |
| Clipboard | save/restore best-effort; не-текстовый не восстанавливаем (doc) |
| Password-поля | exclude list + эвристика; док «не жмите хоткей в паролях» |
| Группа `input` | Документируем риск в SECURITY.md; альтернатива — portal (без буфера) |
| Лицензия | MIT |

UX-строка Settings→Privacy: «SwitchFix не отправляет данные в сеть и не сохраняет введённый текст…»

---

## 10. Риски и открытые вопросы

| Риск | Митигация |
|------|-----------|
| D-Bus API Plasma 6 отличается | Phase 0 spike; fallback kwriteconfig6 / Meta+Space |
| Clipboard-marker race | Настраиваемые `copy_settle_ms`/`paste_settle_ms`; retry |
| «Кейлоггер»-восприятие `input` group | PRIVACY/SECURITY; portal fallback; opt-out буфера |
| Терминалы / full-screen TUI | Terminal mode + `terminal_erase`; TUI (vim/tmux) — не поддерживаем |
| VS Code/Electron selection quirks | M10; fallback word |
| Конфликт хоткеев с grabbers | Pass-through default |
| iced + ksni event loop | Отдельный процесс settings |
| Курсор не в конце слова | Word-path только «с конца»; selection — на месте |

**Открытые вопросы:**
1. Имя продукта (`switchfix` / `klavafix` / `layoutfix`).
2. Дефолт `privacy.typing_buffer`: on (удобно) vs off (строже) — предлагаю **on** + тумблер.
3. Mixed text: конвертить только «чужую» часть (по `detect_direction`).
4. Лицензия MIT vs Apache-2.0.
5. Single-instance lock (D-Bus name `org.switchfix.Daemon`).
6. Поддержка GlobalShortcuts portal кастомных chords (Ctrl+F1) на xdg-desktop-portal-kde — спайк.

---

## 11. Критические файлы (создать при реализации)

| Путь | Роль |
|------|------|
| `crates/switchfix-domain/src/conversion.rs` | LayoutMap, convert |
| `crates/switchfix-domain/src/buffer.rs` | TypingBuffer |
| `crates/switchfix-domain/src/state.rs` | CorrectionState |
| `crates/switchfix-app/src/ports.rs` | Traits |
| `crates/switchfix-app/src/services/correction.rs` | CorrectionService |
| `crates/switchfix-adapters/src/uinput_injector.rs` | TextInjector |
| `crates/switchfix-adapters/src/kde_layout.rs` | LayoutSwitcher |
| `crates/switchfix-adapters/src/wl_clipboard.rs` | ClipboardPort |
| `crates/switchfix-adapters/src/evdev_hotkey.rs` | HotkeySource |
| `crates/switchfix-daemon/src/{main,wiring,doctor}.rs` | CLI + DI |
| `crates/switchfix-ui/src/{tray,settings_app}.rs` | UI |
| `assets/maps/{en2ru,ru2en}.json` | Таблицы |
| `packaging/{deb,systemd,desktop}` | Упаковка |

---

## 12. Верификация

1. `cargo test` — unit + fake-port integration.
2. `cargo run -- daemon` на Kubuntu Wayland → tray icon.
3. `switchfix doctor` → input group, `/dev/uinput`, D-Bus status.
4. Ручная матрица M1–M14 в Kate / Firefox / Konsole / Telegram / VS Code.
5. `journalctl --user -u switchfix -f` — ошибки layout/uinput.
6. Clipboard restore: вставить до операции → после → исходное содержимое.

---

## Приложение A — Первый спринт

| # | Задача | Критерий |
|---|--------|----------|
| A1 | Workspace + crates | `cargo check` |
| A2 | ConversionTable EN↔RU + tests | `cargo test -p switchfix-domain` |
| A3 | TypingBuffer + tests | unit green |
| A4 | Spike D-Bus setLayout/getLayout | работает на Kubuntu |
| A5 | Spike uinput «привет» в Kate | текст появляется |
| A6 | Spike clipboard-marker | selection читается |
| A7–A8 | KdeLayoutSwitcher, UinputInjector | smoke |
| A9 | CorrectionService last word | integration fakes |
| A10 | `switchfix doctor` | статусы |
| A11–A12 | tray + settings заглушки | иконка, окно |

## Приложение B — Карта EN↔RU (QWERTY ↔ ЙЦУКЕН)

`q→й w→ц e→у r→к t→е y→н u→г i→ш o→щ p→з [→х ]→ъ a→ф s→ы d→в f→а g→п h→р j→о k→л l→д ;→ж '→э z→я x→ч c→с v→м b→и n→т m→ь ,→б .→ю /→. \`→ё` + Shift-варианты + `@→\" #→№ $→; ^→: &→?`. Обратная карта — reverse. `detect_direction`: `a-zA-Z` vs `а-яА-ЯёЁ`.

## Приложение C — config.toml (дефолт)

```toml
[general]
enabled_on_start = true
language = "ru"
tray_icon_style = "color"

[hotkeys]
convert_selection = "ctrl+f1"
toggle_enabled = "ctrl+pause"
use_portal_hotkeys = false

[layouts]
active = ["us", "ru"]

[behavior]
capture_mode = "auto"            # auto | selection | word
fallback_to_word_on_no_selection = true
replace_strategy = "auto"        # auto | retype | paste
restore_clipboard = true
copy_settle_ms = 400
paste_settle_ms = 100
terminal_paste_combo = "ctrl+shift+v"
terminal_erase = "line"          # line | backspaces
excluded_apps = ["keepassxc", "kwalletmanager5"]

[privacy]
typing_buffer = true
typing_buffer_max_keys = 256
log_text_content = false

[spell]
enabled = true
mode = "hint"                  # off | hint | hotkey | auto
languages = ["ru", "en"]
min_word_len = 3
completion_min_chars = 3
suggest_count = 3
bar_position = "bottom"
bar_timeout_ms = 4500
auto_fix_min_len = 4
sound = true
```
