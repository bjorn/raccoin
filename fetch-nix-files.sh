#!/usr/bin/env sh
set -euo pipefail

REPO="github:vv01f/raccoin-nix"
FILES=(
  shell.nix
  flake.nix
  flake-compat.nix
  default.nix
  package.nix
  package-np.nix
)

printf "🔍 Generate Tarball from $REPO...\n"
TARBALL=$(nix flake archive --json "$REPO" | jq -r .path)

printf "📦 Unpacking files:\n"
printf "  - %s\n" "${FILES[@]}"

tar -C . -xvf "$TARBALL" "${FILES[@]}"

printf "✅ Done.\n"
