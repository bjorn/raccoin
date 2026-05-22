# nix build
# nix profile add .
{
  description = "Raccoin flake";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      mkPkgForSystem =
        system:
        let
          pkgs = import nixpkgs { inherit system; };
          commonInputs =
            with pkgs;
            [
              cairo
              pango
              harfbuzz
              fontconfig
              freetype
              alsa-lib
              libGL
              openssl
              libxkbcommon
            ]
            ++ pkgs.lib.optionals pkgs.stdenv.isLinux [
              wayland
              xorg.libX11
              xorg.libXcursor
              xorg.libXi
              xorg.libXrandr
            ];

          mkPackage = pkgs.rustPlatform.buildRustPackage rec {
            pname = "raccoin";
            version = "dev";
            NIX_REBUILD_TIMESTAMP = "2025-11-23T00:06";

            src = pkgs.lib.cleanSourceWith {
              filter =
                path: type:
                pkgs.lib.cleanSourceFilter path type || builtins.match ".*raccoin_ui/ui/icons/.*" path != null;
              src = ./.;
            };

            cargoLock.lockFile = ./Cargo.lock;
            doCheck = false;

            nativeBuildInputs = with pkgs; [
              pkg-config
              patchelf
              clang
              cmake
              makeWrapper
            ];
            buildInputs = commonInputs ++ pkgs.lib.optionals pkgs.stdenv.isLinux [ pkgs.xdg-utils ];

            preBuild = ''
              export SLINT_BACKEND=winit
              ROOT_CARGO_TOML=$PWD/Cargo.toml
              if [ -f "$ROOT_CARGO_TOML" ]; then
                sed -i '/slint =/ s/renderer-skia/renderer-software/' $ROOT_CARGO_TOML
              fi
            '';

            postInstall = ''
                            if [[ "$system" == *-linux ]]; then
                              mkdir -p $out/share/applications
                              cat > $out/share/applications/raccoin-com.github.bjorn.desktop <<EOF
[Desktop Entry]
Name=Raccoin
Comment=Raccoin Crypto/Bitcoin Tax Tool
Exec=$out/bin/raccoin
Icon=raccoin-com.github.bjorn.svg
Terminal=false
Type=Application
Categories=Office;Finance;Tax;Utility;
Keywords=bitcoin;crypto;tax;raccoin;
MimeType=text/csv;application/json
StartupWMClass=Raccoin
URL=https://raccoin.org
EOF
                  
                              mkdir -p $out/share/icons/hicolor/scalable/apps
                              install -Dm644 "$src/raccoin_ui/ui/icons/app-icon.svg" \
                                "$out/share/icons/hicolor/scalable/apps/raccoin-com.github.bjorn.svg"
                  
                            fi
                  
                            if [[ "$system" == *-darwin ]]; then
                              APP_DIR=$out/Raccoin.app
                              mkdir -p $APP_DIR/Contents/MacOS
                              mkdir -p $APP_DIR/Contents/Resources
                              cat > $APP_DIR/Contents/Info.plist <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Raccoin</string>
  <key>CFBundleExecutable</key><string>raccoin</string>
  <key>CFBundleIdentifier</key><string>org.github.bjorn.raccoin</string>
  <key>CFBundleVersion</key><string>dev</string>
  <key>CFBundleIconFile</key><string>app-icon.icns</string>
</dict>
</plist>
EOF
                              cp $out/bin/raccoin $APP_DIR/Contents/MacOS/raccoin
                              if command -v convert >/dev/null 2>&1; then
                                convert "$src/raccoin_ui/ui/icons/app-icon.svg" -resize 256x256 "$APP_DIR/Contents/Resources/app-icon.icns"
                              fi
                            fi
            '';

            postFixup = ''
              wrapProgram $out/bin/raccoin \
                --set-default SLINT_BACKEND winit \
                --prefix LD_LIBRARY_PATH : ${
                  pkgs.lib.makeLibraryPath (
                    commonInputs
                    ++ [
                      pkgs.libxkbcommon
                      pkgs.xorg.libX11
                      pkgs.fontconfig
                    ]
                  )
                }
            '';
          };
        in
        mkPackage;
    in
    {
      packages = builtins.listToAttrs (
        map (system: {
          name = system;
          value = mkPkgForSystem system;
        }) systems
      );

      defaultPackage.x86_64-linux = mkPkgForSystem "x86_64-linux";
      defaultPackage.aarch64-linux = mkPkgForSystem "aarch64-linux";
      defaultPackage.x86_64-darwin = mkPkgForSystem "x86_64-darwin";
      defaultPackage.aarch64-darwin = mkPkgForSystem "aarch64-darwin";
    };
}
