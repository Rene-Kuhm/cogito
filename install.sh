#!/bin/sh
# Cogito installer.
#
#   curl -fsSL https://raw.githubusercontent.com/Rene-Kuhm/cogito/main/install.sh | sh
#
# Downloads the prebuilt binary for this platform from the latest GitHub
# Release. Falls back to `cargo install` when no prebuilt binary matches.
#
# Environment:
#   COGITO_VERSION       install a specific version (default: latest release)
#   COGITO_INSTALL_DIR   where to put the binary (default: ~/.local/bin)
#   COGITO_FROM_SOURCE   set to 1 to skip the download and build with cargo
#
# This script never uses sudo and never writes outside COGITO_INSTALL_DIR.

set -eu

REPO="Rene-Kuhm/cogito"
BIN="cogito"

VERSION="${COGITO_VERSION:-}"
INSTALL_DIR="${COGITO_INSTALL_DIR:-$HOME/.local/bin}"
FROM_SOURCE="${COGITO_FROM_SOURCE:-0}"

info() { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

need() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

need uname
need mkdir
need tar

# ---------------------------------------------------------------- platform

detect_target() {
  os="$(uname -s)"
  arch="$(uname -m)"

  case "$os" in
    Linux)
      case "$arch" in
        x86_64|amd64) printf 'x86_64-unknown-linux-gnu' ;;
        aarch64|arm64) printf 'aarch64-unknown-linux-gnu' ;;
        *) return 1 ;;
      esac
      ;;
    Darwin)
      case "$arch" in
        x86_64) printf 'x86_64-apple-darwin' ;;
        arm64|aarch64) printf 'aarch64-apple-darwin' ;;
        *) return 1 ;;
      esac
      ;;
    *) return 1 ;;
  esac
}

# ---------------------------------------------------------------- download

latest_version() {
  # Prefer the API; fall back to following the /releases/latest redirect.
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null \
      | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v\{0,1\}\([^"]*\)".*/\1/p' \
      | head -n 1
  elif command -v wget >/dev/null 2>&1; then
    wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" 2>/dev/null \
      | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v\{0,1\}\([^"]*\)".*/\1/p' \
      | head -n 1
  fi
}

fetch() {
  url="$1"
  dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$url" -o "$dest"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$dest" "$url"
  else
    die "need curl or wget to download the release"
  fi
}

verify_checksum() {
  archive="$1"
  checksum_file="$2"
  expected="$(sed -n "s/^\([0-9a-f]\{64\}\)[[:space:]]\+$(basename "$archive")$/\1/p" "$checksum_file")"
  [ -n "$expected" ] || { warn "checksum for $(basename "$archive") not listed; skipping verification"; return 0; }

  if command -v sha256sum >/dev/null 2>&1; then
    actual="$(sha256sum "$archive" | awk '{print $1}')"
  elif command -v shasum >/dev/null 2>&1; then
    actual="$(shasum -a 256 "$archive" | awk '{print $1}')"
  else
    warn "no sha256 tool available; skipping verification"
    return 0
  fi

  [ "$expected" = "$actual" ] || die "checksum mismatch for $(basename "$archive")"
  info "checksum verified"
}

# ---------------------------------------------------------------- install

install_from_source() {
  need cargo
  info "Building from source with cargo (this can take a few minutes)…"
  if [ -n "$VERSION" ]; then
    cargo install --locked --git "https://github.com/${REPO}" --tag "v${VERSION}" cogito-cli
  else
    cargo install --locked --git "https://github.com/${REPO}" cogito-cli
  fi
  info ""
  info "Installed with cargo. The binary is in \$CARGO_HOME/bin (usually ~/.cargo/bin)."
}

install_prebuilt() {
  target="$1"
  [ -n "$VERSION" ] || VERSION="$(latest_version)"
  [ -n "$VERSION" ] || die "could not determine the latest version; set COGITO_VERSION"

  archive_name="${BIN}-${VERSION}-${target}.tar.gz"
  base_url="https://github.com/${REPO}/releases/download/v${VERSION}"

  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT INT TERM

  info "Downloading ${BIN} v${VERSION} for ${target}…"
  fetch "${base_url}/${archive_name}" "${tmp}/${archive_name}" \
    || die "download failed: ${base_url}/${archive_name}"

  if fetch "${base_url}/cogito-checksums.txt" "${tmp}/checksums.txt" 2>/dev/null; then
    verify_checksum "${tmp}/${archive_name}" "${tmp}/checksums.txt"
  fi

  tar -xzf "${tmp}/${archive_name}" -C "$tmp"

  # The tarball contains a single top-level directory.
  extracted="$(find "$tmp" -maxdepth 2 -type f -name "$BIN" | head -n 1)"
  [ -n "$extracted" ] || die "could not find the ${BIN} binary inside the archive"

  mkdir -p "$INSTALL_DIR"
  cp "$extracted" "${INSTALL_DIR}/${BIN}"
  chmod +x "${INSTALL_DIR}/${BIN}"
  info "Installed ${INSTALL_DIR}/${BIN}"
}

main() {
  if [ "$FROM_SOURCE" = "1" ]; then
    install_from_source
  else
    target="$(detect_target || true)"
    if [ -z "$target" ]; then
      warn "No prebuilt binary for $(uname -s)/$(uname -m)."
      install_from_source
    else
      install_prebuilt "$target" || {
        warn "Prebuilt install failed; falling back to a source build."
        install_from_source
      }
    fi
  fi

  # PATH check
  case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
      info ""
      info "${INSTALL_DIR} is not on your PATH. Add it with:"
      info "  export PATH=\"${INSTALL_DIR}:\$PATH\""
      ;;
  esac

  info ""
  info "Next steps:"
  info "  cogito doctor          # check the install and agent integrations"
  info "  cogito setup           # configure the AI agents you use"
}

main
