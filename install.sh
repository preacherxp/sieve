#!/bin/sh
# Installs a prebuilt sieve binary from GitHub releases.
#   curl -fsSL https://raw.githubusercontent.com/preacherxp/sieve/master/install.sh | sh
# SIEVE_VERSION=v0.1.0 pins a release; SIEVE_INSTALL_DIR overrides ~/.local/bin.
set -eu

repo=preacherxp/sieve
dir=${SIEVE_INSTALL_DIR:-$HOME/.local/bin}

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-musl ;;
  Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-musl ;;
  Darwin-arm64) target=aarch64-apple-darwin ;;
  *) echo "No prebuilt sieve for $(uname -s) $(uname -m); use: cargo install --git https://github.com/$repo --locked" >&2; exit 1 ;;
esac

if [ -n "${SIEVE_VERSION:-}" ]; then
  base="https://github.com/$repo/releases/download/$SIEVE_VERSION"
else
  base="https://github.com/$repo/releases/latest/download"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
archive="sieve-$target.tar.gz"
curl -fsSL -o "$tmp/$archive" "$base/$archive"
curl -fsSL -o "$tmp/$archive.sha256" "$base/$archive.sha256"
if command -v sha256sum >/dev/null; then
  (cd "$tmp" && sha256sum -c "$archive.sha256" >/dev/null)
else
  (cd "$tmp" && shasum -a 256 -c "$archive.sha256" >/dev/null)
fi
tar -xzf "$tmp/$archive" -C "$tmp"
mkdir -p "$dir"
install -m 755 "$tmp/sieve" "$dir/sieve"
echo "Installed sieve to $dir/sieve"
case ":$PATH:" in *":$dir:"*) ;; *) echo "Add $dir to PATH." ;; esac
