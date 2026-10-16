#!/usr/bin/env bash
# Build a .deb from packaging/deb + target/release/switchfix.
# Usage: ./packaging/build-deb.sh [VERSION] [ARCH] [OUTPUT-DIR]
#   VERSION     e.g. 0.1.0   (default: workspace Cargo.toml version)
#   ARCH        amd64|arm64  (default: dpkg --print-architecture)
#   OUTPUT-DIR  default: target/
# When VERSION/ARCH are given, packaging is CI-friendly: the release binary is
# built only if missing (cargo build --release is incremental anyway).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${1:-}"
ARCH="${2:-}"
OUT_DIR="${3:-${ROOT}/target}"
PKG_NAME="switchfix"

if [[ -z "$VERSION" ]]; then
  VERSION="$(python3 -c "import tomllib; print(tomllib.load(open('${ROOT}/Cargo.toml','rb'))['workspace']['package']['version'])")"
fi
if [[ -z "$ARCH" ]]; then
  ARCH="$(dpkg --print-architecture 2>/dev/null || echo amd64)"
fi
case "$ARCH" in
  amd64 | arm64) ;;
  *)
    echo "unsupported arch: $ARCH (want amd64 or arm64)" >&2
    exit 1
    ;;
esac

STAGE="${ROOT}/target/deb-stage"
DEB_PATH="${OUT_DIR}/${PKG_NAME}_${VERSION}_${ARCH}.deb"

echo "==> Building release binary"
cargo build --release --manifest-path "${ROOT}/Cargo.toml"

echo "==> Staging package tree (${PKG_NAME} ${VERSION} ${ARCH})"
rm -rf "$STAGE"
mkdir -p \
  "$STAGE/DEBIAN" \
  "$STAGE/usr/bin" \
  "$STAGE/usr/lib/systemd/user" \
  "$STAGE/usr/share/switchfix/maps" \
  "$STAGE/usr/share/applications" \
  "$STAGE/usr/share/doc/switchfix" \
  "$STAGE/usr/share/man/man1" \
  "$STAGE/etc/udev/rules.d"

cp "${ROOT}/target/release/switchfix" "$STAGE/usr/bin/switchfix"
chmod 0755 "$STAGE/usr/bin/switchfix"

install -m 0644 "${ROOT}/packaging/systemd/switchfix.service" \
  "$STAGE/usr/lib/systemd/user/switchfix.service"
install -m 0644 "${ROOT}/packaging/systemd/99-switchfix-uinput.rules" \
  "$STAGE/etc/udev/rules.d/99-switchfix-uinput.rules"
install -m 0644 "${ROOT}/packaging/desktop/switchfix.desktop" \
  "$STAGE/usr/share/applications/switchfix.desktop"
install -m 0644 "${ROOT}/assets/maps/en2ru.json" "$STAGE/usr/share/switchfix/maps/en2ru.json"
install -m 0644 "${ROOT}/assets/maps/ru2en.json" "$STAGE/usr/share/switchfix/maps/ru2en.json"

if [[ -f "${ROOT}/assets/icons/switchfix.svg" ]]; then
  mkdir -p "$STAGE/usr/share/icons/hicolor/scalable/apps"
  install -m 0644 "${ROOT}/assets/icons/switchfix.svg" \
    "$STAGE/usr/share/icons/hicolor/scalable/apps/switchfix.svg"
fi

for doc in PRIVACY.md SECURITY.md MANUAL-TEST-MATRIX.md; do
  if [[ -f "${ROOT}/docs/${doc}" ]]; then
    install -m 0644 "${ROOT}/docs/${doc}" "$STAGE/usr/share/doc/switchfix/${doc}"
  fi
done

# Debian control scripts
install -m 0755 "${ROOT}/packaging/deb/DEBIAN/postinst" "$STAGE/DEBIAN/postinst"
install -m 0755 "${ROOT}/packaging/deb/DEBIAN/postrm" "$STAGE/DEBIAN/postrm"
# control with Version + Architecture filled in
sed -e "s/^Version: .*/Version: ${VERSION}/" \
  -e "s/^Architecture: .*/Architecture: ${ARCH}/" \
  "${ROOT}/packaging/deb/DEBIAN/control" > "$STAGE/DEBIAN/control"

# installed-Size
size_kb="$(du -sk "$STAGE" | cut -f1)"
echo "Installed-Size: ${size_kb}" >> "$STAGE/DEBIAN/control"

echo "==> Building ${DEB_PATH}"
mkdir -p "$OUT_DIR"
dpkg-deb --build --root-owner-group "$STAGE" "$DEB_PATH"
dpkg-deb --info "$DEB_PATH"
echo
echo "Install with:  sudo apt install ${DEB_PATH}"
