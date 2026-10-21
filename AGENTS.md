# Mistype

Linux tray utility that fixes text typed in the wrong keyboard layout (EN↔RU),
switches the system layout, and LLM-fixes a selected text (spelling / grammar /
punctuation) via an external OpenAI-compatible API. Think **Punto Switcher /
Keyboop** for **KDE Plasma 6 Wayland**.

Primary target: **Kubuntu (Debian + KDE Plasma 6, Wayland)**. Other OS/DE are
stubs behind a platform layer (groundwork for the future).

---

## Status (MVP + LLM-fix)

| Layer | Crate | Status |
|-------|-------|--------|
| Domain (conversion, buffer, state, `detect_lang`) | `mistype-domain` | ✅ tests green |
| Use-cases + ports (incl. `TextCorrector`, undo) | `mistype-app` | ✅ tests green (fakes) |
| Platform layer + OS adapters (incl. `OpenAiCorrector`) | `mistype-adapters` | ✅ tests green |
| Tray + settings GUI | `mistype-ui` | ✅ compiles, flash/sound + busy blink |
| CLI / daemon / doctor / selftest | `mistype-daemon` | ✅ works on live KDE |

**Tests: 128 green** (+ 1 ignored live KDE D-Bus test). Verified live:
hotkey intercept (evdev), layout switch (zbus), convert, tray flash + sound.

Removed in the strip-down (do not resurrect): built-in LLM crate
(`mistype-llm` / llama.cpp), spell-as-you-type, suggestion bar, completions,
zspell / LanguageTool / `llm_cmd` correctors, Plasma plasmoid
(`org.mistype.suggestions`, `docs/PLASMA6-WIDGETS.md` — deleted).

---

## Build & test

```bash
cargo build --release
cargo test --workspace          # must stay green (128)

./target/release/mistype doctor     # diagnostics incl. platform + hotkeys + llm api
./target/release/mistype selftest   # injects ctrl+f20 via uinput, verifies evdev sees it
./target/release/mistype beep       # play hotkey feedback sound
./target/release/mistype convert --text 'ghbdtn'   # → привет

# live KDE D-Bus smoke (ignored by default):
cargo test -p mistype-adapters -- --ignored
```

## Run (needs group `input` **in the session**)

```bash
sudo usermod -aG input "$USER"   # then RE-LOGIN (session must pick the group up)

# or without re-login for a one-off:
sg input -c './target/release/mistype daemon'

./target/release/mistype daemon
./target/release/mistype settings
./target/release/mistype setup      # interactive: usermod, udev rule (sudo only when needed)
```

**Hotkeys are configured in** `~/.config/mistype/config.toml`. Defaults:

| Action | Default key |
|--------|-------------|
| Convert selection (EN↔RU flip) | `ctrl+alt+r` |
| LLM-fix selection (whole selection → LLM → replace) | `alt+r` |
| Toggle enabled | `ctrl+alt+t` |
| Undo last LLM replace | **1× Backspace within ~4 s** (not configurable) |

The live config may differ — always check `mistype doctor`.
Click-to-capture in Settings UI; Esc clears the binding. After config change:
**Save restarts the daemon automatically** (`$TMPDIR/mistype.restart` →
daemon re-execs itself, same PID, so systemd/user service stays intact).

**Backspace undo (LLM-fix only):** after a successful LLM replace, one physical
Backspace within **4 s** restores the original text fully (the replace is
torn down: backspaces + retypes the original). If the user types anything else
first, the undo is cancelled. Undo works even with `privacy.typing_buffer=false`
(the Backspace event is ungated). After the window expires a Backspace is just
a Backspace.

Hotkey feedback: tray icon flashes amber (~350 ms) + system sound + log line
`hotkey received: …`. During an LLM call the tray icon blinks **yellow** (busy)
until the reply arrives. Toggle via `hotkeys.hotkey_consume` (default `true`):
EVIOCGRAB + uinput passthrough so the key never leaks to apps.

---

## Architecture

```
ui / daemon  →  app (use-cases, ports=traits)  →  domain (pure)
                        ↑ implements
                   adapters / platform (evdev, uinput, zbus, clipboard, toml, ureq)
```

### Platform layer (OOP-style: Strategy + Abstract Factory)

`mistype-adapters/src/platform/`:

```
platform/
  mod.rs          # Platform trait, PlatformInfo, PlatformServices, PlatformError
  detect.rs       # parse_desktop / parse_session / detect_os / detect()
  factory.rs      # create_platform() — cfg(target_os) + runtime DE
  stub.rs         # UnsupportedPlatform (fallback)
  portable/       # SharedTypingBuffer, ArboardClipboard, TomlConfigStore,
                  # ManualHotkeySource, LogNotifier, OpenAiCorrector
  linux/          # #[cfg(target_os = "linux")]
    evdev_hotkey.rs, uinput_injector.rs, selftest.rs
    kde/          # KdePlatform (full) + layout.rs (org.kde.keyboard) + focus.rs
    gnome/        # stub → Unsupported
  windows/, macos/  # stubs (cfg-gated)
```

- **One `Platform` per OS/DE**; `build_services()` returns the 8 core ports
  (+ optional `FocusMonitor`).
- Selection: compile-time OS, runtime `XDG_CURRENT_DESKTOP` (KDE / GNOME / other).
- Only **linux/kde** is fully implemented. Stubs return `PlatformError::Unsupported`.
- Do **not** re-export `evdev::KeyCode` outside `platform/linux`.

### Ports (unified interface) — `mistype-app/src/ports.rs`

`TextInjector`, `ClipboardPort`, `ClipboardRead`, `SelectionCapture`,
`TypingBufferPort`, `TextCorrector`, `LlmProbe`, `CorrectorFactory`,
`LayoutSwitcher`, `HotkeySource`, `ConfigStore`, `Notifier`,
`SoundPort`, `LlmHistoryStore`, `HeartbeatPort`, `UpdateChecker`,
`FocusMonitor` (optional).

`TextCorrector::correct_text(&str) -> String` replaces the old
ContextCorrector/SpellChecker. The only implementation is `OpenAiCorrector`
(`platform/portable/openai_corrector.rs`, ureq + `spawn_blocking`). It is
**not** part of `PlatformServices` — it is built/rebuilt by
`wiring::install_text_corrector` from `cfg.llm` at startup **and on config
reload**.

Composition root: `mistype-daemon/src/wiring.rs`
(`create_platform` → `build_services` → `CorrectionService` →
`install_text_corrector`).

`CorrectionService::new` arg order:
`injector, clipboard, selection, buffer, layouts, notifier, tables, config_rx, enabled`.
Clipboard + selection must be **two Arc views of one** `ArboardClipboard`.
Undo state lives in `CorrectionService` (`UndoState`, `on_backspace`,
`on_typing`, `clear_undo`) — 4 s window after an LLM replace.

See `docs/PLAN.md` for analogues, input strategies, test matrix, roadmap.

---

## Development rules

1. **Ports stay abstract.** New OS/DE support = new `Platform` impl, not changes
   to domain/app/CorrectionService. The traits in `ports.rs` are the contract.
2. **Domain is pure** — no I/O, no evdev/zbus, no `std::process`, no network.
   Adapters own OS. `detect_lang` is the only spell-era helper left
   (`domain/lang.rs`).
3. **Dependencies flow downward only:** `ui/daemon → app → domain`. UI must not
   depend on adapters (take `Box<dyn ConfigStore>` etc.).
4. **cfg-gate OS deps.** `evdev`, `zbus`, `futures-lite` live under
   `[target.'cfg(target_os = "linux")'.dependencies]`. Portable code must not
   import them.
5. **Tests must stay green.** `cargo test --workspace` after every change.
   Fakes in `mistype-tests/tests/fakes/` implement ports — keep them compiling.
6. **No network except the user-configured LLM endpoint.** No telemetry, no
   keylog persistence. Typing buffer is RAM-only and optional
   (`privacy.typing_buffer`). Never log typed text, clipboard contents, or the
   LLM `api_token`.
7. **Do not auto-enable systemd user service on install** (PLAN §8).
8. **User-facing error texts stay actionable** (e.g. `Run: sudo usermod -aG input $USER`,
   `Are you on KDE Plasma?`). Don't flatten to generic strings.
9. **UI strings are Russian**, code/identifiers English.
10. **Don't rename port error variants** (`InjectError::Device`, …) without a
    functional reason — it churns fakes for zero gain.
11. **EVIOCGRAB safety:** always open the passthrough uinput **before** grabbing.
    If passthrough fails, never grab (or the keyboard becomes unusable).
12. **Prefer boring, compile-safe Rust.** No speculative abstractions. Three
    similar lines beat a premature helper.
13. **Comments only for non-obvious WHY** (hidden constraints, kernel quirks).

### Adding a new platform (checklist)

1. `platform/<os>/mod.rs` or `platform/linux/<de>/mod.rs` implementing `Platform`.
2. `detect.rs`: teach `parse_desktop` / `OsKind` if needed.
3. `factory.rs`: wire `create_platform_from`.
4. Stub must return `PlatformError::Unsupported` with a helpful reason until real.
5. Target-specific Cargo deps if the OS needs them.
6. Tests for detect/factory selection (pure functions).
7. Never leak OS types (`evdev::KeyCode`, `HWND`, …) through `PlatformServices`.

### Adding a port / capability

1. Define trait + errors in `mistype-app/src/ports.rs`.
2. Implement in `platform/portable` (shared) or per-OS module.
3. Add field to `PlatformServices` and fill in every `Platform::build_services`
   (stubs don't need to — they error).
4. Fake in `mistype-tests/tests/fakes/`.
5. Wire into `CorrectionService` / daemon only through the trait.

---

## Landmines (грабли) — read before touching input/platform

| Pitfall | What happens | Fix / rule |
|---------|--------------|------------|
| **Blocking `fetch_events()`** | First keyboard starves the others; hotkeys look “dead” | Must `set_nonblocking(true)` on all devices and skip `WouldBlock`. See `evdev_hotkey.rs::run_loop`. |
| **Session vs passwd groups** | `usermod -aG input` is invisible until re-login. `id -nG` ≠ `id -nG "$USER"` | Doctor checks both. Run daemon with `sg input -c …` until re-login. |
| **`/dev/input/event*` needs group `input`** | evdev open fails → silent fallback to `ManualHotkeySource` | Error is loud (`tracing::error!`). `selftest` / doctor `open keyboards` catch this. |
| **`/dev/uinput` often writable via ACL** even without `input` | uinput works, evdev doesn't | Don't use uinput success as proof that hotkeys will work. |
| **Plasma 6 D-Bus is camelCase** | zbus `#[proxy]` PascalCases snake_case names (`GetLayout` vs `getLayout`) | Always `#[zbus(name = "getLayout")]` etc. in `kde/layout.rs`. |
| **`WGPU_BACKEND=gl`** | Hybrid NVIDIA + Wayland: iced/wgpu panics | `run_settings` forces `WGPU_BACKEND=gl` if unset. Keep this. |
| **iced `on_key_press` only fires for `Status::Ignored`** | Hotkey capture UI swallows keys if focused widget handles them | Capture targets use `mouse_area`, not `button`/`text_input`. |
| **EVIOCGRAB without passthrough** | Keyboard becomes unusable system-wide | Open passthrough first; abort grab on failure. |
| **Clipboard: one instance, two trait views** | Two clipboards break marker-protocol `last_save` | `let c = Arc::new(ArboardClipboard…); clipboard: c.clone(), selection: c`. |
| **Second uinput for clipboard chords** | Selection capture injects Ctrl+Insert | Keep a separate `UinputInjector` (current behavior). Don't “optimize” to one without retesting M-matrix. |
| **arboard `set_text` drops the offer** | Wayland/X11 clipboard is hosted by the owner process; dropping `Clipboard` (or `wl-clipboard-rs` fork from a MT daemon) makes Ctrl+V paste **empty** and **deletes the selection** | Prefer `wl-copy` (daemonizes a child). Keep an arboard owner thread. Always `get_text()`-verify before Ctrl+V; if missing, `type_text` over the selection. |
| **Never grab our own uinput** | Injected Ctrl+C/Ctrl+V are eaten by the evdev loop | Skip devices named `mistype*` in `evdev_hotkey.rs`. |
| **Copy chord order** | Ctrl+C is SIGINT in Konsole | Capture tries Ctrl+Insert → Ctrl+Shift+C → Ctrl+C. Never lead with Ctrl+C. |
| **Last-word fallback on empty capture** | `backspace(n)` with an active selection wipes it | `ConvertSelection` with `Ok(None)` returns `NoInputFound`. No word path. |
| **`type_text` keymap is incomplete** | After `backspace`, a failed `type_text` leaves nothing | Word replace prefers clipboard paste (`replace_word_via_paste`); `type_text` is fallback only. |
| **Focus monitor is best-effort** | KWin `getWindowInfo` often returns `{}`; `queryWindowInfo` is interactive | `FocusMonitor` = xprop then KWin; `None` → generic GUI path, never block. |
| **`mistype.capture-pause`** | Settings capture must not trigger real hotkeys | Daemon drains the hotkey queue while the file exists. Remove on capture end/exit. |
| **Empty hotkey = disabled** | Esc in Settings clears the binding | `validate()` accepts `""`; `parse_hotkey_bindings` skips empty. |
| **`HotkeySource::events()` is `take()`** | Second call returns an empty channel | Call once in the daemon loop. Never during `build_services`. |
| **`key_to_char` maps US physical keys** | Buffer stored Latin (`ghbdtn`) while user typed Russian | Map through `LayoutCharMapper` (current KDE layout + `en_to_ru_map`). |
| **`key_name_to_code` returns `evdev::KeyCode`** | Linux type leak | Keep it private to `platform/linux`. |
| **SharedTypingBuffer ≠ domain `TypingBuffer`** | Duplicate concepts | Live one is adapters `SharedTypingBuffer`. Do not merge casually; `EvdevOptions.buffer` needs concrete type (`push`/`pop`). |
| **wl-clipboard-rs** | Was a dead dep (arboard is used) | Removed. `wl-copy`/`wl-paste` are invoked as CLI when present; arboard is the library fallback. |
| **`WaylandClipboard` was misnamed** | Actually arboard | Renamed `ArboardClipboard`. |
| **udev rule + group** | Needed for `/dev/uinput` on some systems | `mistype setup` installs `99-mistype-uinput.rules`. |
| **KDE layout set is async** | `setLayout` may not stick immediately | Poll `getLayout` briefly (see `layout.rs`). |
| **Mixed text `hello мир` → majority direction** | Unmapped chars unchanged | Expected domain behavior, not a bug. |
| **selftest keyboard `No such device` warnings** | Temporary uinput disappears after test | Harmless. |
| **`ureq` is blocking** | A synchronous HTTP call inside the async runtime stalls the daemon (no hotkeys, no tray) | `OpenAiCorrector` must call ureq only inside `spawn_blocking`. Never `.send()` on the async thread. |
| **systemd `RestrictAddressFamilies`** | LLM API is TCP; the old unit allowed only `AF_UNIX AF_NETLINK` → connection fails under systemd but works from a shell | Unit must keep `AF_INET AF_INET6` (see `packaging/systemd/mistype.service`). |
| **`OpenAiCorrector` is rebuilt on reload** | A held `Arc<dyn TextCorrector>` keeps the old `base_url`/`system_prompt`/token forever | `wiring::install_text_corrector` rebuilds from `cfg.llm` at startup **and on config reload**. Go through `service.set_text_corrector`, don't cache. |
| **`WordEvent::Backspace` is ungated** | Undo must work even with `privacy.typing_buffer=false` | `SharedTypingBuffer::pop` emits Backspace regardless of the privacy flag. Don't gate it — `CorrectionService::on_backspace` depends on it. |
| **LLM token in logs** | `api_token` is a credential | Never log `llm.api_token` (doctor prints `configured/not configured` only). |
| **`detect_lang` location** | Easy to re-grow spell heuristics in domain | Only `detect_lang` remains (`domain/lang.rs`); used for undo retyping layout. Do not reintroduce spell logic here. |

---

## CLI surface (`mistype`)

| Command | Purpose |
|---------|---------|
| `daemon` (default) | Tray + hotkeys + convert + LLM-fix |
| `settings` | iced settings window (separate process) |
| `convert [--text]` | Pure domain convert (no injection) |
| `doctor` | Diagnostics: platform, groups, uinput, D-Bus, **hotkeys**, sound, **llm api** |
| `setup [-y]` | Interactive privileged setup (sudo only for root steps) |
| `toggle` | Flip enabled/paused via temp state file |
| `reload` | Signal a running daemon to re-read `config.toml` + `maps/` |
| `beep` | Play hotkey feedback sound |
| `selftest` | Inject `ctrl+f20` via uinput, assert evdev intercepts it |

---

## Config (`~/.config/mistype/config.toml`)

`AppConfig` in `mistype-app/src/config.rs` (`Default` + `validate()`):

- `general.enabled_on_start`, `language`
- `hotkeys.convert_selection / toggle_enabled / llm_fix_selection`
  — grammar `ctrl+shift+f1`, `insert`, `capslock`, `home`, … (`HotkeyBinding::parse`)
  — **empty string = action disabled** (Esc in Settings clears the field)
  — `llm_fix_selection` (default `alt+r`) sends the whole selection to the LLM
    and replaces it (see `[llm]`).
- `hotkeys.hotkey_consume` (default **true**) — EVIOCGRAB + passthrough
- `hotkeys.use_portal_hotkeys` — **flag only, not implemented**
- `behavior.restore_clipboard`, `behavior.copy_settle_ms`, `behavior.paste_settle_ms`
- `behavior.terminal_erase`
  — terminal mode: Ctrl+A / Ctrl+K / paste combo, no separate hotkey
- `behavior.terminal_apps` — app ids that use terminal paste (auto)
- `behavior.excluded_apps` — never correct here (keepassxc, …)
- `behavior.disable_in_fullscreen` (default **true**) — skip games / fullscreen video
- `privacy.typing_buffer`, `privacy.typing_buffer_max_keys`
- `layouts.active` — KDE short ids `["us","ru"]`
- `[llm]` — LLM-fix selection:
  - `enabled` (default `true`) — master switch for the hotkey
  - `base_url` — OpenAI-compatible server root (e.g. `https://api.openai.com`,
    `http://127.0.0.1:11434` for Ollama). Client POSTs
    `{base_url}/v1/chat/completions`. **Empty = feature off.**
  - `api_token` — Bearer token; empty = no `Authorization` header (local servers)
  - `system_prompt` — first system message (default in Russian: fix
    orthography/grammar/punctuation, return only corrected text)
  - `model` — optional model id (`gpt-4o-mini`, `qwen2.5:0.5b`, …);
    empty = omit the `model` field
  - `timeout_ms` — default `15000`, valid `500..=120000`

Old `[spell]` tables in existing configs are **ignored** (unknown section).
`llm.base_url` without scheme is rejected by `validate()`.

Settings UI: click field → press combo → saved; Esc **clears** the binding;
checkboxes Ctrl/Alt/Shift/Meta force modifiers; while capturing a
`mistype.capture-pause` file is written and the daemon ignores hotkeys.

Footer actions:
- **«Сохранить»** — writes `config.toml` and signals `$TMPDIR/mistype.restart`;
  the running daemon re-execs itself so hotkeys/consume rebind. If no daemon is
  running, the flag is ignored (start `mistype daemon` to pick the config up).
- **«Редактировать конфиг»** — `xdg-open ~/.config/mistype/` (`config.toml` + `maps/`).
- **«Перезагрузить конфиг»** — re-reads the form from disk and writes
  `$TMPDIR/mistype.reload`; the daemon applies `config` + maps (+ rebuilds the
  `OpenAiCorrector`) within ~2 s. **Hotkeys still require Save (or a daemon
  restart).**

### Layout maps (`~/.config/mistype/maps/`)

User-editable `en2ru.json` / `ru2en.json` (`MapFile` schema: `name`, `from`,
`to`, `map`). Seeded once when the directory has no `*.json`; edits are never
overwritten; deleted files are not restored. Invalid JSON is skipped with a
warning; if nothing loads, the built-in EN↔RU table is used.

Helpers: `mistype_adapters::{seed_default_maps, load_conversion_set}` in
`platform/portable/maps.rs`. `CorrectionService::replace_tables` swaps tables
on reload.

---

## Testing strategy

| Kind | Where | Notes |
|------|-------|-------|
| Domain unit | `mistype-domain` | conversion, buffer, maps, `detect_lang` |
| App unit | `mistype-app` | hotkey parse, config validate, correction, **LLM undo** (`llm_undo_*` in `correction.rs`) |
| Adapter unit | `mistype-adapters` | config_store tempfile, detect/factory, parse_hotkey_bindings, `OpenAiCorrector` |
| Integration order | `mistype-tests/tests/scenarios.rs` | order-sensitive scenarios + shared event-log fakes |
| UI unit | `mistype-ui` | combo formatting |
| Live (ignored) | `kde/layout.rs` | `live_kde_layout_roundtrip` |
| Live CLI | `mistype selftest` | uinput inject → evdev match |

**Manual matrix:** `docs/MANUAL-TEST-MATRIX.md` (convert M1–M14 + LLM-fix /
undo rows) on Kate / Firefox / Konsole / Telegram / VS Code. Include hotkey
consume leak-check.

**Simulating keys (for intercept tests):**
```bash
ydotool key 110:1 110:0          # Insert down/up (if insert is bound)
ydotool key --key-delay=60 29:1 59:1 59:0 29:0   # Ctrl+F1
# KEY_* codes: /usr/include/linux/input-event-codes.h
```
Watch `RUST_LOG=debug` for `evdev key …` / `hotkey match` / `hotkey received`.

---

## Packaging

- `packaging/systemd/mistype.service` — user unit (do **not** enable on
  install). `RestrictAddressFamilies=AF_UNIX AF_NETLINK AF_INET AF_INET6`
  (TCP is required for the optional LLM API). **`PrivateTmp=no`** — daemon IPC
  lives in `/tmp` (`llm-history.json`, toggle/reload/restart flags, heartbeat)
  and must stay visible to `mistype` CLI + the Plasma widget.
- `packaging/systemd/99-mistype-uinput.rules` — uinput group/mode
- `packaging/desktop/mistype.desktop` — KDE autostart bits
- `packaging/plasmoid/org.mistype.llm` — Plasma widget «Mistype LLM»
  (request history + copy; installed to `~/.local/share/plasma/plasmoids/`).
  No suggest-bar / dictionary UI.
- `packaging/install.sh`, `packaging/build-deb.sh` → `target/mistype_0.1.0_amd64.deb`
- `assets/maps/en2ru.json`, `ru2en.json`, `assets/icons/mistype.svg`

DEBIAN control has **no hunspell recommends** (dictionaries are gone).
Docs: `docs/PLAN.md`, `ALGORITHMS.md`, `PRIVACY.md`, `SECURITY.md`, `MANUAL-TEST-MATRIX.md`.

---

## FAQ

**Q: Hotkeys do nothing / no log / no flash.**  
A: (1) `mistype doctor` → `open keyboards` must be OK. If FAIL, session lacks
group `input` — re-login or `sg input -c mistype daemon`. (2) Keys must match
`doctor` → `hotkeys` (config), not the docs default. (3) `mistype selftest` to
prove the intercept path. (4) `RUST_LOG=debug` daemon → look for `hotkey received`.

**Q: Doctor says “log out and back in for this session”.**  
A: `usermod` updated the passwd DB only. The current session token doesn't have
`input`. Re-login, or start with `sg input -c …`.

**Q: How do I know my version / is there an update?**  
A: `mistype --version` prints the current version. `mistype doctor` shows the
version in the header and an `update` line (`up to date` / `update available` /
`check skipped` when offline). The check is one-shot against GitHub Releases
and only runs when you invoke `doctor`.

**Q: Keyboard died / no input after starting daemon.**  
A: `hotkey_consume` grabbed devices. Passthrough should re-inject non-hotkeys.
If passthrough uinput failed, grab is skipped (by design). Kill daemon to restore.
Run `mistype setup` if `/dev/uinput` is not writable.

**Q: Settings window crashes / black window on NVIDIA.**  
A: Hybrid GPU + Wayland wgpu panic. We force `WGPU_BACKEND=gl`. Don't remove.

**Q: I changed a hotkey in Settings but it still uses the old one.**  
A: Press «Сохранить» — the daemon re-execs itself and rebinds hotkeys
(~0.5 s). If the daemon wasn't running, start it. «Перезагрузить конфиг»
only soft-reloads maps/config (and the LLM corrector), not hotkeys.

**Q: How do I change the layout map / terminal app list?**  
A: Settings → «Редактировать конфиг» → edit `maps/en2ru.json` (MapFile schema)
or `behavior.terminal_apps` in `config.toml` → «Перезагрузить конфиг»
(or `mistype reload`). Maps + `terminal_apps` apply without restart;
hotkeys need «Сохранить» (daemon auto-restarts). Save in Settings
**overwrites** `config.toml` — prefer hand-edits then Reload, don't Save
afterwards unless you meant to.

**Q: Reload button does nothing.**  
A: Daemon polls `$TMPDIR/mistype.reload` every ~2 s. Check
`RUST_LOG=info` for `config reloaded` / `layout maps reloaded` /
`text corrector rebuilt`. No daemon running = only the Settings form is refreshed.

**Q: LLM-fix does nothing / “LLM-исправление недоступно”.**  
A: (1) `mistype doctor` → `llm api` must show `configured`. If
`not configured`, set `[llm] base_url` (and `api_token` if the server needs one)
— empty `base_url` turns the feature off. (2) `llm.enabled` must be `true`.
(3) Empty selection = no network call and no replace (by design). (4) Errors
(`bad token`, unreachable host) notify «LLM не ответил — текст не изменён» and
leave the text unchanged.

**Q: LLM-fix works in a shell but fails under systemd.**  
A: The user unit sandboxes address families. `RestrictAddressFamilies` must
include `AF_INET AF_INET6` for the LLM TCP call — see
`packaging/systemd/mistype.service`.

**Q: I replaced text with the LLM and want it back.**  
A: Press **Backspace once within ~4 s** — the whole replace is undone and the
original text is restored. If you type anything else first, the undo is
cancelled; after ~4 s Backspace is a normal Backspace. Undo works even with
`privacy.typing_buffer=false`.

**Q: How do I point the LLM at a local server / different model?**  
A: `[llm] base_url` is the server root — the client POSTs
`{base_url}/v1/chat/completions` (works with Ollama at
`http://127.0.0.1:11434`, llama.cpp server, OpenAI, …). Set `model` to the id
your server expects (empty = omit the field) and `system_prompt` to taste.
«Перезагрузить конфиг» (or Save) rebuilds the corrector — no daemon restart
needed for these fields.

**Q: `ctrl+alt+r` doesn't trigger but `insert` does.**  
A: Your `config.toml` has a custom `convert_selection`. `doctor` prints the
live bindings. Defaults ≠ current config.

**Q: How do I know the program received my keypress?**  
A: Tray icon flashes amber, sound plays, log shows `hotkey received: …`.
During an LLM call the icon blinks yellow. If the correction itself fails
(e.g. empty selection), you still get the flash — that proves intercept works.

**Q: `selftest` FAIL on `hotkey intercept`.**  
A: Without group `input`, it fails at `open event*` first. With the group:
timeout means the injected device wasn't tracked — we create uinput, sleep 200 ms,
then open evdev (hotplug race). If still failing, check `RUST_LOG=debug` for
`evdev device:` lines including `mistype-selftest-kb`.

**Q: How to add GNOME support?**  
A: Implement `Platform` in `platform/linux/gnome/` (gsettings / XDG portal).
Wire `linux::select`. Don't touch domain/app. Keep KDE path green.

**Q: How to add Windows/macOS?**  
A: `platform/windows` / `platform/macos` stubs exist. Real work needs
SendInput/ImmGetLayout (Win) or CGEvent/TIS (macOS) + a layout switcher + tray.
Cross-check with `cargo check --target …`. UI (ksni/iced) is still Linux-oriented.

**Q: Can I use portal hotkeys (`use_portal_hotkeys`)?**  
A: Not implemented. Flag is reserved. evdev is the only HotkeySource.

**Q: What's the difference between `SharedTypingBuffer` and domain `TypingBuffer`?**  
A: Adapters `SharedTypingBuffer` is what runs (implements `TypingBufferPort`,
used by evdev). Domain buffer is a pure helper. Unifying them is a later cleanup.

**Q: Where should a new feature go?**  
A: Pure logic → domain. Orchestration → app service + ports. OS calls →
`platform/{portable,linux,windows,macos}`. UI → `mistype-ui` via ports only.

---

## Privacy

No telemetry, no keylogging persistence. Keystroke buffer is RAM-only and
optional. The only network traffic is the **user-configured LLM endpoint**
(`llm.base_url`): on the `llm_fix_selection` hotkey the current selection is
sent to that endpoint and nothing else. Empty `base_url` = no network at all.
**Exception:** `mistype doctor` performs a one-shot update check against
GitHub Releases (`api.github.com/repos/Traineratwot/mistype`) — explicit user
action, no telemetry, never automatic.
Group `input` is required for global hotkeys / uinput — documented risk,
see `docs/PLAN.md` §9, `docs/PRIVACY.md`, `docs/SECURITY.md`.
