#!/bin/sh
# Install the Harbor release binary for Linux or macOS.
#
# This script is intentionally boring: it resolves a release tag, downloads
# the archive and its published SHA-256 file, verifies the digest, and only
# then extracts and atomically installs one regular executable. It never uses
# sudo, never edits shell startup files, and never executes anything from the
# archive.
#
# Usage:
#   sh install.sh [vX.Y.Z]
#   HARBOR_BIN_DIR="$HOME/bin" sh install.sh
#
# The release workflow uploads this file as `install.sh` on every release. For
# the latest release:
#   curl -fsSLO https://github.com/summertools/harbor/releases/latest/download/install.sh
#   less install.sh
#   sh install.sh

set -eu

REPO="summertools/harbor"
BASE_URL="https://github.com/$REPO"
LATEST_URL="$BASE_URL/releases/latest"

tmp=""
tmp_target=""

usage() {
    cat <<'EOF'
Install Harbor from a GitHub release.

Usage:
  install.sh [vX.Y.Z]

Environment:
  HARBOR_VERSION   Release tag to install (same as the positional argument).
  HARBOR_BIN_DIR   Destination directory (default: ~/.local/bin).

The script verifies the release SHA-256 before installing and never uses sudo.
EOF
}

die() {
    printf 'harbor-install: %s\n' "$*" >&2
    exit 1
}

log() {
    printf '%s\n' "$*"
}

cleanup() {
    if [ -n "$tmp_target" ]; then
        rm -f "$tmp_target"
    fi
    if [ -n "$tmp" ]; then
        rm -rf "$tmp"
    fi
    tmp_target=""
    tmp=""
}
trap cleanup EXIT
trap 'cleanup; exit 1' HUP INT TERM

tag="${HARBOR_VERSION:-}"
if [ "$#" -gt 0 ]; then
    case "$1" in
        -h|--help)
            usage
            exit 0
            ;;
        v[0-9]*)
            tag="$1"
            shift
            ;;
        *)
            usage >&2
            exit 2
            ;;
    esac
fi
[ "$#" -eq 0 ] || {
    usage >&2
    exit 2
}

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v tar >/dev/null 2>&1 || die "tar is required"
command -v mktemp >/dev/null 2>&1 || die "mktemp is required"
command -v uname >/dev/null 2>&1 || die "uname is required"
command -v grep >/dev/null 2>&1 || die "grep is required"

os=$(uname -s 2>/dev/null || true)
arch=$(uname -m 2>/dev/null || true)
case "$os/$arch" in
    Linux/x86_64|Linux/amd64)
        target="x86_64-unknown-linux-musl"
        ;;
    Linux/aarch64|Linux/arm64)
        target="aarch64-unknown-linux-musl"
        ;;
    Darwin/x86_64)
        target="x86_64-apple-darwin"
        ;;
    Darwin/arm64|Darwin/aarch64)
        target="aarch64-apple-darwin"
        ;;
    *)
        die "unsupported platform $os/$arch; download a release archive manually or use 'cargo install --git https://github.com/$REPO'"
        ;;
esac

if [ -z "$tag" ]; then
    log "Resolving the latest Harbor release…"
    latest=$(curl -fsSIL -o /dev/null -w '%{url_effective}' "$LATEST_URL") || die "could not resolve the latest release"
    tag=${latest##*/}
fi

if ! printf '%s' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$'; then
    die "invalid release tag: $tag"
fi

archive="harbor-$tag-$target.tar.gz"
checksum="harbor-$tag-$target.sha256"
download_url="$BASE_URL/releases/download/$tag/$archive"
checksum_url="$BASE_URL/releases/download/$tag/$checksum"

umask 077
tmp=$(mktemp -d "${TMPDIR:-/tmp}/harbor-install.XXXXXX") || die "could not create a temporary directory"

log "Downloading $archive"
curl -fsSL --proto '=https' --proto-redir '=https' --tlsv1.2 --retry 3 --retry-delay 1 \
    -o "$tmp/$archive" "$download_url" || die "download failed"
curl -fsSL --proto '=https' --proto-redir '=https' --tlsv1.2 --retry 3 --retry-delay 1 \
    -o "$tmp/$checksum" "$checksum_url" || die "checksum download failed"

if command -v sha256sum >/dev/null 2>&1; then
    (cd "$tmp" && sha256sum -c "$checksum" >/dev/null 2>&1) || die "SHA-256 verification failed"
elif command -v shasum >/dev/null 2>&1; then
    (cd "$tmp" && shasum -a 256 -c "$checksum" >/dev/null 2>&1) || die "SHA-256 verification failed"
else
    die "no SHA-256 verifier found (install sha256sum or shasum)"
fi
log "SHA-256 verified"

tar -tzf "$tmp/$archive" > "$tmp/entries" || die "could not inspect the archive"
found=0
while IFS= read -r entry; do
    case "$entry" in
        ""|harbor|./harbor)
            found=1
            ;;
        /*|*..*)
            die "archive contains an unsafe path: $entry"
            ;;
        *)
            ;;
    esac
done < "$tmp/entries"
[ "$found" -eq 1 ] || die "archive does not contain harbor"

mkdir "$tmp/unpack" || die "could not prepare the extraction directory"
# Extract only the executable we need. This also prevents any unrelated
# symlink or hardlink entries from being materialised.
tar -xzf "$tmp/$archive" -C "$tmp/unpack" harbor || die "could not extract the archive"
source_bin="$tmp/unpack/harbor"
[ -f "$source_bin" ] && [ ! -L "$source_bin" ] || die "archive did not contain a regular harbor executable"

if [ -n "${HARBOR_BIN_DIR:-}" ]; then
    bin_dir="$HARBOR_BIN_DIR"
elif [ -n "${HOME:-}" ]; then
    bin_dir="${XDG_BIN_HOME:-$HOME/.local/bin}"
else
    die "HOME is not set; set HARBOR_BIN_DIR to choose an install directory"
fi
case "$bin_dir" in
    "~/"*)
        [ -n "${HOME:-}" ] || die "cannot expand ~ without HOME"
        bin_dir="$HOME/${bin_dir#~/}"
        ;;
esac
[ -n "$bin_dir" ] || die "install directory is empty"
[ "$bin_dir" = "/" ] && die "refusing to install directly into /"
bin_dir=${bin_dir%/}
[ -n "$bin_dir" ] || die "install directory is empty"

mkdir -p "$bin_dir" || die "could not create $bin_dir"
[ -d "$bin_dir" ] || die "$bin_dir is not a directory"
[ -w "$bin_dir" ] || die "$bin_dir is not writable; choose a user-owned directory with HARBOR_BIN_DIR"

target="$bin_dir/harbor"
[ ! -d "$target" ] || die "$target is a directory"

tmp_target=$(mktemp "$bin_dir/.harbor.XXXXXX") || die "could not create a temporary install file"
cp "$source_bin" "$tmp_target" || die "could not stage the new executable"
chmod 0755 "$tmp_target" || die "could not set executable permissions"
mv -f "$tmp_target" "$target" || die "could not replace $target"
tmp_target=""

case ":${PATH:-}:" in
    *":$bin_dir:"*)
        ;;
    *)
        printf 'warning: %s is not on PATH\n' "$bin_dir" >&2
        printf '  add it with: export PATH="%s:$PATH"\n' "$bin_dir" >&2
        ;;
esac

version=$("$target" --version 2>/dev/null) || die "installed executable failed to run"
log "Installed $version at $target"
