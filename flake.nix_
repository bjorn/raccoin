{
  description = "raccoin flake";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs";

  outputs = { self, nixpkgs }: let
    system = "x86_64-linux";
    pkgs = import nixpkgs { inherit system; };
  in {
    #~ packages.${system}.default = pkgs.callPackage ./default.nix { };
    packages.${system}.default = pkgs.callPackage ./default.nix { };
    apps.${system}.raccoin = {
      type = "app";
      program = "${self.packages.${system}.default}/bin/raccoin";
    };
  };
}
