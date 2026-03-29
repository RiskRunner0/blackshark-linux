#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# blackshark-ctl installer
# Builds release binaries and wires up the systemd user service + udev rule.
# ---------------------------------------------------------------------------

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="${HOME}/.local/bin"
SYSTEMD_DIR="${HOME}/.config/systemd/user"
UDEV_DIR="/etc/udev/rules.d"

# Colours
RED='\033[0;31m'
GREEN='\033[0;32m'
BOLD='\033[1m'
NC='\033[0m'

info()  { echo -e "${BOLD}==> $*${NC}"; }
ok()    { echo -e "${GREEN}    ok${NC}"; }
die()   { echo -e "${RED}error: $*${NC}" >&2; exit 1; }

# ---------------------------------------------------------------------------
# Checks
# ---------------------------------------------------------------------------

info "Checking dependencies"

command -v cargo     >/dev/null 2>&1 || die "cargo not found — install Rust from https://rustup.rs"
command -v systemctl >/dev/null 2>&1 || die "systemctl not found — this installer requires systemd"

# Warn if the user is not in the 'users' group (needed for the udev rule).
if ! id -nG "$USER" | grep -qw users; then
    echo -e "${RED}warning: you are not in the 'users' group.${NC}"
    echo "         The udev rule grants GROUP=users access to the HID device."
    echo "         Run: sudo usermod -aG users \$USER"
    echo "         Then log out and back in, and re-run this script."
    echo ""
fi

ok

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------

info "Building release binaries (this may take a minute)"
cd "$REPO_DIR"
cargo build --release -p blacksharkd -p blackshark-ctl -p blackshark-tray -p blackshark-gui
ok

# ---------------------------------------------------------------------------
# Install binaries
# ---------------------------------------------------------------------------

info "Installing binaries to ${BIN_DIR}"
mkdir -p "$BIN_DIR"

for bin in blacksharkd blackshark-ctl blackshark-tray blackshark-gui; do
    install -m755 "target/release/${bin}" "${BIN_DIR}/${bin}"
    echo "    ${BIN_DIR}/${bin}"
done
ok

# Make sure ~/.local/bin is on PATH (common omission on fresh systems).
if [[ ":$PATH:" != *":${BIN_DIR}:"* ]]; then
    echo ""
    echo -e "${RED}warning: ${BIN_DIR} is not in your PATH.${NC}"
    echo "         Add this to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
    echo "           export PATH=\"\$HOME/.local/bin:\$PATH\""
    echo ""
fi

# ---------------------------------------------------------------------------
# Systemd user service
# ---------------------------------------------------------------------------

info "Installing systemd user service"
mkdir -p "$SYSTEMD_DIR"
install -m644 "${REPO_DIR}/pkg/blacksharkd.service" "${SYSTEMD_DIR}/blacksharkd.service"
echo "    ${SYSTEMD_DIR}/blacksharkd.service"

systemctl --user daemon-reload
systemctl --user enable blacksharkd
systemctl --user restart blacksharkd
echo "    enabled and started blacksharkd"
ok

# ---------------------------------------------------------------------------
# udev rule (requires sudo)
# ---------------------------------------------------------------------------

info "Installing udev rule (requires sudo)"
sudo install -m644 "${REPO_DIR}/pkg/99-blackshark.rules" "${UDEV_DIR}/99-blackshark.rules"
echo "    ${UDEV_DIR}/99-blackshark.rules"
sudo udevadm control --reload-rules
sudo udevadm trigger
echo "    udev rules reloaded"
ok

# ---------------------------------------------------------------------------
# Done
# ---------------------------------------------------------------------------

echo ""
echo -e "${GREEN}${BOLD}Installation complete.${NC}"
echo ""
echo "  Daemon:   systemctl --user status blacksharkd"
echo "  CLI:      blackshark-ctl status"
echo "  Tray:     blackshark-tray  (add to your autostart)"
echo "  GUI:      blackshark-gui"
echo ""
echo "If the headset is not detected, replug the USB dongle after the udev"
echo "rules have been applied."
