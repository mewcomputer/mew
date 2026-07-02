#!/usr/bin/env bash
set -euo pipefail

# One-line installer for mew.
# Usage:
#   curl --proto '=https' --tlsv1.2 -sSf https://mew.computer/get.sh | sh
#
# Flags:
#   --nightly        install the latest nightly build instead of the latest release
#
# Environment variables:
#   MEW_CHANNEL      - "stable" or "nightly" (default: "stable")
#   MEW_VERSION      - install a specific stable version (e.g. "v0.2.0") or "latest"
#   MEW_INSTALL_DIR  - override install directory
#   MEW_DRY_RUN      - set to "1" to print the plan without installing
#   MEW_VERBOSE      - set to "1" for extra logging

repo="mewcomputer/mew"
api_base="https://api.github.com/repos/${repo}"
install_dir_override="${MEW_INSTALL_DIR:-}"
dry_run="${MEW_DRY_RUN:-0}"
verbose="${MEW_VERBOSE:-0}"
channel="${MEW_CHANNEL:-stable}"
requested_version="${MEW_VERSION:-latest}"

# Parse command-line flags. Everything else is ignored so the script can be
# piped from curl without surprising behavior.
for arg in "$@"; do
    case "$arg" in
        --nightly)
            channel="nightly"
            ;;
        --stable)
            channel="stable"
            ;;
        --help|-h)
            sed -n '2,20p' "$0"
            exit 0
            ;;
    esac
done

log() {
    printf '%s\n' "$*"
}

verbose_log() {
    if [[ "$verbose" == "1" ]]; then
        printf '%s\n' "$*"
    fi
}

fail() {
    printf 'ERROR: %s\n' "$*" >&2
    exit 1
}

require_tool() {
    local tool="$1"
    if ! command -v "$tool" >/dev/null 2>&1; then
        fail "missing required tool '$tool'"
    fi
}

normalize_home_path() {
    local path="$1"
    case "$path" in
        \~) printf '%s\n' "$HOME" ;;
        \~/*) printf '%s/%s\n' "$HOME" "${path#\~/}" ;;
        \$HOME) printf '%s\n' "$HOME" ;;
        \$HOME/*) printf '%s/%s\n' "$HOME" "${path#\$HOME/}" ;;
        *) printf '%s\n' "$path" ;;
    esac
}

path_contains_dir() {
    local wanted="$1"
    local entry
    local normalized
    IFS=':' read -r -a path_entries <<< "${PATH:-}"
    for entry in "${path_entries[@]}"; do
        normalized="$(normalize_home_path "$entry")"
        if [[ "$normalized" == "$wanted" ]]; then
            return 0
        fi
    done
    return 1
}

choose_install_dir() {
    local local_bin="$HOME/.local/bin"
    local home_bin="$HOME/bin"

    if [[ -n "$install_dir_override" ]]; then
        normalize_home_path "$install_dir_override"
        return
    fi

    IFS=':' read -r -a path_entries <<< "${PATH:-}"
    for entry in "${path_entries[@]}"; do
        normalized="$(normalize_home_path "$entry")"
        case "$normalized" in
            "$local_bin"|"$home_bin")
                printf '%s\n' "$normalized"
                return
                ;;
        esac
    done

    printf '%s\n' "$local_bin"
}

resolve_stable_version() {
    if [[ "$requested_version" != "latest" ]]; then
        printf '%s\n' "$requested_version"
        return
    fi

    local url="${api_base}/releases/latest"
    verbose_log "Fetching latest stable release from ${url}"
    local json
    json="$(curl --fail --show-error --silent --proto '=https' --tlsv1.2 \
        --header 'Accept: application/vnd.github+json' \
        --header 'X-GitHub-Api-Version: 2022-11-28' \
        "$url")"

    local tag
    tag="$(printf '%s' "$json" | grep -o '"tag_name": "v[^"]*"' | head -n1 | cut -d'"' -f4)"
    if [[ -z "$tag" ]]; then
        fail "could not determine latest stable release from GitHub API"
    fi
    printf '%s\n' "$tag"
}

platform_info() {
    local os
    local arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os:$arch" in
        Linux:x86_64|Linux:amd64)
            printf '%s\n' "x86_64-unknown-linux-gnu"
            ;;
        Linux:aarch64|Linux:arm64)
            printf '%s\n' "aarch64-unknown-linux-gnu"
            ;;
        Darwin:x86_64|Darwin:amd64)
            printf '%s\n' "x86_64-apple-darwin"
            ;;
        Darwin:arm64|Darwin:aarch64)
            printf '%s\n' "aarch64-apple-darwin"
            ;;
        *)
            fail "unsupported platform: $os $arch"
            ;;
    esac
}

sha256_file() {
    local file="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$file" | awk '{print $1}'
    else
        fail "missing sha256 tool; install sha256sum or shasum"
    fi
}

cleanup() {
    if [[ -n "${tmp_dir:-}" ]]; then
        rm -rf "$tmp_dir"
    fi
}
trap cleanup EXIT

require_tool awk
require_tool curl
require_tool grep
require_tool mkdir
require_tool mv
require_tool tar
require_tool uname

tmp_dir="$(mktemp -d 2>/dev/null || mktemp -d -t mew-install)"
target="$(platform_info)"
install_dir="$(choose_install_dir)"
target_path="$install_dir/mew"
path_needs_update=0
if ! path_contains_dir "$install_dir"; then
    path_needs_update=1
fi

if [[ "$channel" == "nightly" ]]; then
    if [[ "$requested_version" != "latest" ]]; then
        fail "MEW_VERSION cannot be combined with nightly channel"
    fi
    version="nightly"
    base_url="https://github.com/${repo}/releases/download/nightly"
    tarball_name="mew-nightly-${target}.tar.gz"
else
    version="$(resolve_stable_version)"
    base_url="https://github.com/${repo}/releases/download/${version}"
    tarball_name="mew-${version}-${target}.tar.gz"
fi

checksums_url="$base_url/SHA256SUMS"
tarball_url="$base_url/$tarball_name"

if [[ "$dry_run" == "1" ]]; then
    log "mew installer dry run"
    log "  channel: $channel"
    log "  version: $version"
    log "  platform: $target"
    log "  tarball: $tarball_url"
    log "  checksums: $checksums_url"
    log "  install dir: $install_dir"
    log "No files changed."
    exit 0
fi

archive_path="$tmp_dir/$tarball_name"
checksums_path="$tmp_dir/SHA256SUMS"
extract_dir="$tmp_dir/extract"
mkdir -p "$extract_dir"

log "Installing mew $version for $target"
verbose_log "Downloading $tarball_url"
curl --fail --show-error --silent --proto '=https' --tlsv1.2 --retry 3 --retry-delay 1 \
    --output "$archive_path" "$tarball_url"

verbose_log "Downloading $checksums_url"
curl --fail --show-error --silent --proto '=https' --tlsv1.2 --retry 3 --retry-delay 1 \
    --output "$checksums_path" "$checksums_url"

expected_sha="$(awk -v rel_path="$tarball_name" '$2 == rel_path { print $1 }' "$checksums_path")"
if [[ -z "$expected_sha" ]]; then
    fail "missing checksum for $tarball_name in SHA256SUMS"
fi
actual_sha="$(sha256_file "$archive_path")"
if [[ "$actual_sha" != "$expected_sha" ]]; then
    fail "checksum mismatch for $tarball_name"
fi
verbose_log "Checksum verified: $actual_sha"

tar -xzf "$archive_path" -C "$extract_dir"
binary_path="$(find "$extract_dir" -type f -name mew | head -n1)"
if [[ -z "$binary_path" ]]; then
    fail "archive did not contain a mew binary"
fi
chmod 0755 "$binary_path"

mkdir -p "$install_dir"
if [[ ! -w "$install_dir" ]]; then
    fail "install directory is not writable: $install_dir"
fi

tmp_target="$install_dir/.mew.tmp.$$"
cp "$binary_path" "$tmp_target"
chmod 0755 "$tmp_target"
mv "$tmp_target" "$target_path"

log "mew $version installed to $target_path"

if [[ "$path_needs_update" == "1" ]]; then
    log ""
    log "Add mew to your PATH:"
    if [[ "$install_dir" == "$HOME/.local/bin" ]]; then
        log '  export PATH="$HOME/.local/bin:$PATH"'
    elif [[ "$install_dir" == "$HOME/bin" ]]; then
        log '  export PATH="$HOME/bin:$PATH"'
    else
        log "  export PATH=\"$install_dir:\$PATH\""
    fi
    log ""
    log "Add that line to your shell profile, then restart your shell."
else
    log "Run: mew --version"
fi
