#!/bin/sh
# Installs orcux in ~/.local/bin (or $ORCUX_BIN): the latest version, or the one in $ORCUX_VERSION (0.1.0).
set -eu
repo=https://github.com/borjamrd/orcux
bin=${ORCUX_BIN:-$HOME/.local/bin}

fail() { echo "orcux: $1" >&2; exit 1; }

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  Linux-x86_64) target=x86_64-unknown-linux-musl ;;
  Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-musl ;;
  *) fail "no binary for $(uname -s) $(uname -m): try cargo install orcux" ;;
esac
file=orcux-$target.tar.gz
from=${ORCUX_URL:-$repo/releases/${ORCUX_VERSION:+download/v$ORCUX_VERSION}}
[ -n "${ORCUX_VERSION:-}" ] || [ -n "${ORCUX_URL:-}" ] || from=$repo/releases/latest/download

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL "$from/$file" -o "$tmp/$file" || fail "cannot download $from/$file"
curl -fsSL "$from/SHA256SUMS" -o "$tmp/SHA256SUMS" || fail "cannot download the checksums"

want=$(awk -v f="$file" '$2 == f { print $1 }' "$tmp/SHA256SUMS")
if command -v sha256sum >/dev/null 2>&1; then have=$(sha256sum "$tmp/$file"); else have=$(shasum -a 256 "$tmp/$file"); fi
[ -n "$want" ] && [ "$want" = "${have%% *}" ] || fail "the checksum of $file does not match: nothing was installed"

tar -xzf "$tmp/$file" -C "$tmp"
mkdir -p "$bin"
# a new file in its place: an orcux that is running goes on until it notices
cp "$tmp/orcux" "$bin/.orcux.$$"
chmod 755 "$bin/.orcux.$$"
mv -f "$bin/.orcux.$$" "$bin/orcux"
echo "installed $("$bin/orcux" --version) in $bin"
case ":$PATH:" in *":$bin:"*) ;; *) echo "add it to your PATH: export PATH=\"$bin:\$PATH\"" ;; esac
"$bin/orcux" doctor || true
