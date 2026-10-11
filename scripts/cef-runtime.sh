#!/usr/bin/env bash

# CEF's Cargo version metadata pins the runtime that the native client links.
cef_runtime_version() {
    local package_id
    package_id="$(cargo pkgid --manifest-path "$1/Cargo.toml" -p cef-dll-sys)" || return
    printf '%s\n' "${package_id##*+}"
}

cef_framework_matches() {
    local version
    version="$(/usr/bin/plutil -extract CFBundleShortVersionString raw -o - \
        "$1/Resources/Info.plist" 2>/dev/null)" || return 1
    [[ "$version" == "$2.0" ]]
}

find_cef_framework() {
    local version="$1" arch="$2" target_dir="$3" cache="$4" explicit="$5"
    local candidate
    if [[ -n "$explicit" ]]; then
        candidate="$explicit"
        if [[ -d "$candidate/Chromium Embedded Framework.framework" ]]; then
            candidate="$candidate/Chromium Embedded Framework.framework"
        fi
        if cef_framework_matches "$candidate" "$version"; then
            printf '%s\n' "$candidate"
            return
        fi
        echo "CEF override must contain runtime $version: $explicit" >&2
        return 1
    fi
    for candidate in \
        "$cache" \
        "$cache/Chromium Embedded Framework.framework" \
        "$cache"/*/cef_macos_"$arch"/Chromium\ Embedded\ Framework.framework \
        "$target_dir"/build/cef-dll-sys-*/out/cef_macos_"$arch"/Chromium\ Embedded\ Framework.framework; do
        if cef_framework_matches "$candidate" "$version"; then
            printf '%s\n' "$candidate"
            return
        fi
    done
    echo "CEF runtime $version for $arch was not found; build the native client first" >&2
    return 1
}
