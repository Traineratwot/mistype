# Manual test matrix

Run on **Kubuntu (KDE Plasma 6, Wayland)** in: Kate, Firefox, Konsole, Telegram, VS Code.
Mark each cell: ✅ pass · ❌ fail · ⚠️ partial · — not applicable.

Hotkeys below are the **defaults** (`ctrl+alt+r` / `alt+r` / `ctrl+alt+t`) —
substitute your live bindings from `switchfix doctor` if configured.

Prerequisites:

```bash
cargo build --release
./packaging/install.sh          # or: sudo apt install ./target/switchfix_*.deb
sudo usermod -aG input "$USER"  # re-login after
switchfix doctor                # all checks OK (incl. "llm api: configured" for L rows)
systemctl --user start switchfix
```

LLM rows (L*) additionally need `[llm] base_url` set (and `api_token` if the
server requires one), e.g. `http://127.0.0.1:11434` for Ollama.

## Matrix (PLAN.md §5)

| # | Scenario | Expectation | Kate | Firefox | Konsole | Telegram | VS Code |
|---|----------|-------------|------|---------|---------|----------|---------|
| M1 | Select `ghbdtn`, press **Ctrl+Alt+R** | Selection becomes `привет`, layout switches to RU, clipboard restored | | | | | |
| M2 | No selection, type `ghbdtn` | NoInputFound (no last-word path) | | | | | |
| M3 | Empty selection, **Ctrl+Alt+R** | `NoInputFound`, clipboard unchanged | | | | | |
| M4 | Select `hello мир`, **Ctrl+Alt+R** | Converts by majority direction (`detect_direction`); mixed parts handled per map | | | | | |
| M5 | Password field + hotkey | No leak: excluded app or user docs warning; buffer not filled if app excluded | | | | | |
| M6 | Konsole, type `ghbdtn` (terminal app) | Line cleared (Ctrl+A, Ctrl+K) + paste `привет` (Ctrl+Shift+V) | | | | | |
| M7 | Toggle paused (tray or **Ctrl+Alt+T**) | Hotkeys no-op, tray icon grey, buffer cleared | | | | | |
| M8 | Switch layout with system **Meta+Space** | `switchfix` current layout stays in sync (D-Bus `layoutChanged`) | | | | | |
| M9 | Unplug / replug USB keyboard | Hotkeys keep working (evdev rescan) | | | | | |
| M10 | VS Code search box selection + Ctrl+Alt+R | Selection path works | | | | | |
| M11 | Firefox contenteditable selection + Ctrl+Alt+R | Selection path works | | | | | |
| M12 | Add a 3rd layout (UA) in System Settings | EN↔RU conversion still works; UA text left alone | | | | | |
| M13 | User **not** in `input` group | `switchfix doctor` fails with hint; settings + tray still start | | | | | |
| M14 | Clipboard holds an image, convert selection | Text conversion works; image clipboard is **not** restored (documented) | | | | | |

## LLM-fix selection + undo

| # | Scenario | Expectation | Kate | Firefox | Konsole | Telegram | VS Code |
|---|----------|-------------|------|---------|---------|----------|---------|
| L1 | Select a sentence with a typo, **Alt+R** | Busy yellow blink while the LLM works; selection replaced with corrected text; flash + sound | | | | | |
| L2 | Immediately after L1, press **Backspace once** (within ~4 s) | Whole replace undone — original text restored | | | | | |
| L3 | After an LLM replace, wait **>5 s**, press Backspace | Normal Backspace: only one character of the replacement is deleted | | | | | |
| L4 | After an LLM replace, type a character, then Backspace | Undo cancelled by typing; Backspace deletes one char only | | | | | |
| L5 | `privacy.typing_buffer = false`, LLM replace, Backspace within 4 s | Undo still works (Backspace event is ungated) | | | | | |
| L6 | Empty selection, **Alt+R** | No network call (check `RUST_LOG=debug` / server logs); no replace | | | | | |
| L7 | Bad `api_token` or unreachable `base_url`, **Alt+R** | Error notify «LLM не ответил — текст не изменён»; selection text **unchanged** | | | | | |
| L8 | Settings: change `base_url` / `system_prompt`, Save or Reload | New settings apply (no daemon restart needed); next Alt+R uses the new endpoint/prompt | | | | | |

## Extra checks

| # | Scenario | Expectation |
|---|----------|-------------|
| E1 | `switchfix convert --text 'ghbdtn'` | Prints `привет`, no injection |
| E2 | `switchfix convert` with stdin `руддщ` | Prints `hello` |
| E3 | Save clipboard text → convert selection → paste | Original clipboard content is back |
| E4 | Kill daemon mid-conversion | No stuck keys; restart recovers |
| E5 | `journalctl --user -u switchfix -f` | No typed text, no `api_token`, no selection text in logs (default config) |
| E6 | Settings window: change hotkey, Save | `config.toml` updated; daemon restarts itself and the new hotkey works |
| E7 | Tray menu «Конвертировать выделение» | Same as Ctrl+Alt+R |
| E8 | Two conversions in a row | Both succeed |
| E9 | `switchfix doctor` with/without `[llm] base_url` | `llm api: configured …` / `not configured …`; token never printed |

## Known limitations (expected)

- TUI full-screen apps (vim, tmux) are **not** supported in terminal mode.
- Hotkey changes apply after Settings Save (daemon auto-restarts).
- Non-text clipboard is not restored (M14).
- Without `input` group: no global hotkeys / no keystroke buffer — use tray or `switchfix convert`.
- LLM-fix sends the selection to the user-configured `llm.base_url` only; with
  an empty `base_url` the feature is off and nothing is sent.
- Backspace undo is fixed at one Backspace within ~4 s and is not configurable.
