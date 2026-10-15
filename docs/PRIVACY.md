# Privacy

SwitchFix is designed so that fixing a typo never turns into surveillance.

## Principles

| Principle | How it is implemented |
|-----------|----------------------|
| Not a keylogger | The keystroke buffer exists only in RAM (≤ 256 keys by default), is opt-in, and is dropped on exit / pause. Typed text is **never** written to disk or logs. |
| Local only | No network sockets, no telemetry, no analytics. The systemd user unit sets `RestrictAddressFamilies=AF_UNIX AF_NETLINK` and `ProtectSystem=strict`. |
| Clipboard is borrowed, not kept | Before reading a selection SwitchFix saves the current clipboard, uses a unique marker, then restores the snapshot after paste. Non-text clipboard content is not restored (documented limitation). |
| Password fields | Exclude list (`behavior.excluded_apps`, default KeepassXC / KWalletManager) plus a documentation warning: do not press the hotkey in password fields. |
| Group `input` is a trade-off | Global hotkeys and uinput injection require membership in `input`. The risk is documented in [SECURITY.md](SECURITY.md). The XDG GlobalShortcuts portal fallback works without `input` and without the keystroke buffer. |
| Config is local | `~/.config/switchfix/config.toml` — plain TOML, no cloud sync. |

## What leaves the process

Nothing. SwitchFix does not open network connections. The only IPC is:

- **D-Bus session bus** — `org.kde.keyboard` / `org.kde.KeyboardLayouts` to read and set the current layout.
- **evdev / uinput** — read key events (for hotkeys + optional buffer) and inject synthetic keys.
- **Clipboard** (Wayland/X11) — save / put / get / restore around a single conversion.

## What is stored

| Data | Where | Lifetime |
|------|-------|----------|
| Keystroke buffer (last N chars) | RAM | Until convert, pause, exit, or overflow |
| Saved clipboard snapshot | RAM | Duration of one conversion |
| Config | `~/.config/switchfix/config.toml` | Until you delete it |
| Logs | journald (optional) | Never include typed text (`privacy.log_text_content = false`, default) |

## Settings → Privacy

> SwitchFix не отправляет данные в сеть и не сохраняет введённый текст. Буфер набранных символов живёт только в оперативной памяти и очищается при паузе или выходе.

## Your choices

- `privacy.typing_buffer = false` — disable the keystroke buffer entirely.
- `privacy.typing_buffer_max_keys` — shrink the RAM window (default 256).
- `privacy.log_text_content` — keep **false** unless you are debugging; when true, logs may contain converted text.
- Uninstall / purge removes the package; delete `~/.config/switchfix/` to remove all local state.
