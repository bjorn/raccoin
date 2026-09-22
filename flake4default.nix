# nix build
{
  description = "Raccoin flake";

  inputs = {
    # Nixpkgs unstable für Rust >= 1.89
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs =
    { self, nixpkgs }:
    let
      # Multi-Arch: hier z.B. x86_64 und aarch64
      systems = [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
      mkPackage =
        system:
        let
          pkgs = import nixpkgs { inherit system; };
        in
        pkgs.callPackage ./default.nix { };
    in
    {
      # Default-Paket für `nix profile add .`
      defaultPackage = builtins.listToAttrs (
        map (system: {
          name = system;
          value = mkPackage system;
        }) systems
      );

      # Zugriff über packages.<system>.raccoin
      packages = builtins.listToAttrs (
        map (system: {
          name = system;
          value = {
            raccoin = mkPackage system;
          };
        }) systems
      );
    };
}
