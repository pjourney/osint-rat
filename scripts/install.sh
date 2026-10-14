#!/usr/bin/env bash
# One-liner installer for tagged os-int releases (Linux and macOS).
# Pin the script URL and OS_INT_VERSION to the same tag, not main:
#   OS_INT_VERSION=v1.0.0 curl -fsSL https://raw.githubusercontent.com/pjourney/osint-rat/v1.0.0/scripts/install.sh | bash
set -euo pipefail

REPO="${OS_INT_REPO:-pjourney/osint-rat}"
INSTALL_DIR="${OS_INT_INSTALL_DIR:-${HOME}/.local/bin}"
VERSION="${OS_INT_VERSION:-latest}"

say() {
  printf 'os-int-install: %s\n' "$*" >&2
}

die() {
  say "$*"
  exit 1
}

target_triple() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "${os}:${arch}" in
    Linux:x86_64) echo "x86_64-unknown-linux-gnu" ;;
    Linux:amd64) echo "x86_64-unknown-linux-gnu" ;;
    Darwin:x86_64) echo "x86_64-apple-darwin" ;;
    Darwin:arm64) echo "aarch64-apple-darwin" ;;
    Linux:aarch64 | Linux:arm64)
      die "no prebuilt Linux aarch64 archive yet; install from source (docs/install.md)"
      ;;
    *) die "unsupported platform ${os}/${arch}. See docs/install.md (Windows is a zip from GitHub Releases)." ;;
  esac
}

checksum_ok() {
  local sums="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum --strict -c "$sums"
    return
  fi
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 -c "$sums"
    return
  fi
  die "need sha256sum or shasum to verify checksums"
}

fetch() {
  local url="$1"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL -A "os-int-install" "$url"
    return
  fi
  if command -v wget >/dev/null 2>&1; then
    wget -qO- --user-agent="os-int-install" "$url"
    return
  fi
  die "need curl or wget"
}

download() {
  local url="$1"
  local dest="$2"
  fetch "$url" >"$dest"
}

release_tag() {
  if [ "$VERSION" != "latest" ]; then
    echo "$VERSION"
    return
  fi
  local tag
  tag="$(fetch "https://api.github.com/repos/${REPO}/releases/latest" | sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)"
  [ -n "$tag" ] || die "no published GitHub release found (/releases/latest ignores drafts). Install from source: cargo install --git https://github.com/${REPO}"
  echo "$tag"
}

main() {
  local triple tag base archive sums work
  triple="$(target_triple)"
  tag="$(release_tag)"
  base="os-int-${triple}"
  archive="${base}.tar.gz"
  sums="${archive}.sha256"
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  say "installing ${tag} (${triple}) into ${INSTALL_DIR}"
  download "https://github.com/${REPO}/releases/download/${tag}/${archive}" "${work}/${archive}"
  download "https://github.com/${REPO}/releases/download/${tag}/${sums}" "${work}/${sums}"
  (
    cd "$work"
    checksum_ok "$sums"
    tar -xzf "$archive"
  )
  mkdir -p "$INSTALL_DIR"
  if [ -x "${work}/${base}/os-int" ]; then
    cp "${work}/${base}/os-int" "${INSTALL_DIR}/os-int"
  elif [ -x "${work}/os-int" ]; then
    cp "${work}/os-int" "${INSTALL_DIR}/os-int"
  else
    die "archive did not contain an os-int binary"
  fi
  chmod +x "${INSTALL_DIR}/os-int"
  say "installed ${INSTALL_DIR}/os-int"
  if ! command -v os-int >/dev/null 2>&1; then
    say "add ${INSTALL_DIR} to PATH, then run: os-int --version && os-int doctor"
  else
    "${INSTALL_DIR}/os-int" --version || true
  fi
}

main "$@"
