{
  description = "raccoin flake";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs";

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
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
        ++ lib.optionals pkgs.stdenv.isLinux [
          wayland
          xorg.libX11
          xorg.libXcursor
          xorg.libXi
          xorg.libXrandr
        ];
      commonBuildInputs =
        with pkgs;
        [
          rustc
          cargo
          pkg-config
          patchelf
        ];
    in
    {
      # Release build (for `nix build`)
      packages.${system}.default = pkgs.rustPlatform.buildRustPackage {
        pname = "raccoin";
        version = "0.2.0";

        src = ./.;

        cargoLock.lockFile = ./Cargo.lock;

        nativeBuildInputs = [ pkgs.pkg-config ];
        buildInputs = commonInputs;

        doCheck = false;

        postFixup = ''
          patchelf --set-rpath "${pkgs.lib.makeLibraryPath commonInputs}" \
                   $out/bin/raccoin
        '';
      };

      # Dev shell (for `nix develop`)
      devShells.${system} = {
        # debug dev shell (fast incremental builds)
        default = pkgs.mkShell {
          buildInputs = commonInputs ++ commonBuildInputs;

          shellHook = ''
            export CARGO_TARGET_DIR=$PWD/target
            echo "Debug dev shell: incremental builds in ./target/"
          '';
        };

        # release dev shell (optimized builds + patched RPATH)
        release = pkgs.mkShell {
		  packages = [ pkgs.nix-update ];

          buildInputs = commonInputs ++ commonBuildInputs;

          shellHook = ''
            export CARGO_TARGET_DIR=$PWD/target-release
            export RUSTFLAGS="-C opt-level=3"
            echo "Release dev shell: optimized builds in ./target-release/"

            # helper to patch RPATH of raccoin binary
            patch_raccoin() {
              local bin="$CARGO_TARGET_DIR/release/raccoin"
              if [ -f "$bin" ]; then
                patchelf --set-rpath "${pkgs.lib.makeLibraryPath commonInputs}" "$bin"
                echo "Patched RPATH for $bin"
              fi
            }

            # wrap cargo so patchelf runs after release builds
            cargo() {
              if [ "$1" = "build" ] && [ "$2" = "--release" ]; then
                command cargo "$@"
                patch_raccoin
              else
                command cargo "$@"
              fi
            }
          '';
        };

      };

      # For `nix run`
      apps.${system}.default = {
        type = "app";
        program = "${self.packages.${system}.default}/bin/raccoin";
      };
    };
}
