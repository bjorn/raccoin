# nix-build
{
  pkgs ? import <nixpkgs> { },
}:

let
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
    ++ lib.optionals stdenv.isLinux [
      wayland
      xorg.libX11
      xorg.libXcursor
      xorg.libXi
      xorg.libXrandr
    ];
in

pkgs.rustPlatform.buildRustPackage rec {
  pname = "raccoin";
  version = "dev";
  # force rebuild
  NIX_REBUILD_TIMESTAMP = "2025-11-22T23:22";

  #~ src = ./.;
  src = pkgs.lib.cleanSourceWith {
    filter =
      path: type:
      # allow everything in default cleanSource
      pkgs.lib.cleanSourceFilter path type
      # plus force-include the icons directory
      || builtins.match ".*raccoin_ui/ui/icons/.*" path != null;

    src = ./.;
  };

  cargoLock.lockFile = ./Cargo.lock;

  doCheck = false;

  #~ cargoInstallHook = false;
  #~ doInstallCheck = false;    # optional
  #~ installPhase = ''
  #~ mkdir -p $out/bin
  #~ cp target/release/raccoin $out/bin/
  #~ '';

  nativeBuildInputs = with pkgs; [
    pkg-config
    patchelf
    clang
    cmake
    makeWrapper
  ];

  buildInputs = commonInputs ++ [
    #    pkgs.skia
  ];

  preBuild = ''
    #    export SKIA_USE_SYSTEM_LIBRARIES=1
    #    export SKIA_DIR=${pkgs.skia}
    #    export SKIA_LIBRARY_PATH=${pkgs.skia}/lib
    #    export SKIA_INCLUDE_PATH=${pkgs.skia}/include
        export SLINT_BACKEND=winit
        
        ROOT_CARGO_TOML=$PWD/Cargo.toml
        if [ -f "$ROOT_CARGO_TOML" ]; then
          echo "Patching Cargo.toml to use renderer-software instead of renderer-skia"
          sed -i '/slint =/ s/renderer-skia/renderer-software/' $ROOT_CARGO_TOML
        else
          echo "Error: $ROOT_CARGO_TOML not found!"
          exit 1
        fi
  '';

  #~ buildPhase = ''
  #export CARGO_TARGET_DIR=$PWD/target

  #cargo build --release
  #~ '';

  #~ postBuild = ''
  #~ mkdir -p $PWD/target/xkb-libs
  #~ ln -sf ${pkgs.libxkbcommon}/lib/libxkbcommon.so $PWD/target/xkb-libs/libxkbcommon-x11.so
  #~ '';

  #~ mkdir -p $out/bin
  #~ install -m755 target/release/raccoin $out/bin/raccoin
  postInstall = ''
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

}
