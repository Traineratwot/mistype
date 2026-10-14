# SwitchFix

Linux tray utility that fixes text typed in the wrong keyboard layout (EN↔RU),
switches the system layout, and (in later phases) will add spelling fixes and
snippet expansion. Think **Punto Switcher / Keyboop** for **KDE Plasma 6 Wayland**.

## Status

**MVP in progress.** Domain + app + adapters compile; unit tests green.
UI tray/settings compile against ksni 0.2 / iced 0.13 but are not fully wired/tested yet.

| Layer | Crate | Status |
|-------|-------|--------|
| Domain (conversion, buffer, state) | `switchfix-domain` | ✅ tests green |
| Use-cases + ports | `switchfix-app` | ✅ tests green (fakes) |
| OS adapters (evdev/uinput/D-Bus/clipboard/config) | `switchfix-adapters` | ✅ compiles |
| Tray + settings GUI | `switchfix-ui` | ✅ compiles, smoke not done |
| CLI / daemon / doctor | `switchfix-daemon` | ✅ compiles |

## Build & test

```bash
cargo build --release
cargo test --workspace
./target/release/switchfix doctor
./target/release/switchfix convert --text 'ghbdtn'   # → привет
```

## Run (needs group `input`)

```bash
sudo usermod -aG input "$USER"   # then re-login
./target/release/switchfix daemon
./target/release/switchfix settings
```

Default hotkeys: `Ctrl+F1` selection · `Ctrl+F2` last word · `Ctrl+Shift+F1` terminal · `Ctrl+Pause` toggle.

## Architecture

```
ui / daemon  →  app (use-cases, ports=traits)  →  domain (pure)
                        ↑ implements
                   adapters (evdev, uinput, zbus, clipboard, toml)
```

See `docs/PLAN.md` for the full development plan (analogues, input strategies,
test matrix, roadmap).

## Privacy

No network, no keylogging persistence. Keystroke buffer is RAM-only and optional.
Membership in group `input` is required for global hotkeys / uinput — documented
risk, see `docs/PLAN.md` §9.
