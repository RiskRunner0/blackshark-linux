#!/usr/bin/env bash
set -euo pipefail

# ---------------------------------------------------------------------------
# blackshark-linux development installer for NixOS
#
# Builds from the checkout and installs binaries and the systemd user service
# into the user's home. This is for development only; use app-blackshark-linux.nix
# from your NixOS configuration for a normal installation.
# Does NOT modify /etc or install udev rules automatically.
#
# NixOS udev configuration should be added declaratively to configuration.nix:
#
# services.udev.extraRules = ''
#     KERNEL=="hidraw*", SUBSYSTEM=="hidraw", ATTRS{idVendor}=="1532",
#     ATTRS{idProduct}=="0577", MODE="0660", GROUP="input", TAG+="uaccess"
# '';
# ---------------------------------------------------------------------------

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

BIN_DIR="${HOME}/.local/bin"
SYSTEMD_DIR="${HOME}/.config/systemd/user"

RED='\033[0;31m'
GREEN='\033[0;32m'
BOLD='\033[1m'
NC='\033[0m'

info() {
    echo -e "${BOLD}==> $*${NC}"
}

ok() {
    echo -e "${GREEN}    ok${NC}"
}

die() {
    echo -e "${RED}error: $*${NC}" >&2
    exit 1
}

IGNORE_HID_PERMISSIONS=false
for arg in "$@"; do
    case "$arg" in
        --ignore-hid-permissions)
            IGNORE_HID_PERMISSIONS=true
            ;;
        *)
            die "unknown option: $arg"
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Checks
# ---------------------------------------------------------------------------

info "Checking dependencies"

command -v cargo >/dev/null 2>&1 || \
    die "cargo not found. Enter your Nix development shell first."

command -v systemctl >/dev/null 2>&1 || \
    die "systemctl not found. This installer requires systemd."

command -v install >/dev/null 2>&1 || \
    die "install command not found."

command -v udevadm >/dev/null 2>&1 || \
    die "udevadm not found. This installer requires udev."

# Verify the Nix runtime libraries used by blackshark-gui are available.
for pkg in wayland libxkbcommon libglvnd mesa; do
    nix eval --raw "nixpkgs#${pkg}" >/dev/null 2>&1 || \
        die "Nix package '${pkg}' is not available."
done

ok

# ---------------------------------------------------------------------------
# Check udev permissions
# ---------------------------------------------------------------------------

echo ""
info "Checking HID permissions"

if [[ "$IGNORE_HID_PERMISSIONS" == true ]]; then
    echo "    skipped (--ignore-hid-permissions)"
else
    user_in_input=false
    if id -nG "$USER" | grep -qw input; then
        user_in_input=true
        echo "    user is in the 'input' group"
    else
        echo -e "${RED}warning: $USER is not in the 'input' group.${NC}"
    fi

    device_in_input=false
    if command -v udevadm >/dev/null 2>&1; then
        shopt -s nullglob
        for device in /dev/hidraw*; do
            if udevadm info --query=property --name="$device" 2>/dev/null | \
                grep -qFx 'ID_VENDOR_ID=1532' && \
                udevadm info --query=property --name="$device" 2>/dev/null | \
                grep -qFx 'ID_MODEL_ID=0577'; then
                device_group="$(stat -c '%G' "$device")"
                if [[ "$device_group" == input ]]; then
                    device_in_input=true
                    echo "    $device is in the 'input' group"
                else
                    echo -e "${RED}warning: $device is in the '$device_group' group, not 'input'.${NC}"
                fi
            fi
        done
    fi

    if [[ "$device_in_input" == false ]]; then
        echo -e "${RED}warning: BlackShark V3 Pro device was not found in the 'input' group.${NC}"
        echo "    If you know what you are doing and use another permission setup, you can"
        echo "    ignore this check with:"
        echo ""
        echo "      ./dev-install-nix.sh --ignore-hid-permissions"
        echo ""
        echo "    Make sure your user still has read/write access to the HID device."
        echo "    Without that access, blacksharkd will not be able to communicate with the headset."
    fi

    if [[ "$user_in_input" == false || "$device_in_input" == false ]]; then
        echo ""
        echo "    Add this to /etc/nixos/configuration.nix:"
        echo ""
        echo "      users.users.${USER}.extraGroups = [ \"input\" ];"
        echo ""
        echo "    and add the udev rule:"
        echo ""
        echo "      services.udev.extraRules = ''"
        echo "        KERNEL==\"hidraw*\", SUBSYSTEM==\"hidraw\", ATTRS{idVendor}==\"1532\", ATTRS{idProduct}==\"0577\", MODE=\"0660\", GROUP=\"input\", TAG+=\"uaccess\""
        echo "      '';"
        echo ""
        echo "    Then run:"
        echo ""
        echo "      sudo nixos-rebuild switch"
        echo ""
    fi
fi

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
    src="${REPO_DIR}/target/release/${bin}"

    [[ -x "$src" ]] || die "built binary not found: $src"

    install -m755 "$src" "${BIN_DIR}/${bin}"
    echo "    ${BIN_DIR}/${bin}"
done

# Make sure NixOS loads the required runtime libraries for the GUI.
mv "${BIN_DIR}/blackshark-gui" "${BIN_DIR}/blackshark-gui.bin"

cat > "${BIN_DIR}/blackshark-gui" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

# Make sure NixOS loads the required runtime libraries.
export LD_LIBRARY_PATH="$(
    nix eval --raw nixpkgs#wayland
)/lib:$(
    nix eval --raw nixpkgs#libxkbcommon
)/lib:$(
    nix eval --raw nixpkgs#libglvnd
)/lib:$(
    nix eval --raw nixpkgs#mesa
)/lib:${LD_LIBRARY_PATH:-}"

exec "$HOME/.local/bin/blackshark-gui.bin" "$@"
EOF

chmod 755 "${BIN_DIR}/blackshark-gui"

echo "    ${BIN_DIR}/blackshark-gui"
echo "    ${BIN_DIR}/blackshark-gui.bin"

ok

# ---------------------------------------------------------------------------
# PATH warning
# ---------------------------------------------------------------------------

if [[ ":$PATH:" != *":${BIN_DIR}:"* ]]; then
    echo ""
    echo -e "${RED}warning: ${BIN_DIR} is not in your PATH.${NC}"
    echo ""
    echo "For a temporary session:"
    echo ""
    echo "    export PATH=\"\$HOME/.local/bin:\$PATH\""
    echo ""
    echo "For NixOS, consider adding this to your shell configuration:"
    echo ""
    echo "    environment.localBinInPath = true;"
    echo ""
fi

# ---------------------------------------------------------------------------
# systemd user service
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
# Done
# ---------------------------------------------------------------------------

echo ""
echo -e "${GREEN}${BOLD}Installation complete.${NC}"
echo ""

echo "Next steps:"
echo ""

echo "  1. Make sure the NixOS udev rule is configured:"
echo ""
echo "       services.udev.extraRules = ''"
echo "         KERNEL==\"hidraw*\", SUBSYSTEM==\"hidraw\", ATTRS{idVendor}==\"1532\", ATTRS{idProduct}==\"0577\", MODE=\"0660\", GROUP=\"input\", TAG+=\"uaccess\""
echo "       '';"
echo ""

echo "  2. Rebuild NixOS if you changed the configuration:"
echo ""
echo "       sudo nixos-rebuild switch"
echo ""

echo "  3. Make sure ~/.local/bin is in PATH:"
echo ""
echo "       export PATH=\"\$HOME/.local/bin:\$PATH\""
echo ""

echo "  4. Plug in the BlackShark V3 Pro USB dongle."
echo ""

echo "  5. Check the daemon:"
echo ""
echo "       systemctl --user status blacksharkd"
echo ""

echo "  6. Check the headset:"
echo ""
echo "       blackshark-ctl status"
echo ""

echo "  7. Read battery:"
echo ""
echo "       blackshark-ctl battery"
echo ""

echo "  JSON status:"
echo ""
echo "       blackshark-ctl status --json"
echo ""

