#!/bin/sh
# The Homebrew formula of a version, out of its checksums: formula.sh 0.1.0 SHA256SUMS
set -eu
version=$1 sums=$2
sum() { awk -v f="orcux-$1.tar.gz" '$2 == f { print $1; found = 1 } END { exit !found }' "$sums"; }
url() { echo "https://github.com/borjamrd/orcux/releases/download/v$version/orcux-$1.tar.gz"; }
# before anything is printed: a missing checksum must stop it
for t in aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-musl x86_64-unknown-linux-musl; do
  sum "$t" >/dev/null || { echo "no checksum for $t in $sums" >&2; exit 1; }
done
asset() { printf '      url "%s"\n      sha256 "%s"\n' "$(url "$1")" "$(sum "$1")"; }
cat <<EOF
class Orcux < Formula
  desc "Git worktrees, ports and terminals for several coding agents at once"
  homepage "https://github.com/borjamrd/orcux"
  version "$version"
  license "MIT"

  depends_on "tmux"

  on_macos do
    on_arm do
$(asset aarch64-apple-darwin)
    end
    on_intel do
$(asset x86_64-apple-darwin)
    end
  end

  on_linux do
    on_arm do
$(asset aarch64-unknown-linux-musl)
    end
    on_intel do
$(asset x86_64-unknown-linux-musl)
    end
  end

  def install
    bin.install "orcux"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/orcux --version")
  end
end
EOF
