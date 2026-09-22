#!/usr/bin/env nix-shell
#!nix-shell -i bash -p nix-prefetch-github jq cargo

set -euo pipefail

owner="bjorn"
repo="raccoin"

# version from tags, remove annotated tags (ending with '^{}')
latest_tag="$(git ls-remote --tags https://github.com/$owner/$repo.git \
  | grep -v "{}$" \
  | grep -o 'refs/tags/v[0-9].*' \
  | sed 's|refs/tags/||' \
  | sort -V \
  | tail -n1)"

printf "%s\n" "Latest tag: $latest_tag"

src_hash=$(nix-prefetch-github "$owner" "$repo" --rev "$latest_tag" | jq -r .hash)

# todo
#~ cargoHash = ""

cat <<EOF
{
  version = "${latest_tag#v}";
  rev = "$latest_tag";
  src_hash = "$src_hash";
}
EOF
  #~ cargo_hash = "$cargo_hash";
