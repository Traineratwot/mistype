#!/usr/bin/env bash
# Mistype developer / local installer (no dpkg required).
# Privileged steps use `sudo` and prompt for a password only when needed.
#
# Usage:
#   ./packaging/install.sh              # build + install to PREFIX (default /usr/local)
#   PREFIX=$HOME/.local ./packaging/install.sh   # no sudo needed for user prefix
#   ./packaging/install.sh --yes        # non-interactive; still uses sudo if required
#   ./packaging/install.sh --uninstall
#   ./packaging/install.sh --udev       # force-install udev rule
#   ./packaging/install.sh --input-group # force-add user to group 'input'
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PREFIX="${PREFIX:-/usr/local}"
BIN_DIR="${PREFIX}/bin"
LIB_DIR="${PREFIX}/lib"
SYSTEMD_USER_DIR="${LIB_DIR}/systemd/user"
SHARE_DIR="${PREFIX}/share/mistype"
DESKTOP_DIR="${PREFIX}/share/applications"
DOC_DIR="${PREFIX}/share/doc/mistype"
ICON_DIR="${PREFIX}/share/icons/hicolor/scalable/apps"
UDEV_RULE_SRC="${ROOT}/packaging/systemd/99-mistype-uinput.rules"
UDEV_RULE_DST="/etc/udev/rules.d/99-mistype-uinput.rules"

uninstall=0
assume_yes=0
with_udev=0
with_input_group=0
for arg in "$@"; do
  case "$arg" in
    --uninstall) uninstall=1 ;;
    --yes|-y) assume_yes=1 ;;
    --udev) with_udev=1 ;;
    --input-group) with_input_group=1 ;;
    -h|--help)
      sed -n '2,12p' "$0"
      exit 0
      ;;
    *)
      echo "unknown option: $arg" >&2
      exit 2
      ;;
  esac
done

die() { echo "error: $*" >&2; exit 1; }

is_root() { [[ "$(id -u)" -eq 0 ]]; }

# Run a command as root, prompting for a sudo password only when needed.
# Usage: run_root cmd args...
run_root() {
  if is_root; then
    "$@"
  else
    echo "  (sudo) $*"
    sudo -- "$@"
  fi
}

# Install files into PREFIX; escalate to sudo only if we cannot write there.
install_file() { # mode src dst
  local mode="$1" src="$2" dst="$3"
  if install -m "$mode" "$src" "$dst" 2>/dev/null; then
    return 0
  fi
  run_root install -m "$mode" "$src" "$dst"
}

install_dir() { # dir...
  local d
  for d in "$@"; do
    if mkdir -p "$d" 2>/dev/null; then
      continue
    fi
    run_root mkdir -p "$d"
  done
}

ask_yes_no() { # prompt default(0/1)
  local prompt="$1" def="${2:-1}" hint ans
  if [[ "$assume_yes" -eq 1 ]]; then
    return 0
  fi
  if [[ "$def" -eq 1 ]]; then hint="[Y/n]"; else hint="[y/N]"; fi
  printf '%s %s ' "$prompt" "$hint"
  if ! read -r ans; then
    ans=""
  fi
  case "${ans,,}" in
    y|yes) return 0 ;;
    n|no) return 1 ;;
    *) return "$def" ;;
  esac
}

if [[ "$uninstall" -eq 1 ]]; then
  echo "Uninstalling Mistype from ${PREFIX}"
  rm -f "${BIN_DIR}/mistype" 2>/dev/null || run_root rm -f "${BIN_DIR}/mistype"
  rm -f "${SYSTEMD_USER_DIR}/mistype.service" 2>/dev/null || run_root rm -f "${SYSTEMD_USER_DIR}/mistype.service"
  rm -f "${DESKTOP_DIR}/mistype.desktop" 2>/dev/null || run_root rm -f "${DESKTOP_DIR}/mistype.desktop"
  rm -rf "${SHARE_DIR}" "${DOC_DIR}" 2>/dev/null || run_root rm -rf "${SHARE_DIR}" "${DOC_DIR}"
  rm -f "${ICON_DIR}/mistype.svg" 2>/dev/null || run_root rm -f "${ICON_DIR}/mistype.svg"
  systemctl --user disable --now mistype.service 2>/dev/null || true
  systemctl --user daemon-reload 2>/dev/null || true
  if [[ -f "$UDEV_RULE_DST" ]]; then
    if ask_yes_no "Remove udev rule ${UDEV_RULE_DST} (needs sudo)?" 1; then
      run_root rm -f "$UDEV_RULE_DST"
      run_root udevadm control --reload-rules || true
    fi
  fi
  echo "Done."
  exit 0
fi

echo "==> Building release binary"
cargo build --release --manifest-path "${ROOT}/Cargo.toml"

echo "==> Installing to ${PREFIX}"
install_dir "$BIN_DIR" "$SYSTEMD_USER_DIR" "$SHARE_DIR/maps" "$DESKTOP_DIR" "$DOC_DIR" "$ICON_DIR"

install_file 0755 "${ROOT}/target/release/mistype" "${BIN_DIR}/mistype"
install_file 0644 "${ROOT}/packaging/systemd/mistype.service" "${SYSTEMD_USER_DIR}/mistype.service"
install_file 0644 "${ROOT}/packaging/desktop/mistype.desktop" "${DESKTOP_DIR}/mistype.desktop"
install_file 0644 "${ROOT}/assets/maps/en2ru.json" "${SHARE_DIR}/maps/en2ru.json"
install_file 0644 "${ROOT}/assets/maps/ru2en.json" "${SHARE_DIR}/maps/ru2en.json"

if [[ -f "${ROOT}/assets/icons/mistype.svg" ]]; then
  install_file 0644 "${ROOT}/assets/icons/mistype.svg" "${ICON_DIR}/mistype.svg"
fi

# Plasma LLM-history widget → ~/.local/share/plasma/plasmoids (user-level, no sudo).
PLASMOID_SRC="${ROOT}/packaging/plasmoid/org.mistype.llm"
PLASMOID_DST="${HOME}/.local/share/plasma/plasmoids/org.mistype.llm"
if [[ -d "$PLASMOID_SRC" ]]; then
  echo "==> Installing Plasma widget to ${PLASMOID_DST}"
  mkdir -p "${HOME}/.local/share/plasma/plasmoids"
  rm -rf "$PLASMOID_DST"
  cp -a "$PLASMOID_SRC" "$PLASMOID_DST"
  echo "    Add it via: right-click panel → Add Widgets → 'Mistype LLM'"
fi

for doc in PRIVACY.md SECURITY.md MANUAL-TEST-MATRIX.md; do
  if [[ -f "${ROOT}/docs/${doc}" ]]; then
    install_file 0644 "${ROOT}/docs/${doc}" "${DOC_DIR}/${doc}"
  fi
done

# Ship the udev rule next to maps so `mistype setup` can find it later.
if [[ -f "$UDEV_RULE_SRC" ]]; then
  install_file 0644 "$UDEV_RULE_SRC" "${SHARE_DIR}/99-mistype-uinput.rules"
fi

systemctl --user daemon-reload 2>/dev/null || true

# ---- input group (sudo password prompt only here if accepted) ----
if id -nG "$USER" | tr ' ' '\n' | grep -qx input; then
  if id -nG | tr ' ' '\n' | grep -qx input; then
    echo "==> Already in group 'input'"
  else
    echo "==> User is in group 'input' (re-login needed for this session)"
  fi
elif [[ "$with_input_group" -eq 1 ]] || ask_yes_no "Add \$USER to group 'input' for global hotkeys (needs sudo)?" 1; then
  echo "==> Adding user to group 'input'"
  run_root usermod -aG input "$USER"
  echo "    (log out and back in for this to take effect)"
else
  echo "==> Skipped group 'input' (global hotkeys / keystroke buffer will be unavailable)"
fi

# ---- udev rule (sudo password prompt only here if accepted) ----
if [[ -f "$UDEV_RULE_DST" ]]; then
  echo "==> udev rule already installed"
elif [[ "$with_udev" -eq 1 ]] || ask_yes_no "Install udev rule for /dev/uinput (needs sudo)?" 1; then
  echo "==> Installing udev rule"
  run_root install -m 0644 "$UDEV_RULE_SRC" "$UDEV_RULE_DST"
  run_root udevadm control --reload-rules
  run_root udevadm trigger --subsystem-match=input --action=change || true
else
  echo "==> Skipped udev rule"
fi

# ---- user service (no sudo) ----
if ask_yes_no "Enable and start the Mistype user service (no sudo)?" 1; then
  systemctl --user enable --now mistype || echo "    (unit not available yet — start later with: systemctl --user enable --now mistype)"
else
  echo "==> Service left disabled (enable later: systemctl --user enable --now mistype)"
fi

cat <<EOF

Mistype installed to ${PREFIX}.

  Check:   mistype doctor
  Setup:   mistype setup          # re-run privileged steps interactively
  Config:  \${XDG_CONFIG_HOME:-\$HOME/.config}/mistype/config.toml

If you were just added to group 'input', log out and back in before using
global hotkeys.
EOF
