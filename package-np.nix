{
  lib,
  rustPlatform,
  fetchFromGitHub,
  harfbuzz,
  fontconfig,
  makeDesktopItem,
  cairo,
  pango,
  freetype,
  alsa-lib,
  libGL,
  openssl,
  libxkbcommon,
  pkg-config,
  makeWrapper,
  xorg,
  wayland,
}:

let

  desktopItem = makeDesktopItem {
    name = "Raccoin";
    exec = "raccoin";
    icon = "raccoin-com.github.bjorn";
    comment = "Raccoin Crypto/Bitcoin Tax Tool";
    desktopName = "Raccoin";
    categories = [
      "Office"
      "Finance"
      "Utility"
    ];
    terminal = false;
    startupWMClass = "Raccoin";
    keywords = [
      "bitcoin"
      "crypto"
      "tax"
      "raccoin"
    ];
    mimeTypes = [
      "text/csv"
      "application/json"
    ];
  };

in
rustPlatform.buildRustPackage (finalAttrs: {
  pname = "raccoin";
  version = "0.2.0";

  src = fetchFromGitHub {
    owner = "bjorn";
    repo = "raccoin";
    rev = "v${finalAttrs.version}";
    hash = "sha256-6BwRFU8qU6K0KqKdK+soKcWU2LPxkKKPOcn2gupunGg=";
  };

  cargoHash = "sha256-pz3dwIIBQZIwTammiI/UQwM0Iy1ZgC9ntK+qNGv3s24=";

  nativeBuildInputs = [
    pkg-config
    makeWrapper
  ];

  buildInputs = [
    cairo
    pango
    harfbuzz
    fontconfig
    freetype
    alsa-lib
    libGL
    openssl
    libxkbcommon
    wayland
    xorg.libX11
    xorg.libXcursor
    xorg.libXi
    xorg.libXrandr
  ];

  doCheck = false;

  preBuild = ''
    export SLINT_BACKEND=winit
    ROOT_CARGO_TOML="$PWD/Cargo.toml"
    if [ -f "$ROOT_CARGO_TOML" ]; then
      sed -i '/slint =/ s/renderer-skia/renderer-software/' "$ROOT_CARGO_TOML"
    fi
  '';

  postInstall = ''
            #DESKTOP_FILE="$out/share/applications/raccoin-com.github.bjorn.desktop"
            ICON_FILE="$out/share/icons/hicolor/scalable/apps/raccoin-com.github.bjorn.svg"
            if [ "$(uname -s)" = "Linux" ]; then
              mkdir -p "$out/share/applications"
              cp ${desktopItem}/share/applications/* $out/share/applications
              mkdir -p "$out/share/icons/hicolor/scalable/apps"
              install -Dm644 "$src/raccoin_ui/ui/icons/app-icon.svg" \
                "$ICON_FILE"
            fi

            if [ "$(uname -s)" = "Darwin" ]; then
              APP_DIR="$out/Raccoin.app"
              mkdir -p "$APP_DIR/Contents/MacOS"
              mkdir -p "$APP_DIR/Contents/Resources"
              cat > "$APP_DIR/Contents/Info.plist" <<EOF
    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
    <plist version="1.0">
    <dict>
      <key>CFBundleName</key><string>Raccoin</string>
      <key>CFBundleExecutable</key><string>raccoin</string>
      <key>CFBundleIdentifier</key><string>org.github.bjorn.raccoin</string>
      <key>CFBundleVersion</key><string>${finalAttrs.version}</string>
      <key>CFBundleIconFile</key><string>app-icon.icns</string>
    </dict>
    </plist>
    EOF
              cp "$out/bin/raccoin" "$APP_DIR/Contents/MacOS/raccoin"
        	  if command -v convert >/dev/null 2>&1; then
        		convert "$src/raccoin_ui/ui/icons/app-icon.svg" -resize 256x256 "$APP_DIR/Contents/Resources/app-icon.icns"
        	  fi
            fi
  '';

  postFixup = ''
    wrapProgram "$out/bin/raccoin" \
      --set-default SLINT_BACKEND winit \
      --prefix LD_LIBRARY_PATH : ${
        lib.makeLibraryPath [
          fontconfig
          libxkbcommon
          openssl
          xorg.libX11
        ]
      }
  '';

  passthru.updateScript = nix-update-script { };

  meta = {
    description = "Crypto portfolio & capital-gains reporting tool (Rust + Slint)";
    longDescription = ''
      This GUI Tool enables accounting with the quite basic principle
      "first in, first out" (FIFO) in multiple wallets. It is for
      e.g. bitcoiners that actually use the currency and want
      reports on their holdings.
    '';
    homepage = "https://raccoin.org";
    changelog = "https://github.com/bjorn/raccoin/releases/tag/v${finalAttrs.version}";
    license = lib.licenses.gpl3Plus;
    mainProgram = "raccoin";
    platforms = [
      "x86_64-linux"
      "aarch64-linux"
      "x86_64-darwin"
      "aarch64-darwin"
    ];
    maintainers = with lib.maintainers; [ vv01f ];
  };
})
