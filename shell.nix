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

  commonBuildInputs = with pkgs; [
    rustc
    cargo
    pkg-config
    patchelf
    cmake
    ninja
    icu
    expat
    libjpeg
    libpng
    clang
    gcc
    rustup
  ];

  pkgConfigLibs = with pkgs; [
    freetype
    fontconfig
    harfbuzz
    icu
    expat
    libjpeg
    libpng
  ];

  pkgConfigPath = pkgs.lib.concatStringsSep ":" (map (p: "${p.dev}/lib/pkgconfig") pkgConfigLibs);

in
pkgs.mkShell {
  buildInputs = commonInputs ++ commonBuildInputs;

  shellHook = ''
    export LD_LIBRARY_PATH=${pkgs.libxkbcommon}/lib:$LD_LIBRARY_PATH
    ln -sf ${pkgs.libxkbcommon}/lib/libxkbcommon.so ${pkgs.libxkbcommon}/lib/libxkbcommon-x11.so

    export PKG_CONFIG_PATH=${pkgConfigPath}
    echo "PKG_CONFIG_PATH set to $PKG_CONFIG_PATH"
    echo "Default dev shell loaded. Incremental builds in ./target/"

    export CARGO_TARGET_DIR=$PWD/target

    alias shell-release='export CARGO_TARGET_DIR=$PWD/target-release && export RUSTFLAGS="-C opt-level=3" && echo "Release shell active"'

    patch_raccoin() {
      local bin="$CARGO_TARGET_DIR/release/raccoin"
      if [ -f "$bin" ]; then
        patchelf --set-rpath "${pkgs.lib.makeLibraryPath commonInputs}" "$bin"
        echo "Patched RPATH for $bin"
      fi
    }

    cargo() {
      if [ "$1" = "build" ] && [ "$2" = "--release" ]; then
        command cargo "$@"
        patch_raccoin
      else
        command cargo "$@"
      fi
    }
  '';
}
