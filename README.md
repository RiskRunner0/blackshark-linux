# blackshark-ctl

Linux userspace driver for the **Razer BlackShark V3 Pro** (PC and Xbox editions) and
**Razer BlackShark V2 X HyperSpeed (V2 HS)** wireless headsets.
Controls sidetone, EQ presets, THX Spatial Audio, ANC and power savings without Razer Synapse or Windows.

![Device tab showing battery, connection status and audio controls](assets/Device_page.png)

## Features

- Sidetone (0–15), EQ presets (9), THX Spatial Audio, ANC (on/off, level 1–4), power-savings timeout
- Battery percentage and charging state
- Settings persist in `~/.config/blackshark/config.toml` and are restored on reconnect
- System tray, settings GUI and a scriptable CLI (`status --json` for waybar)

![System tray menu with headset controls and daemon status](assets/tray_icon.png)

## Install

Requires Linux with a systemd user session. Tested on Arch/CachyOS (KDE, Wayland).

1. Download the latest tarball from [Releases](https://github.com/RiskRunner0/blackshark-linux/releases).
2. Run:
   ```bash
   tar xzf blackshark-ctl-*.tar.gz && cd blackshark-ctl-*
   ./install.sh
   ```
3. Unplug and re-plug the dongle.

To build from source instead, clone the repo and run `./install.sh` (needs [Rust](https://rustup.rs)).
The script installs binaries to `~/.local/bin`, starts `blacksharkd` as a systemd user service, and
installs the udev rule (needs `sudo`).

**Firmware 1.3.x or later is required** on both the headset and the dongle (update via Synapse on Windows).

To remove everything: `./uninstall.sh` (add `--purge` to also delete your settings).

## Usage

```bash
blackshark-ctl status [--json]
blackshark-ctl battery
blackshark-ctl sidetone <0-15>
blackshark-ctl eq <0-8>                       # 0 = Flat
blackshark-ctl thx <on|off>
blackshark-ctl anc <on|off> [level 1-4]
blackshark-ctl power-savings <0|15|30|45|60>  # minutes
blackshark-ctl monitor                        # stream live changes
```

`blackshark-gui` opens the settings panel (its Advanced tab has daemon controls, a log viewer and the
experimental PipeWire game/chat mix). `blackshark-tray` shows battery and quick toggles; to start it on
login: `install -m644 pkg/blackshark-tray.desktop ~/.config/autostart/`.

## Troubleshooting

`blackshark-ctl status` says "headset is not connected"? Check the daemon log:

```bash
journalctl --user -u blacksharkd -f
```

| Log line | Meaning |
|---|---|
| `cannot open dongle: ... not visible` | Dongle unplugged, or no permission on `/dev/hidraw*`. Replug it after installing. |
| `cannot open dongle: ... no HID interface 5` | Different interface layout on your firmware. Please open an issue. |
| `waiting for headset ...` | Dongle is fine but the headset isn't answering. See below. |
| `headset connected` | Working. |

If it's stuck on "waiting for headset", the dongle isn't linked to the headset (audio can still play,
because the dongle is also a USB sound card). Check that the headset is on, in 2.4GHz mode (green LED,
not white/Bluetooth), and on firmware 1.3.x+. If it lost its pairing, re-pair it with Razer's pairing
tool on Windows ([#7](https://github.com/RiskRunner0/blackshark-linux/issues/7)).

When opening an issue, attach the output of:

```bash
pkg/diagnose.sh > blackshark-diagnostics.txt
```

For protocol-level logs, run the daemon in the foreground:

```bash
systemctl --user stop blacksharkd
RUST_LOG=blacksharkd=debug,blackshark_device=debug blacksharkd
```

## How it works

```
blackshark-ctl  (CLI)  ──┐
blackshark-tray (tray) ──┤  D-Bus: net.blackshark1 (session bus)
blackshark-gui  (GUI)  ──┘
                          │
                    blacksharkd  (systemd user service)
                          │
                    /dev/hidraw*  (hidapi)
                          │
                    BlackShark headset dongle (USB)
```

`blacksharkd` is the only process that touches `/dev/hidraw*`; everything else talks to it over D-Bus.
Crates live in `crates/` (`protocol`, `device`, `blackshark-client`, `blacksharkd`, and the three front ends).

## Device info and udev

- USB VID: `0x1532` (Razer USA, Ltd)
- Supported PIDs:
  - `0x0577` (Razer BlackShark V3 Pro)
  - `0x0a55` (Razer BlackShark V3 Pro for Xbox)
  - `0x0565` (Razer BlackShark V2 HS 2.4)
- HID reports: 64 bytes, report ID `0x02`
- Interface: HID control interface (interface 5 for V3 Pro, interface 3 for V2 HS)
- Protocol: custom Razer HID (not HID++ or OpenRazer-compatible)

The udev rule in `pkg/99-blackshark.rules` grants the active local user and members
of the `users` group read/write access to the headset's HID interface:

```
# Razer BlackShark V3 Pro (PC and Xbox)
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="1532", ATTRS{idProduct}=="0577|0a55", TAG+="uaccess", MODE="0660", GROUP="users"
# Razer BlackShark V2 HS 2.4
SUBSYSTEM=="hidraw", ATTRS{idVendor}=="1532", ATTRS{idProduct}=="0565", TAG+="uaccess", MODE="0660", GROUP="users"
```

Make sure your user is in the `users` group (`groups $USER`) if your session does not
receive the `uaccess` ACL:

```bash
sudo usermod -aG users $USER
# log out and back in, then re-run install.sh
```

## Device Support Matrix & Limitations

| Feature | Razer BlackShark V3 Pro | Razer BlackShark V2 HS |
| :--- | :---: | :---: |
| **Connection & Battery** | Yes | Yes |
| **Sidetone (Mic Monitoring)** | Yes | Yes (hardware support, volume can be low) |
| **Power Savings (Auto Off)** | Yes | Yes |
| **PipeWire Game/Chat Mix** | Yes | Yes (requires client-side routing) |
| **Hardware EQ Presets** | Yes | No (dongle returns Timeout status) |
| **Active Noise Cancellation** | Yes | No (no ANC hardware) |
| **THX Spatial Audio** | Yes | No (software-only feature) |

> [!NOTE]
> For headsets with limited or missing hardware EQ support (like the V2 HS), use
> **EasyEffects** on Linux for custom software equalization. Unsupported hardware settings
> such as THX or ANC produce a warning instead of disconnecting the V2 HS.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
