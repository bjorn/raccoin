# nix-build -E 'with import <nixpkgs> {}; callPackage ./package.nix {}'
# nix-build -E 'with import <nixpkgs> {}; callPackage ./package.nix { useLatest = true; }'
# nix-build -E 'with import <nixpkgs> {}; callPackage ./package.nix { useLatest = true; buildBranch = "newfeature"; }'
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
  patchelf,
  xorg,
  wayland,
  useLatest ? false,
  buildBranch ? "master",
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
  version = "0.2.0"; # last release
  NIX_REBUILD_TIMESTAMP = "2025-12-17T13:42";

  src =
    if useLatest then
      fetchGit {
        url = "https://github.com/bjorn/raccoin.git";
        ref = buildBranch;
      }
    else
      fetchFromGitHub {
        owner = "bjorn";
        repo = "raccoin";
        rev = "v${finalAttrs.version}";
        hash = "sha256-6BwRFU8qU6K0KqKdK+soKcWU2LPxkKKPOcn2gupunGg=";
      };

  cargoHash =
    if useLatest then
      "sha256-K7y8RHk+S+hY0j3QX6o82AYDU2WrthSn1NCeb+Es8u8=" # for new hash use ""
    # v0.2.0
    else
      "sha256-pz3dwIIBQZIwTammiI/UQwM0Iy1ZgC9ntK+qNGv3s24=";
  #~ cargoHash = "";

  nativeBuildInputs = [
    pkg-config
    makeWrapper
    patchelf
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

  # upstream has no tests
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

  # better use wrapProgram than patchelf
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
  #~ postFixup = ''
  #~ patchelf --set-rpath "${
  #~ lib.makeLibraryPath (
  #~ [
  #~ fontconfig
  #~ libxkbcommon
  #~ openssl
  #~ xorg.libX11
  #~ ]
  #~ )
  #~ }" \
  #~ $out/bin/raccoin
  #~ '';

  #~ passthru.updateScript = nix-update-script { };

  meta = {
    description = "Crypto portfolio & capital-gains reporting tool (Rust + Slint)";
    longDescription = ''
      This GUI Tool enables accounting with the quite basic principle
      "first in, first out" (FIFO) in multiple wallets. It is for
      e.g. bitcoiners that actually use the currency and want
      reports on their holdings.
    '';
    homepage = "https://raccoin.org";
    changelog =
      if !useLatest then "https://github.com/bjorn/raccoin/releases/tag/v${finalAttrs.version}" else "";
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
