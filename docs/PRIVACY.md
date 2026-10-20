# Privacy

SwitchFix is designed so that fixing a typo never turns into surveillance.

## Principles

| Principle | How it is implemented |
|-----------|----------------------|
| Not a keylogger | The keystroke buffer exists only in RAM (≤ 256 keys by default), is opt-in, and is dropped on exit / pause. Typed text is **never** written to disk or logs. |
| Network only when you ask | No telemetry, no analytics. The only outbound network call is the optional **LLM-fix selection** request to the OpenAI-compatible endpoint **you** configure. |
| Clipboard is borrowed, not kept | Before reading a selection SwitchFix saves the current clipboard, uses a unique marker, then restores the snapshot after paste. Non-text clipboard content is not restored (documented limitation). |
| Password fields | Exclude list (`behavior.excluded_apps`, default KeepassXC / KWalletManager) plus a documentation warning: do not press the hotkey in password fields. |
| Group `input` is a trade-off | Global hotkeys and uinput injection require membership in `input`. The risk is documented in [SECURITY.md](SECURITY.md). |
| Config is local | `~/.config/switchfix/config.toml` — plain TOML, no cloud sync. The LLM API token lives here (recommend `chmod 600`). |

## What leaves the process

By default nothing. When **LLM-fix selection** is enabled (`llm.enabled` and
`llm.base_url` set), pressing the LLM hotkey sends:

- the **selected text** (only what you highlighted),
- the configured **system prompt**,

to `POST {llm.base_url}/v1/chat/completions`. No other text, no keystroke
buffer, no clipboard history. The response is used once to replace the
selection and is not stored.

Other IPC (always local):

- **D-Bus session bus** — `org.kde.keyboard` / `org.kde.KeyboardLayouts` to read and set the current layout.
- **evdev / uinput** — read key events (for hotkeys + optional buffer) and inject synthetic keys.
- **Clipboard** (Wayland/X11) — save / put / get / restore around a single conversion.

## What is stored

| Data | Where | Lifetime |
|------|-------|----------|
| Keystroke buffer (last N chars) | RAM | Until convert, pause, exit, or overflow |
| Saved clipboard snapshot | RAM | Duration of one conversion |
| Undo target (original + replacement) | RAM | ~4 seconds after an LLM replace, or until the next key |
| LLM-fix history (last 30 from/to pairs) | `/tmp/switchfix.llm-history.json` | Until reboot or tmp cleanup. Used by the Plasma widget; click-to-copy only. |
| Config (incl. LLM token) | `~/.config/switchfix/config.toml` | Until you delete it |
| Logs | journald (optional) | Never include typed text (`privacy.log_text_content = false`, default). Never include the API token. |

## Settings → Privacy

> Текст набирается только в оперативной памяти (опционально). При включённом
> LLM-исправлении выделенный текст отправляется на указанный вами API-адрес.
> Никакой другой текст не уходит в сеть.

## Your choices

- `llm.enabled = false` or empty `llm.base_url` — no network at all.
- `privacy.typing_buffer = false` — disable the keystroke buffer entirely (LLM Backspace-undo still works).
- `privacy.typing_buffer_max_keys` — shrink the RAM window (default 256).
- `privacy.log_text_content` — keep **false** unless you are debugging; when true, logs may contain converted text.
- Uninstall / purge removes the package; delete `~/.config/switchfix/` to remove all local state (including the token).
