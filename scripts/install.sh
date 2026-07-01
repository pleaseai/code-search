#!/usr/bin/env bash
#
# Auth-free installer for the `csp` standalone binary.
#
# Why this exists:
#   Homebrew (`brew install pleaseai/tap/csp`) and npm (`bunx @pleaseai/csp`)
#   are the primary install paths, but both assume a package manager is present.
#   This script downloads the precompiled, self-contained Rust binary straight
#   from GitHub's release-download CDN — no Homebrew, no Node/Bun, no token.
#
#   Tag resolution uses the public `releases/latest` web redirect instead of
#   api.github.com, so it is not subject to the unauthenticated API rate limit
#   (60 req/hour per IP) and works at Docker build time with no credentials.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/pleaseai/code-search/main/scripts/install.sh | bash
#   curl -fsSL .../install.sh | bash -s -- --version v0.1.7        # pin a version
#   curl -fsSL .../install.sh | CSP_INSTALL_DIR=/usr/local/bin bash # choose a dir
#
# Environment overrides:
#   CSP_VERSION       Pin a release tag (same as --version).
#   CSP_INSTALL_DIR   Directory to install `csp` into (default: ~/.local/bin).

set -euo pipefail

# --- Constants ---------------------------------------------------------------
readonly OWNER="pleaseai"
readonly REPO="code-search"
readonly BIN_NAME="csp"
readonly RELEASES_URL="https://github.com/${OWNER}/${REPO}/releases"

# --- Output helpers ----------------------------------------------------------
# All human-facing messages (info/ok/err) go to stderr, keeping stdout clean for
# piping. Gate colors on the same fd they're written to (stderr): otherwise, with
# stdout redirected to a log file, escape sequences would land in the log instead
# of on screen.
if [[ -t 2 ]]; then
  RED='\033[0;31m'
  GREEN='\033[0;32m'
  YELLOW='\033[1;33m'
  NC='\033[0m'
else
  RED=''
  GREEN=''
  YELLOW=''
  NC=''
fi

# Temp dir cleaned up by the EXIT trap; declared at file scope so the trap can
# reference it after main()'s locals go out of scope.
WORKDIR=""

info() { printf '%b\n' "${YELLOW}==>${NC} $*" >&2; }
ok() { printf '%b\n' "${GREEN}✓${NC} $*" >&2; }
err() { printf '%b\n' "${RED}✗${NC} $*" >&2; }

die() {
  err "$*"
  exit 1
}

# --- musl detection ----------------------------------------------------------
# Alpine and other musl distros need the musl-linked binary; the glibc build
# segfaults at startup on them. Only x86_64 ships a musl asset upstream.
is_musl() {
  # `ldd --version` prints "musl libc" on musl systems (to stderr on Alpine).
  if ldd --version 2>&1 | grep -qi musl; then
    return 0
  fi
  # Fallback: the musl dynamic loader is present as /lib/ld-musl-*.
  if compgen -G '/lib/ld-musl-*' >/dev/null 2>&1; then
    return 0
  fi
  return 1
}

# --- Platform detection ------------------------------------------------------
# Maps `uname` output to the release asset name (e.g. `csp-darwin-arm64`).
detect_asset() {
  local os arch
  case "$(uname -s)" in
    Linux) os="linux" ;;
    Darwin) os="darwin" ;;
    MINGW* | MSYS* | CYGWIN*)
      die "Windows is not supported by this installer. Use 'brew install pleaseai/tap/csp', 'bunx @pleaseai/csp', or download csp-windows-x64.exe from ${RELEASES_URL}/latest."
      ;;
    *) die "Unsupported OS: $(uname -s)." ;;
  esac

  case "$(uname -m)" in
    x86_64 | amd64) arch="x64" ;;
    arm64 | aarch64) arch="arm64" ;;
    *) die "Unsupported architecture: $(uname -m)." ;;
  esac

  # musl only applies to Linux, and only x64 has a musl build upstream.
  if [[ "$os" == "linux" ]] && is_musl; then
    if [[ "$arch" != "x64" ]]; then
      die "No musl (Alpine) binary is published for ${arch}. Install via npm (needs Node 22+/Bun) or build from source."
    fi
    printf '%s-%s-%s-musl' "$BIN_NAME" "$os" "$arch"
    return
  fi

  printf '%s-%s-%s' "$BIN_NAME" "$os" "$arch"
}

# --- Tag resolution ----------------------------------------------------------
# Resolves the latest tag from the `releases/latest` redirect (no API call).
resolve_latest_tag() {
  local redirect
  redirect="$(curl -fsS -o /dev/null -w '%{redirect_url}' "${RELEASES_URL}/latest" || true)"
  [[ -n "$redirect" ]] || die "Could not resolve the latest release tag from ${RELEASES_URL}/latest"
  printf '%s' "${redirect##*/tag/}"
}

# --- Checksum verification ---------------------------------------------------
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    die "Neither sha256sum nor shasum is available for checksum verification."
  fi
}

verify_checksum() {
  local binary="$1" checksum_file="$2"
  local expected actual
  # Upstream ships a per-asset `<asset>.sha256` file whose content is
  # "<hash>  <asset>". Extract just the hash rather than running `shasum -c`,
  # which would look for the original asset filename in the cwd.
  expected="$(awk '{print $1}' "$checksum_file")"
  # Fail closed: an empty/absent hash must abort, never install unverified.
  [[ -n "$expected" ]] || die "Empty checksum file — cannot verify binary integrity."
  actual="$(sha256_of "$binary")"
  [[ "$expected" == "$actual" ]] || die "Checksum mismatch (expected ${expected}, got ${actual})."
  ok "Checksum verified."
}

# --- PATH advice -------------------------------------------------------------
warn_if_not_on_path() {
  local dir="$1"
  case ":${PATH}:" in
    *":${dir}:"*) ;;
    *)
      info "${dir} is not on your PATH. Add it to your shell profile:"
      printf "    export PATH=\"%s:\$PATH\"\n" "$dir" >&2
      ;;
  esac
}

# --- Main --------------------------------------------------------------------
main() {
  local version="${CSP_VERSION:-}"

  while [[ $# -gt 0 ]]; do
    case "$1" in
      --version)
        [[ $# -ge 2 ]] || die "--version requires a tag argument."
        version="$2"
        shift 2
        ;;
      -h | --help)
        printf 'Usage: install.sh [--version <tag>]\n'
        printf '  --version <tag>   Install a specific release (default: latest).\n'
        printf '  CSP_INSTALL_DIR   Target directory (default: ~/.local/bin).\n'
        exit 0
        ;;
      *) die "Unknown argument: $1" ;;
    esac
  done

  command -v curl >/dev/null 2>&1 || die "curl is required but not installed."

  local asset tag base binary checksum install_dir
  asset="$(detect_asset)"

  if [[ -n "$version" ]]; then
    tag="$version"
  else
    info "Resolving latest release..."
    tag="$(resolve_latest_tag)"
  fi

  base="${RELEASES_URL}/download/${tag}"
  info "Installing ${BIN_NAME} ${tag} (${asset})..."

  WORKDIR="$(mktemp -d)"
  trap 'rm -rf "${WORKDIR:-}"' EXIT

  binary="${WORKDIR}/${BIN_NAME}"
  checksum="${WORKDIR}/${asset}.sha256"

  curl -fsSL -o "$binary" "${base}/${asset}" \
    || die "Failed to download ${base}/${asset} (does release ${tag} have a ${asset} binary?)."
  curl -fsSL -o "$checksum" "${base}/${asset}.sha256" \
    || die "Failed to download checksum from ${base}/${asset}.sha256"

  verify_checksum "$binary" "$checksum"

  install_dir="${CSP_INSTALL_DIR:-$HOME/.local/bin}"
  mkdir -p "$install_dir"
  install -m 0755 "$binary" "${install_dir}/${BIN_NAME}"

  ok "Installed ${BIN_NAME} to ${install_dir}/${BIN_NAME}"
  warn_if_not_on_path "$install_dir"
  info "Run: ${BIN_NAME} --version"
}

main "$@"
