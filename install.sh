#!/bin/sh
# KubeTitan installer.
#
#   curl -fsSL https://kubetitan.com/install | sh
#
# Downloads the latest release for this platform from GitHub, checks it against
# the release's SHA256SUMS, and puts the binary somewhere on your PATH.
#
# Environment overrides, for the impatient and for testing:
#   KT_VERSION=v0.2.0     install a specific release instead of the latest
#   KT_PREFIX=/opt/kt     install into /opt/kt/bin, no sudo
#   KT_REPO / KT_API / KT_DL   point at a different repository or a mirror
#
# Nothing here reads stdin: the script is meant to be piped straight into `sh`.
set -eu

REPO="${KT_REPO:-dotmethodme/KubeTitan}"
API="${KT_API:-https://api.github.com}"
DL="${KT_DL:-https://github.com}"
VERSION="${KT_VERSION:-latest}"
PREFIX="${KT_PREFIX:-}"

say() { printf '%s\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

need() {
  command -v "$1" >/dev/null 2>&1 || die "$1 is required but not installed"
}

# ---- what are we installing onto -------------------------------------------
os=$(uname -s)
arch=$(uname -m)
case "$os" in
  Linux) plat_os=linux ;;
  Darwin) plat_os=macos ;;
  *) die "unsupported operating system: $os (KubeTitan ships for Linux and macOS)" ;;
esac
case "$arch" in
  x86_64|amd64) plat_arch=x86_64 ;;
  arm64|aarch64) plat_arch=aarch64 ;;
  *) die "unsupported architecture: $arch" ;;
esac
platform="$plat_os-$plat_arch"

case "$platform" in
  linux-x86_64|macos-aarch64) ;;
  *) die "no build published for $platform yet (published: linux-x86_64, macos-aarch64)" ;;
esac

need curl
need tar

# ---- which version ---------------------------------------------------------
if [ "$VERSION" = "latest" ]; then
  # /releases/latest redirects to the newest non-prerelease tag; following it and
  # reading the final URL avoids parsing JSON without jq
  resolved=$(curl -fsSL -o /dev/null -w '%{url_effective}' "$API/repos/$REPO/releases/latest") \
    || die "could not reach GitHub to find the latest release"
  tag=$(printf '%s' "$resolved" | sed -n 's|.*/tag/||p')
  [ -n "$tag" ] || die "could not work out the latest release tag (got: $resolved)"
  VERSION="$tag"
fi
case "$VERSION" in
  v*) ;;
  *) VERSION="v$VERSION" ;;
esac

archive="kubetitan-$platform.tar.gz"
base="$DL/$REPO/releases/download/$VERSION"

say "KubeTitan $VERSION for $platform"

# ---- download --------------------------------------------------------------
tmp=$(mktemp -d 2>/dev/null || mktemp -d -t kubetitan)
trap 'rm -rf "$tmp"' EXIT INT TERM

curl -fsSL --retry 3 -o "$tmp/$archive" "$base/$archive" \
  || die "could not download $base/$archive"
curl -fsSL --retry 3 -o "$tmp/SHA256SUMS" "$base/SHA256SUMS" \
  || die "could not download $base/SHA256SUMS"

# ---- verify ---------------------------------------------------------------
# a checksum that does not match is the difference between installing a release
# and running whatever the network handed over
expected=$(grep " \*\?$archive\$" "$tmp/SHA256SUMS" | awk '{print $1}' | head -1)
[ -n "$expected" ] || die "no checksum for $archive in SHA256SUMS"
if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$tmp/$archive" | awk '{print $1}')
else
  actual=$(shasum -a 256 "$tmp/$archive" | awk '{print $1}')
fi
[ "$expected" = "$actual" ] || die "checksum mismatch for $archive (expected $expected, got $actual)"
say "checksum ok"

tar -C "$tmp" -xzf "$tmp/$archive" || die "could not unpack $archive"
[ -f "$tmp/kubetitan" ] || die "the archive did not contain a kubetitan binary"

# ---- install --------------------------------------------------------------
if [ -n "$PREFIX" ]; then
  bin="$PREFIX/bin"
  mkdir -p "$bin"
  install -m 0755 "$tmp/kubetitan" "$bin/kubetitan"
elif [ -w /usr/local/bin ]; then
  install -m 0755 "$tmp/kubetitan" /usr/local/bin/kubetitan
  bin=/usr/local/bin
elif command -v sudo >/dev/null 2>&1; then
  say "installing to /usr/local/bin (needs sudo)"
  sudo install -m 0755 "$tmp/kubetitan" /usr/local/bin/kubetitan
  bin=/usr/local/bin
else
  bin="$HOME/.local/bin"
  mkdir -p "$bin"
  install -m 0755 "$tmp/kubetitan" "$bin/kubetitan"
fi

# macOS keeps the quarantine bit only on files it saw arrive through a browser;
# curl does not set it, so an ad-hoc signature is all a downloaded binary needs
if [ "$plat_os" = macos ] && command -v codesign >/dev/null 2>&1; then
  codesign --force --sign - "$bin/kubetitan" >/dev/null 2>&1 || true
fi

say "installed $bin/kubetitan"
case ":$PATH:" in
  *":$bin:"*) ;;
  *) say "note: $bin is not on your PATH; add it with: export PATH=\"$bin:\$PATH\"" ;;
esac
say "run 'kubetitan' with a kubeconfig that already works with kubectl"
