# SwitchFix

Linux tray utility that fixes text typed in the wrong keyboard layout (EN↔RU),
switches the system layout, and (in later phases) will add spelling fixes and
snippet expansion. Think **Punto Switcher / Keyboop** for **KDE Plasma 6 Wayland**.

Primary target: **Kubuntu (Debian + KDE Plasma 6, Wayland)**. Other OS/DE are
stubs behind a platform layer (groundwork for the future).

---

## Status (MVP + Phase 2)

| Layer | Crate | Status |
|-------|-------|--------|
| Domain (conversion, buffer, state, spell heuristics) | `switchfix-domain` | ✅ tests green |
| Use-cases + ports | `switchfix-app` | ✅ tests green (fakes) |
| Platform layer + OS adapters | `switchfix-adapters` | ✅ tests green |
| Tray + settings GUI + suggestion bar | `switchfix-ui` | ✅ compiles, flash/sound + bar |
| CLI / daemon / doctor / selftest / spell | `switchfix-daemon` | ✅ works on live KDE |

**Tests: 119 green** (+ 1 ignored live KDE D-Bus test). Verified live:
hotkey intercept (evdev), layout switch (zbus), convert, tray flash + sound,
spellcheck CLI (zspell RU/EN + layout-convert candidates).

---

## Build & test

```bash
cargo build --release
cargo test --workspace          # must stay green (119)

./target/release/switchfix doctor     # diagnostics incl. platform + hotkeys + dictionaries
./target/release/switchfix selftest   # injects ctrl+f20 via uinput, verifies evdev sees it
./target/release/switchfix beep       # play hotkey feedback sound
./target/release/switchfix convert --text 'ghbdtn'   # → привет
./target/release/switchfix spell --text 'приветт'    # → 1. привет
./target/release/switchfix spell --text 'ghbdtn'     # → 1. привет (раскладка)

# live KDE D-Bus smoke (ignored by default):
cargo test -p switchfix-adapters -- --ignored
```

## Run (needs group `input` **in the session**)

```bash
sudo usermod -aG input "$USER"   # then RE-LOGIN (session must pick the group up)

# or without re-login for a one-off:
sg input -c './target/release/switchfix daemon'

./target/release/switchfix daemon
./target/release/switchfix settings
./target/release/switchfix setup      # interactive: usermod, udev rule (sudo only when needed)
```

**Hotkeys are configured in** `~/.config/switchfix/config.toml`. Defaults:

| Action | Default key |
|--------|-------------|
| Convert selection | `ctrl+f1` |
| Fix spelling of last word | `ctrl+f2` |
| Toggle enabled | `ctrl+pause` |
| Accept suggestion slot (while bar visible) | `ctrl+1/2/3` |
| Add word to dictionary (while bar visible) | `ctrl+0` |
| Dismiss suggestion bar | `ctrl+period` |

The live config may differ (e.g. `insert`) — always check `switchfix doctor`.
Click-to-capture in Settings UI; Esc cancels. After config change: **restart daemon**.

Hotkey feedback: tray icon flashes amber (~350 ms) + system sound + log line
`hotkey received: …`. Toggle via `hotkeys.hotkey_consume` (default `true`):
EVIOCGRAB + uinput passthrough so the key never leaks to apps.

---

## Architecture

```
ui / daemon  →  app (use-cases, ports=traits)  →  domain (pure)
                        ↑ implements
                   adapters / platform (evdev, uinput, zbus, clipboard, toml)
```

### Platform layer (OOP-style: Strategy + Abstract Factory)

`switchfix-adapters/src/platform/`:

```
platform/
  mod.rs          # Platform trait, PlatformInfo, PlatformServices, PlatformError
  detect.rs       # parse_desktop / parse_session / detect_os / detect()
  factory.rs      # create_platform() — cfg(target_os) + runtime DE
  stub.rs         # UnsupportedPlatform (fallback)
  portable/       # SharedTypingBuffer, ArboardClipboard, TomlConfigStore,
                  # ManualHotkeySource, LogNotifier
  linux/          # #[cfg(target_os = "linux")]
    evdev_hotkey.rs, uinput_injector.rs, selftest.rs
    kde/          # KdePlatform (full) + layout.rs (org.kde.keyboard)
    gnome/        # stub → Unsupported
  windows/, macos/  # stubs (cfg-gated)
```

- **One `Platform` per OS/DE**; `build_services()` returns all 8 ports.
- Selection: compile-time OS, runtime `XDG_CURRENT_DESKTOP` (KDE / GNOME / other).
- Only **linux/kde** is fully implemented. Stubs return `PlatformError::Unsupported`.
- Do **not** re-export `evdev::KeyCode` outside `platform/linux`.

### Ports (unified interface) — `switchfix-app/src/ports.rs`

`TextInjector`, `ClipboardPort`, `SelectionCapture`, `TypingBufferPort`,
`LayoutSwitcher`, `HotkeySource`, `ConfigStore`, `Notifier`.

Composition root: `switchfix-daemon/src/wiring.rs`
(`create_platform` → `build_services` → `CorrectionService`).

`CorrectionService::new` arg order:
`injector, clipboard, selection, buffer, layouts, notifier, tables, config_rx, enabled`.
Clipboard + selection must be **two Arc views of one** `ArboardClipboard`.

See `docs/PLAN.md` for analogues, input strategies, test matrix, roadmap.

---

## Development rules

1. **Ports stay abstract.** New OS/DE support = new `Platform` impl, not changes
   to domain/app/CorrectionService. The 8 traits are the contract.
2. **Domain is pure** — no I/O, no evdev/zbus, no `std::process`. Adapters own OS.
3. **Dependencies flow downward only:** `ui/daemon → app → domain`. UI must not
   depend on adapters (take `Box<dyn ConfigStore>` etc.).
4. **cfg-gate OS deps.** `evdev`, `zbus`, `futures-lite` live under
   `[target.'cfg(target_os = "linux")'.dependencies]`. Portable code must not
   import them.
5. **Tests must stay green.** `cargo test --workspace` after every change.
   Fakes in `switchfix-tests/tests/fakes/` implement ports — keep them compiling.
6. **No network, no keylog persistence.** Typing buffer is RAM-only and optional
   (`privacy.typing_buffer`). Never log typed text or clipboard contents.
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

1. Define trait + errors in `switchfix-app/src/ports.rs`.
2. Implement in `platform/portable` (shared) or per-OS module.
3. Add field to `PlatformServices` and fill in every `Platform::build_services`
   (stubs don't need to — they error).
4. Fake in `switchfix-tests/tests/fakes/`.
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
| **Never grab our own uinput** | Injected Ctrl+C/Ctrl+V are eaten by the evdev loop | Skip devices named `switchfix*` in `evdev_hotkey.rs`. |
| **Copy chord order** | Ctrl+C is SIGINT in Konsole | Capture tries Ctrl+Insert → Ctrl+Shift+C → Ctrl+C. Never lead with Ctrl+C. |
| **Last-word fallback on empty capture** | `backspace(n)` with an active selection wipes it | `ConvertSelection` with `Ok(None)` returns `NoInputFound`. No word path. |
| **`type_text` keymap is incomplete** | After `backspace`, a failed `type_text` leaves nothing | Word replace prefers clipboard paste (`replace_word_via_paste`); `type_text` is fallback only. |
| **Focus monitor is best-effort** | KWin `getWindowInfo` often returns `{}`; `queryWindowInfo` is interactive | `FocusMonitor` = xprop then KWin; `None` → generic GUI path, never block. |
| **`switchfix.capture-pause`** | Settings capture must not trigger real hotkeys | Daemon drains the hotkey queue while the file exists. Remove on capture end/exit. |
| **Empty hotkey = disabled** | Esc in Settings clears the binding | `validate()` accepts `""`; `parse_hotkey_bindings` skips empty. |
| **`HotkeySource::events()` is `take()`** | Second call returns an empty channel | Call once in the daemon loop. Never during `build_services`. |
| **Accept chords `Ctrl+1/2/3`** | Would steal app shortcuts if always on | evdev matches them **only while** `bar_visible` is true. |
| **Suggestion bar vs ksni** | Two iced/winit loops in one process clash | Bar is `switchfix suggest-bar` — separate process, file IPC (`switchfix.suggest.json` / `.cmd`). |
| **`key_name_to_code` returns `evdev::KeyCode`** | Linux type leak | Keep it private to `platform/linux`. |
| **SharedTypingBuffer ≠ domain `TypingBuffer`** | Duplicate concepts | Live one is adapters `SharedTypingBuffer`. Do not merge casually; `EvdevOptions.buffer` needs concrete type (`push`/`pop`). |
| **wl-clipboard-rs** | Was a dead dep (arboard is used) | Removed. `wl-copy`/`wl-paste` are invoked as CLI when present; arboard is the library fallback. |
| **`WaylandClipboard` was misnamed** | Actually arboard | Renamed `ArboardClipboard`. |
| **udev rule + group** | Needed for `/dev/uinput` on some systems | `switchfix setup` installs `99-switchfix-uinput.rules`. |
| **KDE layout set is async** | `setLayout` may not stick immediately | Poll `getLayout` briefly (see `layout.rs`). |
| **Mixed text `hello мир` → majority direction** | Unmapped chars unchanged | Expected domain behavior, not a bug. |
| **selftest keyboard `No such device` warnings** | Temporary uinput disappears after test | Harmless. |

---

## CLI surface (`switchfix`)

| Command | Purpose |
|---------|---------|
| `daemon` (default) | Tray + hotkeys + corrections + spellcheck |
| `settings` | iced settings window (separate process) |
| `convert [--text]` | Pure domain convert (no injection) |
| `spell [--text]` | Spellcheck a word; prints candidates (layout-convert + dict) |
| `suggest-bar` | Floating suggestion strip (spawned by daemon) |
| `doctor` | Diagnostics: platform, groups, uinput, D-Bus, **hotkeys**, sound, **dictionaries** |
| `setup [-y]` | Interactive privileged setup (sudo only for root steps) |
| `toggle` | Flip enabled/paused via temp state file |
| `beep` | Play hotkey feedback sound |
| `selftest` | Inject `ctrl+f20` via uinput, assert evdev intercepts it |

---

## Config (`~/.config/switchfix/config.toml`)

`AppConfig` in `switchfix-app/src/config.rs` (`Default` + `validate()`):

- `general.enabled_on_start`, `language`
- `hotkeys.convert_selection / toggle_enabled / fix_spelling`
  — grammar `ctrl+shift+f1`, `insert`, `capslock`, `home`, … (`HotkeyBinding::parse`)
  — **empty string = action disabled** (Esc in Settings resets the field)
- `hotkeys.hotkey_consume` (default **true**) — EVIOCGRAB + passthrough
- `hotkeys.use_portal_hotkeys` — **flag only, not implemented**
- `behavior.copy_settle_ms`, `behavior.paste_settle_ms`
- `behavior.terminal_apps` — app ids that use terminal paste (auto, no separate hotkey)
- `behavior.excluded_apps` — never correct here (keepassxc, …)
- `behavior.disable_in_fullscreen` (default **true**) — skip games / fullscreen video
- `privacy.typing_buffer`, `privacy.typing_buffer_max_keys`
- `layouts.active` — KDE short ids `["us","ru"]`
- `spell.enabled`, `spell.mode` (`off|hint|hotkey|auto`)
- `spell.languages` (`["ru","en"]`), `spell.min_word_len`, `spell.completion_min_chars`
- `spell.suggest_count`, `spell.bar_position`, `spell.bar_timeout_ms`
- `spell.auto_fix_min_len`, `spell.sound` (short signal on word replace)

Settings UI: click field → press combo → saved; Esc **clears** the binding;
checkboxes Ctrl/Alt/Shift/Meta force modifiers; while capturing a
`switchfix.capture-pause` file is written and the daemon ignores hotkeys.

---

## Testing strategy

| Kind | Where | Notes |
|------|-------|-------|
| Domain unit | `switchfix-domain` | conversion, buffer, maps, spell heuristics |
| App unit | `switchfix-app` | hotkey parse, config validate, correction, **spell service** (inline fakes) |
| Adapter unit | `switchfix-adapters` | config_store tempfile, detect/factory, parse_hotkey_bindings, **zspell** |
| Integration order | `switchfix-tests/tests/scenarios.rs` | 10 order-sensitive scenarios + shared event-log fakes |
| UI unit | `switchfix-ui` | combo formatting |
| Live (ignored) | `kde/layout.rs` | `live_kde_layout_roundtrip` |
| Live CLI | `switchfix selftest` | uinput inject → evdev match |

**Manual matrix:** `docs/MANUAL-TEST-MATRIX.md` (M1–M22) on Kate / Firefox /
Konsole / Telegram / VS Code. Include hotkey consume leak-check and spell bar.

**Simulating keys (for intercept tests):**
```bash
ydotool key 110:1 110:0          # Insert down/up (if insert is bound)
ydotool key --key-delay=60 29:1 59:1 59:0 29:0   # Ctrl+F1
# KEY_* codes: /usr/include/linux/input-event-codes.h
```
Watch `RUST_LOG=debug` for `evdev key …` / `hotkey match` / `hotkey received`.

---

## Packaging

- `packaging/systemd/switchfix.service` — user unit (do **not** enable on install)
- `packaging/systemd/99-switchfix-uinput.rules` — uinput group/mode
- `packaging/desktop/switchfix.desktop` — KDE autostart bits
- `packaging/install.sh`, `packaging/build-deb.sh` → `target/switchfix_0.1.0_amd64.deb`
- `assets/maps/en2ru.json`, `ru2en.json`, `assets/icons/switchfix.svg`

Docs: `docs/PLAN.md`, `PRIVACY.md`, `SECURITY.md`, `MANUAL-TEST-MATRIX.md`.

---

## FAQ

**Q: Hotkeys do nothing / no log / no flash.**  
A: (1) `switchfix doctor` → `open keyboards` must be OK. If FAIL, session lacks
group `input` — re-login or `sg input -c switchfix daemon`. (2) Keys must match
`doctor` → `hotkeys` (config), not the docs default. (3) `switchfix selftest` to
prove the intercept path. (4) `RUST_LOG=debug` daemon → look for `hotkey received`.

**Q: Doctor says “log out and back in for this session”.**  
A: `usermod` updated the passwd DB only. The current session token doesn't have
`input`. Re-login, or start with `sg input -c …`.

**Q: Keyboard died / no input after starting daemon.**  
A: `hotkey_consume` grabbed devices. Passthrough should re-inject non-hotkeys.
If passthrough uinput failed, grab is skipped (by design). Kill daemon to restore.
Run `switchfix setup` if `/dev/uinput` is not writable.

**Q: Settings window crashes / black window on NVIDIA.**  
A: Hybrid GPU + Wayland wgpu panic. We force `WGPU_BACKEND=gl`. Don't remove.

**Q: I changed a hotkey in Settings but it still uses the old one.**  
A: MVP reads bindings at daemon start. **Restart the daemon.**

**Q: Spellcheck does nothing / no bar.**  
A: (1) `switchfix doctor` → `spell dictionaries` must show `loaded [ru, en]`.
If FAIL: `sudo apt install hunspell-ru hunspell-en-us`. (2) `spell.mode` must not
be `off`. (3) `switchfix spell --text 'приветт'` proves the dictionary path.
(4) Bar is a separate process — `pgrep -af suggest-bar`.

**Q: `ctrl+f1` doesn't trigger but `insert` does.**  
A: Your `config.toml` has `convert_selection = "insert"`. `doctor` prints the
live bindings. Defaults ≠ current config.

**Q: How do I know the program received my keypress?**  
A: Tray icon flashes amber, sound plays, log shows `hotkey received: …`.
If the correction itself fails (e.g. empty selection), you still get the flash —
that proves intercept works.

**Q: `selftest` FAIL on `hotkey intercept`.**  
A: Without group `input`, it fails at `open event*` first. With the group:
timeout means the injected device wasn't tracked — we create uinput, sleep 200 ms,
then open evdev (hotplug race). If still failing, check `RUST_LOG=debug` for
`evdev device:` lines including `switchfix-selftest-kb`.

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
`platform/{portable,linux,windows,macos}`. UI → `switchfix-ui` via ports only.

---

## Privacy

No network, no keylogging persistence. Keystroke buffer is RAM-only and optional.
Group `input` is required for global hotkeys / uinput — documented risk,
see `docs/PLAN.md` §9, `docs/PRIVACY.md`, `docs/SECURITY.md`.
