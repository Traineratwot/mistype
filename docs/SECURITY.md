# Security

## Threat model (MVP)

SwitchFix runs as the logged-in user and needs low-level input access. The
threat we care about: **a local attacker or a compromised process abusing
SwitchFix to exfiltrate typed text**. The threat we explicitly do not defend
against: a compromised kernel or a root attacker.

## Capabilities

| Capability | Why | Risk | Mitigation |
|------------|-----|------|------------|
| `input` group | Read `/dev/input/event*` (hotkeys, keystroke buffer), open `/dev/uinput` (inject keys) | Any process in `input` can keylog or inject | Documented; buffer is RAM-only and opt-out |
| Session D-Bus | `org.kde.KeyboardLayouts` — get/set layout | Minimal (layout index only) | No other bus names claimed |
| Clipboard | Save/put/get/restore around a conversion | Transient exposure of one selection | Marker protocol + best-effort restore; no persistence |
| Outbound HTTP(S) (optional) | «LLM-fix selection» to your configured OpenAI-compatible API | Selected text leaves the machine | Off unless `llm.base_url` is set; only the selection + system prompt are sent; token never logged |
| systemd user sandbox | Limits blast radius of a compromised binary | — | `ProtectSystem=strict`, `ProtectHome=read-only`, `NoNewPrivileges`, `RestrictAddressFamilies=AF_UNIX AF_NETLINK AF_INET AF_INET6`, `DevicePolicy=closed` + explicit `/dev/input` + `/dev/uinput` |

## Known limitations

1. **Group `input` is powerful.** Membership allows any `input`-group process to
   read raw key events. SwitchFix itself only keeps a short RAM buffer and does
   not log text by default. If this is unacceptable:
   - remove yourself from `input`, and
   - set `hotkeys.use_portal_hotkeys = true` (XDG GlobalShortcuts portal — no raw
     input access, no keystroke buffer; last-word mode becomes unavailable).
2. **uinput injection** can type arbitrary text as the user. The daemon only
   injects the converted string and a fixed set of chords (Ctrl+V, Ctrl+Insert,
   …). A compromised `switchfix` binary could inject anything — treat the
   binary like any other input-sensitive tool (verify package checksums).
3. **Clipboard races.** Another clipboard manager may observe or replace the
   marker / converted text during the settle window (`copy_settle_ms` /
   `paste_settle_ms`). Clipboard contents are never written to disk.
4. **Password fields cannot be reliably detected** on Wayland without AT-SPI.
   Do not press the hotkey in password fields. `behavior.excluded_apps` is a
   best-effort exclusion list for known password managers.
5. **Non-text clipboard** (images, files) is not restored after a conversion.
   See MANUAL-TEST-MATRIX M14.

## Secure defaults

- `privacy.typing_buffer = true` but RAM-only, max 256 keys, cleared on pause/exit.
- `privacy.log_text_content = false` — logs never contain typed text.
- `llm.base_url` empty — no network until you configure an endpoint.
- Service runs with a restrictive systemd sandbox (`AF_INET`/`AF_INET6` only for the optional LLM API).
- No telemetry, no auto-update. API token is stored in `config.toml` (recommend `chmod 600`).

## Reporting

Please report security issues privately to the maintainers (see `Cargo.toml`
authors) before public disclosure. Do not open a public issue for exploitable
bugs that involve keylogging or injection.
