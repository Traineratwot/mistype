# Manual test matrix

Run on **Kubuntu (KDE Plasma 6, Wayland)** in: Kate, Firefox, Konsole, Telegram, VS Code.
Mark each cell: ✅ pass · ❌ fail · ⚠️ partial · — not applicable.

Prerequisites:

```bash
cargo build --release
./packaging/install.sh          # or: sudo apt install ./target/switchfix_*.deb
sudo usermod -aG input "$USER"  # re-login after
switchfix doctor                # all checks OK
systemctl --user start switchfix
```

## Matrix (PLAN.md §5)

| # | Scenario | Expectation | Kate | Firefox | Konsole | Telegram | VS Code |
|---|----------|-------------|------|---------|---------|----------|---------|
| M1 | Select `ghbdtn`, press **Ctrl+F1** | Selection becomes `привет`, layout switches to RU, clipboard restored | | | | | |
| M2 | No selection, type `ghbdtn` | NoInputFound (no last-word path) | | | | | |
| M3 | Empty selection, **Ctrl+F1** | `NoInputFound` (or fallback word if enabled), clipboard unchanged | | | | | |
| M4 | Select `hello мир`, **Ctrl+F1** | Converts by majority direction (`detect_direction`); mixed parts handled per map | | | | | |
| M5 | Password field + hotkey | No leak: excluded app or user docs warning; buffer not filled if app excluded | | | | | |
| M6 | Konsole, type `ghbdtn`, **Ctrl+Shift+F1** | Line cleared (Ctrl+A, Ctrl+K) + paste `привет` (Ctrl+Shift+V) | | | | | |
| M7 | Toggle paused (tray or **Ctrl+Pause**) | Hotkeys no-op, tray icon grey, buffer cleared | | | | | |
| M8 | Switch layout with system **Meta+Space** | `switchfix` current layout stays in sync (D-Bus `layoutChanged`) | | | | | |
| M9 | Unplug / replug USB keyboard | Hotkeys keep working (evdev rescan) | | | | | |
| M10 | VS Code search box selection + Ctrl+F1 | Selection path works (or falls back to word) | | | | | |
| M11 | Firefox contenteditable selection + Ctrl+F1 | Selection path works | | | | | |
| M12 | Add a 3rd layout (UA) in System Settings | EN↔RU conversion still works; UA text left alone | | | | | |
| M13 | User **not** in `input` group | `switchfix doctor` fails with hint; settings + tray still start | | | | | |
| M14 | Clipboard holds an image, convert selection | Text conversion works; image clipboard is **not** restored (documented) | | | | | |
| M15 | Type `ghbdtn` + space (mode `hint`) | Suggestion bar: `1. привет (раскладка)` | | | | | |
| M16 | Type `приветт` + space (mode `hint`) | Suggestion bar: `1. привет` | | | | | |
| M17 | **Ctrl+1** while bar visible | Word replaced, bar hides, short sound plays | | | | | |
| M18 | Click a candidate in the bar | Same as Ctrl+1 | | | | | |
| M19 | **Ctrl+F2** (mode `hotkey` or `hint`) | Top-1 applied without waiting for the bar; flash + sound | | | | | |
| M20 | Type `при` (completion on) | Bar shows completions (`привет`, …) | | | | | |
| M21 | **Ctrl+0** / «+в словарь» | Word written to `user.dic`, never flagged again | | | | | |
| M22 | `spell.mode = "auto"`, type `приветт` + space | Silently replaced with `привет` + sound; uncertain cases still show the bar | | | | | |
| M23 | Bar hidden, press **Ctrl+1** | Reaches the app (accept chords not consumed) | | | | | |

## Extra checks

| # | Scenario | Expectation |
|---|----------|-------------|
| E1 | `switchfix convert --text 'ghbdtn'` | Prints `привет`, no injection |
| E2 | `switchfix convert` with stdin `руддщ` | Prints `hello` |
| E3 | `switchfix spell --text 'приветт'` | `misspelled` + `1. привет` |
| E4 | `switchfix spell --text 'ghbdtn'` | `1. привет (раскладка)` |
| E5 | `switchfix spell --text 'привет'` | `correct ✓` |
| E3 | Save clipboard text → convert selection → paste | Original clipboard content is back |
| E4 | Kill daemon mid-conversion | No stuck keys; restart recovers |
| E5 | `journalctl --user -u switchfix -f` | No typed text in logs (default config) |
| E6 | Settings window: change hotkey, Save | `config.toml` updated; restart daemon to apply |
| E7 | Tray menu «Конвертировать выделение» | Same as Ctrl+F1 |
| E8 | Two conversions in a row (word, then selection) | Both succeed; buffer cleared after word path |

## Known limitations (expected)

- TUI full-screen apps (vim, tmux) are **not** supported in terminal mode.
- Hotkey changes require a daemon restart (MVP).
- Non-text clipboard is not restored (M14).
- Word-path only works when the caret is at the end of the word.
- Without `input` group: no global hotkeys / no keystroke buffer — use tray or `switchfix convert`.
