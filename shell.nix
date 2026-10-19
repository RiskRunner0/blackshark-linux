{ pkgs ? import <nixpkgs> {} }:

pkgs.mkShell {
  packages = with pkgs; [
    rustc
    cargo
    pkg-config

    # HID
    hidapi
    libusb1

    # D-Bus
    dbus

    # GUI / font dependencies
    fontconfig
    freetype
    libpng
    expat

    # Other native dependencies
    openssl

    # systemd
    systemd
  ];
}

