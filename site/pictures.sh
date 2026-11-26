#!/bin/sh
# The pictures of the site, all made from the mark of src/mark.rs. Run from the root of the repo when the mark changes.
# The PNGs come from a headless Chrome (CHROME=<path> for another one): nothing else to install.
set -e
mark() { cargo run --quiet -- mark "$@"; }
mark svg >site/logo.svg
mark svg light >site/logo-light.svg
mark icon >site/favicon.svg
chrome=${CHROME:-"/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"}
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
png() { # <svg> <size> <png>
  printf '<body style="margin:0"><img src="file://%s/%s" width="%s">' "$PWD" "$1" "$2" >"$tmp/p.html"
  "$chrome" --headless --disable-gpu --hide-scrollbars --allow-file-access-from-files --default-background-color=00000000 \
    --force-device-scale-factor=1 --window-size="$2,$2" --screenshot="$PWD/$3" "file://$tmp/p.html" 2>/dev/null
}
png site/favicon.svg 32 site/favicon-32.png
png site/favicon.svg 180 site/apple-touch-icon.png
