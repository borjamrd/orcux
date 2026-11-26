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

# the picture of a shared link, 1280 by 640
cat >"$tmp/og.html" <<EOF
<body style="margin:0;width:1280px;height:640px;background:#0f1114;color:#e8eaed;display:flex;align-items:center;gap:72px;padding:0 110px;box-sizing:border-box;font-family:ui-monospace,Menlo,monospace">
<img src="file://$PWD/site/logo.svg" width="380"><div><div style="font-size:84px;font-weight:700;letter-spacing:-2px">orcux</div>
<div style="font-size:34px;line-height:1.35;color:#9aa1a9;margin-top:18px">Several coding agents at once, each in a worktree of its own.</div></div>
EOF
"$chrome" --headless --disable-gpu --hide-scrollbars --allow-file-access-from-files --force-device-scale-factor=1 \
  --window-size=1280,640 --screenshot="$PWD/site/og.png" "file://$tmp/og.html" 2>/dev/null

# the demo as a video, a tenth of the GIF, and its first frame as the poster
if command -v ffmpeg >/dev/null; then
  ffmpeg -loglevel error -y -i docs/demo.gif -movflags +faststart -pix_fmt yuv420p -vf 'scale=trunc(iw/2)*2:trunc(ih/2)*2' \
    -c:v libx264 -crf 26 -preset slow site/demo.mp4
  ffmpeg -loglevel error -y -i docs/demo.gif -frames:v 1 -q:v 4 site/demo.jpg
else
  echo 'no ffmpeg: site/demo.mp4 and site/demo.jpg left as they are' >&2
fi
