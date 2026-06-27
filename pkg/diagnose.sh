#!/usr/bin/env bash
# Collects everything needed to debug a "headset not connected" report.
# Usage: ./diagnose.sh > blackshark-diagnostics.txt   (then attach it to the issue)
# Contains no secrets; review before posting if you like.

section() { printf '\n===== %s =====\n' "$*"; }
run() { printf '$ %s\n' "$*"; "$@" 2>&1; }

section "system"
run uname -srm
grep -E '^(PRETTY_NAME|VERSION_ID)=' /etc/os-release
echo "session: ${XDG_SESSION_TYPE:-?} / ${XDG_CURRENT_DESKTOP:-?}"
run id

section "installed binaries"
for b in blacksharkd blackshark-ctl blackshark-tray blackshark-gui; do
    which -a "$b" 2>/dev/null | awk '!seen[$0]++' | grep . || echo "$b: NOT FOUND"
done
# Not blacksharkd: releases before 0.1.5 have no --version and would start a second daemon.
command -v blackshark-ctl >/dev/null && run blackshark-ctl --version

section "usb (Razer devices)"
lsusb 2>/dev/null | grep -i 1532 || echo "no Razer USB devices found"
for pid in 0577 0a55; do
    lsusb -d "1532:$pid" -v 2>/dev/null | grep -E 'bcdDevice|iProduct|bInterfaceNumber|bInterfaceClass' | sort -u
done

section "hidraw nodes for 1532:0577 / 1532:0a55"
found=0
for n in /dev/hidraw*; do
    props=$(udevadm info -q property "$n" 2>/dev/null)
    grep -qE '^ID_MODEL_ID=(0577|0a55)$' <<<"$props" || continue
    found=1
    echo "$n iface=$(grep ID_USB_INTERFACE_NUM <<<"$props" | cut -d= -f2)"
    ls -l "$n"
    getfacl -p "$n" 2>/dev/null | grep -E '^(user|group):' || true
    [[ -r "$n" && -w "$n" ]] && echo "  access: OK" || echo "  access: DENIED for $(id -un)"
done
(( found )) || echo "no hidraw node found: dongle not plugged in, or udev rule not applied (replug the dongle)"

section "udev rule"
run cat /etc/udev/rules.d/99-blackshark.rules

section "audio devices"
(pactl list short sinks 2>/dev/null | grep -i -E 'razer|blackshark') || echo "no BlackShark audio sink (headset audio path not up?)"

section "daemon"
run systemctl --user status blacksharkd --no-pager -l
run busctl --user introspect net.blackshark1 /net/blackshark1/Headset net.blackshark1.Headset
run blackshark-ctl status

section "daemon log (last 80 lines)"
journalctl --user -u blacksharkd -n 80 --no-pager 2>&1
