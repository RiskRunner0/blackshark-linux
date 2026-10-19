# blackshark-ctl

Linux userspace driver for the **Razer BlackShark V3 Pro** wireless headset.

Controls sidetone, EQ presets, THX Spatial Audio, Active Noise Cancellation, and power savings — without Razer Synapse or Windows.

---

## Features

- **Sidetone** — mic monitoring level (0–15)
- **EQ presets** — all 9 named Synapse presets (Flat, Bass Boost, FPS, etc.)
- **THX Spatial Audio** — toggle surround sound on/off
- **ANC** — enable/disable and set level (1–4)
- **Power savings** — auto-off timeout (off, 15, 30, 45, 60 min)
- **Battery** — percentage and charging status, polled every 5 minutes
- **Settings persist** — config saved to `~/.config/blackshark/config.toml`, restored on reconnect
- **System tray** — battery %, quick toggles, EQ/sidetone submenus, daemon controls
- **GUI** — full settings panel with live updates
- **CLI** — scriptable control and JSON status output

![Device tab showing battery, connection status and audio controls](assets/Device_page.png)
*GUI settings panel — Device tab*

![System tray menu with headset controls and daemon status](assets/tray_icon.png)
*System tray with quick-access controls*

---

## Requirements

- Linux (tested on Arch/CachyOS with KDE Plasma + Wayland)
- Rust (stable) — [rustup.rs](https://rustup.rs)
- systemd (user session)
- PipeWire or PulseAudio (optional — only needed for the experimental game/chat mix feature)

> **Firmware note:** Firmware 1.3.x or later is required on both the headset and dongle.
> Users on 1.2.x have reported the daemon sees the dongle but cannot communicate with the headset.
> Update firmware via Razer Synapse on Windows before switching to Linux. (See [#1](https://github.com/RiskRunner0/blackshark-linux/issues/1))

> **Stuck on "waiting for headset"?** The daemon can see the dongle but the dongle is not linked
> to the headset. Check that the headset is powered on, in 2.4GHz mode (LED green, not white/Bluetooth),
> and still paired to the dongle. Re-pair with Razer's pairing tool on Windows if needed
> (see [#7](https://github.com/RiskRunner0/blackshark-linux/issues/7)). Run `journalctl --user -u blacksharkd`
> for details.

---

## Quick install

NixOS users can follow the [Install for NixOS](#install-for-nixos) instructions below.

**Option A — pre-built release (recommended):**

1. Download the latest release tarball from the [Releases page](https://github.com/RiskRunner0/blackshark-linux/releases)
2. Extract and run:
   ```bash
   tar xzf blackshark-ctl-*.tar.gz
   cd blackshark-ctl-*
   ./install.sh
   ```

**Option B — build from source:**

```bash
git clone https://github.com/RiskRunner0/blackshark-linux.git
cd blackshark-linux
./install.sh
```

The script installs the binaries to `~/.local/bin/`, starts the daemon as a systemd user service, and installs the udev rule (requires `sudo`) so the daemon can access the headset without root.

---

## Install for NixOS

NixOS users should install BlackShark declaratively. Copy [app-blackshark-linux.nix](app-blackshark-linux.nix) somewhere managed by your NixOS configuration, for example `/etc/nixos/app-blackshark-linux.nix`, then add this to `/etc/nixos/configuration.nix`:

The example below is for a traditional, non-flake NixOS configuration. It also works with flakes if `app-blackshark-linux.nix` is included in the flake and the path is changed to `./app-blackshark-linux.nix`. Home Manager by itself cannot install the udev rule or add your user to the `input` group; those parts still belong in the NixOS configuration.

```nix
{ config, lib, pkgs, ... }:

let
      custom-app-blackshark-linux =
            pkgs.callPackage /etc/nixos/app-blackshark-linux.nix { };
in
{
      # Allows users from group 'input' to read/write from the headset
      users.users.<your-username>.extraGroups = [ "input" ];

      # Installs the udev rule from the package
      services.udev.packages = [
            custom-app-blackshark-linux
      ];

      # Installs blacksharkd, blackshark-ctl, blackshark-tray, and blackshark-gui
      environment.systemPackages = [
            custom-app-blackshark-linux
      ];

      # Runs blacksharkd as a systemd user service
      systemd.user.services.blacksharkd = {
            wantedBy = [ "default.target" ];
            after = [ "graphical-session.target" ];

            serviceConfig = {
                  ExecStart = "${custom-app-blackshark-linux}/bin/blacksharkd";
                  Restart = "on-failure";
                  RestartSec = 2;
            };
      };
}
```

Replace `<your-username>` with your Linux username. The package builds the current tagged release, installs all four programs, and provides the udev rule. `users.users.<your-username>.extraGroups = [ "input" ];` gives your user permission to access the headset; log out and back in after adding the group. If the dongle was already connected, unplug and reconnect it after rebuilding so udev applies the rule.

Apply the configuration with:

```bash
sudo nixos-rebuild switch
```

Then check the daemon and headset:

```bash
systemctl --user status blacksharkd
blackshark-ctl status
```

Before using the headset on Linux, follow the firmware requirement in [Requirements](#requirements).

### Updating the NixOS package

If you keep a local copy of the package definition, run:

```bash
cd blackshark-linux-nix
./nixos-update-blackshark-linux.sh
```

It updates the `version` and `sha256` in `app-blackshark-linux.nix` to the latest tag. If anything breaks, there is a `app-blackshark-linux.nix.bak` you can restore.

If the package file stops working for the newest tags, open an issue on the repo.

### Building locally for development

`dev-install-nix.sh` is provided for development or for situations where you specifically need to build and install the binaries from a checkout. It is not the normal NixOS installation path: it installs files imperatively into `~/.local/bin` and `~/.config/systemd/user`, outside the Nix store and your NixOS configuration.

If you need it, enter the repository's development shell first:

```bash
cd blackshark-linux-nix
nix-shell
./dev-install-nix.sh
```

The `shell.nix` file supplies the Rust toolchain and native dependencies needed to build the project. The development installer does not modify your NixOS configuration or install udev rules automatically, so the declarative configuration above is still required for headset permissions.

---

## Getting started

After install, plug in the USB dongle. Then:

**1. Check the daemon is running:**
```bash
systemctl --user status blacksharkd
```

**2. Verify the headset is detected:**
```bash
blackshark-ctl status
```

**3. Start the system tray:**
```bash
blackshark-tray &
```
The tray icon appears in your taskbar with battery %, quick toggles for EQ, sidetone, THX, and ANC, and a Daemon submenu if you need to restart it.

**4. Open the settings GUI:**
```bash
blackshark-gui
```
All settings are applied immediately and sync back to the tray in real time.

**5. Add the tray to autostart** so it launches on login:
```bash
install -m644 pkg/blackshark-tray.desktop ~/.config/autostart/
```
Or on KDE: Settings → Autostart → Add Application → search "BlackShark Tray".

---

## Usage

### Daemon

```bash
systemctl --user status blacksharkd
systemctl --user restart blacksharkd
```

### CLI

```bash
blackshark-ctl status           # human-readable status
blackshark-ctl status --json    # JSON output for waybar/scripts
blackshark-ctl battery          # battery percentage and charging state
blackshark-ctl sidetone <0-15>  # set sidetone level
blackshark-ctl eq <0-8>         # set EQ preset (0 = Flat)
blackshark-ctl thx <on|off>     # toggle THX Spatial Audio
blackshark-ctl anc <on|off> [level]  # toggle ANC, optional level 1-4
blackshark-ctl power-savings <0|15|30|45|60>  # auto-off timeout in minutes
blackshark-ctl monitor          # stream live D-Bus property changes
```

### System tray

```bash
blackshark-tray &
```

Add to your desktop autostart. Shows battery % in the tooltip, quick toggles and submenus for all settings in the menu, and a Daemon submenu to start/stop/restart the daemon.

### GUI

```bash
blackshark-gui
```

Full settings panel. All changes are applied immediately via D-Bus and sync back to the tray and CLI in real time. The Advanced tab has daemon controls, a live log viewer, and an opt-in toggle for the experimental PipeWire game/chat mix feature.

---

## Architecture

```
blackshark-ctl  (CLI)  ──┐
blackshark-tray (tray) ──┤  D-Bus: net.blackshark1 (session bus)
blackshark-gui  (GUI)  ──┘
                          │
                    blacksharkd  (systemd user service)
                          │
                    /dev/hidraw*  (hidapi)
                          │
                    BlackShark V3 Pro dongle (USB)
```

The daemon owns the HID device exclusively. All other tools talk to it over D-Bus (`net.blackshark1`, session bus, path `/net/blackshark1/Headset`). No tool other than the daemon touches `/dev/hidraw*`.

---

## Repository layout

```
crates/
  protocol/          HID report format and command constants
  device/            hidapi open/send/recv
  blackshark-client/ zbus D-Bus proxy (shared by CLI, tray, GUI)
  blacksharkd/       daemon: HID ownership, D-Bus service, battery polling
  blackshark-ctl/    CLI client
  blackshark-tray/   ksni system tray
  blackshark-gui/    slint settings GUI
pkg/
  99-blackshark.rules      udev rule
  blacksharkd.service      systemd user unit
  blackshark-gui.desktop   app launcher entry (copy to ~/.local/share/applications/)
  blackshark-tray.desktop  autostart entry (copy to ~/.config/autostart/)
install.sh                 one-shot build + install script
dev-install-nix.sh         Development-only local NixOS build + install script
shell.nix                  Nix development shell and Rust dependencies
```

---

## udev rule

For the standard `install.sh` installer, the udev rule grants the `users` group read/write access to the headset's HID interface:

```
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="1532", ATTRS{idProduct}=="0577", MODE="0660", GROUP="users"
```

Make sure your user is in the `users` group (`groups $USER`). If not:

```bash
sudo usermod -aG users $USER
# log out and back in, then re-run install.sh
```

For NixOS, use the package's udev rule as described in [Install for NixOS](#install-for-nixos).

---

## CI

GitHub Actions runs on every push:
- `cargo fmt --check`
- `cargo clippy -D warnings`
- `cargo build --all`
- `cargo test --all`

Security audit runs weekly via `cargo audit`. Release builds for `x86_64` are produced automatically on version tags.

---

## Device info

- USB VID/PID: `0x1532` / `0x0577`
- HID reports: 64 bytes, report ID `0x02`
- Interface: HID interface 5, endpoint `0x84`
- Protocol: custom Razer HID (not HID++ or OpenRazer-compatible)
