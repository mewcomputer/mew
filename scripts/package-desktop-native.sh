#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
profile="${MEW_DESKTOP_PROFILE:-release}"

if [[ -n "${MEW_VERSION:-}" ]]; then
    version="${MEW_VERSION#v}"
else
    package_id="$(cargo pkgid --manifest-path "$repo_root/Cargo.toml" -p mew-desktop)"
    version="${package_id##*#}"
fi

case "$version" in
    ''|*[!0-9.]*|.*|*.)
        echo "invalid desktop version: $version" >&2
        exit 1
        ;;
esac

case "$profile" in
    debug|release) ;;
    *)
        echo "unsupported desktop profile: $profile" >&2
        exit 1
        ;;
esac

case "$(uname -s)" in
    Darwin)
        browser_cli="$(command -v agent-browser || true)"
        if [[ -z "$browser_cli" ]]; then
            echo "agent-browser 0.34.0 or newer is required for native browser tab ownership" >&2
            exit 1
        fi
        while [[ -L "$browser_cli" ]]; do
            browser_link="$(readlink "$browser_cli")"
            if [[ "$browser_link" == /* ]]; then
                browser_cli="$browser_link"
            else
                browser_cli="$(dirname "$browser_cli")/$browser_link"
            fi
        done
        if [[ "$(file -b "$browser_cli")" != *Mach-O* ]]; then
            case "$(uname -m)" in
                arm64) browser_arch="arm64" ;;
                x86_64) browser_arch="x64" ;;
                *) echo "unsupported native browser architecture" >&2; exit 1 ;;
            esac
            browser_cli="$(dirname "$browser_cli")/agent-browser-darwin-$browser_arch"
            if [[ ! -x "$browser_cli" || "$(file -b "$browser_cli")" != *Mach-O* ]]; then
                echo "agent-browser's standalone macOS binary is required for app packaging" >&2
                exit 1
            fi
        fi
        browser_version="$("$browser_cli" --version)"
        if [[ "$browser_version" =~ ([0-9]+)\.([0-9]+)\.([0-9]+) ]]; then
            browser_major="${BASH_REMATCH[1]}"
            browser_minor="${BASH_REMATCH[2]}"
        else
            echo "could not determine agent-browser version: $browser_version" >&2
            exit 1
        fi
        if (( browser_major == 0 && browser_minor < 34 )); then
            echo "native browser requires agent-browser 0.34.0 or newer; found $browser_version" >&2
            exit 1
        fi
        target_dir="$repo_root/target/$profile"
        app="$target_dir/bundle/macos/mew.app"
        contents="$app/Contents"
        frameworks="$contents/Frameworks"
        mkdir -p "$contents/MacOS" "$contents/Resources" "$frameworks"
        for binary in "$target_dir/mew-desktop" "$target_dir/mew"; do
            if [[ ! -x "$binary" ]]; then
                echo "desktop binary is missing or not executable: $binary" >&2
                exit 1
            fi
        done
        rm -f "$contents/MacOS/mew-desktop" "$contents/MacOS/mew" \
            "$contents/MacOS/agent-browser" "$contents/MacOS/mew-cef-host-helper"
        cp "$target_dir/mew-desktop" "$contents/MacOS/mew-desktop"
        cp "$target_dir/mew" "$contents/MacOS/mew"
        cp "$browser_cli" "$contents/MacOS/agent-browser"
        cef_framework="${MEW_CEF_FRAMEWORK_SOURCE:-}"
        if [[ -n "$cef_framework" && -d "$cef_framework/Chromium Embedded Framework.framework" ]]; then
            cef_framework="$cef_framework/Chromium Embedded Framework.framework"
        fi
        if [[ -z "$cef_framework" ]]; then
            cef_path="${CEF_PATH:-$HOME/.local/share/cef}"
            if [[ -d "$cef_path/Chromium Embedded Framework.framework" ]]; then
                cef_framework="$cef_path/Chromium Embedded Framework.framework"
            elif [[ -d "$cef_path" && "$(basename "$cef_path")" == "Chromium Embedded Framework.framework" ]]; then
                cef_framework="$cef_path"
            else
                case "$(uname -m)" in
                    arm64) cef_arch="aarch64" ;;
                    x86_64) cef_arch="x86_64" ;;
                    *) echo "unsupported macOS architecture for the native browser runtime" >&2; exit 1 ;;
                esac
                for candidate in "$cef_path"/*/cef_macos_"$cef_arch"/Chromium\ Embedded\ Framework.framework; do
                    if [[ -d "$candidate" ]]; then
                        cef_framework="$candidate"
                        break
                    fi
                done
            fi
        fi
        if [[ -z "$cef_framework" ]]; then
            for candidate in "$target_dir"/build/cef-dll-sys-*/out/cef_macos_"$cef_arch"/Chromium\ Embedded\ Framework.framework; do
                if [[ -d "$candidate" ]]; then
                    cef_framework="$candidate"
                    break
                fi
            done
        fi
        cef_helper="${MEW_CEF_HELPER_PATH:-$target_dir/mew-cef-host-helper}"
        if [[ ! -d "$cef_framework" || ! -x "$cef_helper" ]]; then
            echo "native browser runtime assets are missing" >&2
            exit 1
        fi
        rm -rf "$frameworks/Chromium Embedded Framework.framework"
        ditto "$cef_framework" "$frameworks/Chromium Embedded Framework.framework"
        cp "$cef_helper" "$contents/MacOS/mew-cef-host-helper"
        # Windowed CEF on macOS selects process-specific helper bundles for
        # renderer and GPU processes. A single adjacent helper executable is
        # enough for development on some platforms, but it leaves the native
        # browser with a page target and no renderer on macOS.
        helper_names=(
            "mew-desktop Helper"
            "mew-desktop Helper (Alerts)"
            "mew-desktop Helper (GPU)"
            "mew-desktop Helper (Plugin)"
            "mew-desktop Helper (Renderer)"
        )
        for helper_name in "${helper_names[@]}"; do
            helper_bundle="$frameworks/$helper_name.app"
            rm -rf "$helper_bundle"
            mkdir -p "$helper_bundle/Contents/MacOS" \
                "$helper_bundle/Contents/Resources" \
                "$helper_bundle/Contents/Frameworks"
            cp "$cef_helper" "$helper_bundle/Contents/MacOS/$helper_name"
            cp "$repo_root/apps/mew-desktop/Info.plist" "$helper_bundle/Contents/Info.plist"
            /usr/bin/plutil -replace CFBundleExecutable -string "$helper_name" \
                "$helper_bundle/Contents/Info.plist"
            /usr/bin/plutil -replace CFBundleShortVersionString -string "$version" \
                "$helper_bundle/Contents/Info.plist"
            /usr/bin/plutil -replace CFBundleVersion -string "$version" \
                "$helper_bundle/Contents/Info.plist"
        done
        cp "$repo_root/apps/mew-desktop/Info.plist" "$contents/Info.plist"
        /usr/bin/plutil -replace CFBundleShortVersionString -string "$version" \
            "$contents/Info.plist"
        /usr/bin/plutil -replace CFBundleVersion -string "$version" \
            "$contents/Info.plist"
        chmod 755 "$contents/MacOS/mew-desktop" "$contents/MacOS/mew" \
            "$contents/MacOS/agent-browser" \
            "$contents/MacOS/mew-cef-host-helper" \
            "$frameworks"/*/Contents/MacOS/*
        echo "✓ packaged $app"
        ;;
    *)
        echo "native desktop binaries are built in target/release; app bundling is currently macOS-only" >&2
        ;;
esac
