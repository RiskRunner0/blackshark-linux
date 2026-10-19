{ pkgs ? import <nixpkgs> {} }:

pkgs.rustPlatform.buildRustPackage rec {
  pname = "blackshark-linux";
  version = "0.1.4";

  src = pkgs.fetchFromGitHub {
    owner = "RiskRunner0";
    repo = "blackshark-linux";
    rev = "v${version}";
    hash = "sha256-/NtpjfFJ9TPqEIDzWMSm2TfITbQeAe0BL17U8lWKTrU=";
  };

  cargoLock.lockFile = "${src}/Cargo.lock";

  cargoBuildFlags = [
    "-p" "blacksharkd"
    "-p" "blackshark-ctl"
    "-p" "blackshark-tray"
    "-p" "blackshark-gui"
  ];

  nativeBuildInputs = with pkgs; [
    pkg-config
    makeWrapper
    copyDesktopItems
  ];

  desktopItems = [
    (pkgs.makeDesktopItem {
      name = "blackshark-gui";
      exec = "blackshark-gui";
      icon = "blackshark-gui";
      desktopName = "BlackShark GUI";
      categories = [ "Settings" "Utility" ];
    })
  ];

  buildInputs = with pkgs; [
    hidapi
    libusb1
    dbus
    systemd
    wayland
    libxkbcommon
    libglvnd
    mesa
    fontconfig
    freetype
    libpng
    expat
    openssl
  ];

  doCheck = false;

  installPhase = ''
    runHook preInstall

    # ---------------------------------------------------------------
    # Locate Cargo binaries
    # ---------------------------------------------------------------

    install_binary() {
      local name="$1"
      local source

      source="$(find target -type f \
        -path "*/release/$name" \
        -print -quit)"

      if [ -z "$source" ]; then
        echo "ERROR: $name binary not found" >&2
        echo "Available binaries:" >&2
        find target -type f -path '*/release/*' -print >&2 || true
        exit 1
      fi

      install -Dm755 "$source" "$out/bin/$name"
    }

    install_binary blacksharkd
    install_binary blackshark-ctl
    install_binary blackshark-tray
    install_binary blackshark-gui

    # ---------------------------------------------------------------
    # udev rule
    # ---------------------------------------------------------------

    install -Dm644 \
      pkg/99-blackshark.rules \
      "$out/lib/udev/rules.d/99-blackshark.rules"

    substituteInPlace \
      "$out/lib/udev/rules.d/99-blackshark.rules" \
      --replace-fail \
      'GROUP="users"' \
      'GROUP="input"'

    # ---------------------------------------------------------------
    # Tray autostart entry
    # ---------------------------------------------------------------

    if [ -f pkg/blackshark-tray.desktop ]; then
      install -Dm644 \
        pkg/blackshark-tray.desktop \
        "$out/share/autostart/blackshark-tray.desktop"

      substituteInPlace \
        "$out/share/autostart/blackshark-tray.desktop" \
        --replace-fail \
        "Exec=blackshark-tray" \
        "Exec=$out/bin/blackshark-tray"
    fi

    # ---------------------------------------------------------------
    # GUI runtime libraries
    # ---------------------------------------------------------------

    wrapProgram "$out/bin/blackshark-gui" \
      --prefix LD_LIBRARY_PATH : "${pkgs.lib.makeLibraryPath [
        pkgs.wayland
        pkgs.libxkbcommon
        pkgs.libglvnd
        pkgs.mesa
        pkgs.fontconfig
        pkgs.freetype
        pkgs.libpng
        pkgs.expat
        pkgs.openssl
      ]}"

    runHook postInstall
  '';

  meta = with pkgs.lib; {
    description =
      "Linux userspace driver for the Razer BlackShark V3 Pro wireless headset";

    homepage =
      "https://github.com/RiskRunner0/blackshark-linux";

    license = licenses.mit;

    platforms = platforms.linux;
  };
}