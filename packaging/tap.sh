#!/bin/sh
# Puts the formula of a published release in the tap (github.com/borjamrd/homebrew-tap), so that
# `brew install borjamrd/tap/orcux` gets it: tap.sh 0.2.0. Written from the checksums of the release by formula.sh,
# as the gh account that can write the tap.
set -eu
version=$1 tap=borjamrd/homebrew-tap here=$(dirname "$0")
dir=$(mktemp -d); trap 'rm -rf "$dir"' EXIT
gh release download "v$version" --repo borjamrd/orcux --pattern SHA256SUMS --dir "$dir"
"$here/formula.sh" "$version" "$dir/SHA256SUMS" > "$dir/orcux.rb"
sha=$(gh api "repos/$tap/contents/Formula/orcux.rb" --jq .sha 2>/dev/null || true)
gh api -X PUT "repos/$tap/contents/Formula/orcux.rb" -f message="orcux $version" -f content="$(base64 < "$dir/orcux.rb" | tr -d '\n')" ${sha:+-f sha="$sha"} --jq .commit.html_url
