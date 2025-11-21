# default.nix
{ pkgs ? import <nixpkgs> {} }:
pkgs.callPackage ./default.nix {}
