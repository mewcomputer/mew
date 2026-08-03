#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
profile="${MEW_DESKTOP_PROFILE:-release}"

case "$profile" in
    debug|release) ;;
    *)
        echo "unsupported desktop profile: $profile" >&2
        exit 1
        ;;
esac

case "$(uname -s)" in
    Darwin)
        target_dir="$repo_root/target/$profile"
        app="$target_dir/bundle/macos/mew.app"
        contents="$app/Contents"
        frameworks="$contents/Frameworks"
        mkdir -p "$contents/MacOS" "$contents/Resources" "$frameworks"
        rm -f "$contents/MacOS/mew-desktop" "$contents/MacOS/mew"
        cp "$target_dir/mew-desktop" "$contents/MacOS/mew-desktop"
        cp "$target_dir/mew" "$contents/MacOS/mew"
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
        done
        cp "$repo_root/apps/mew-desktop/Info.plist" "$contents/Info.plist"
        chmod 755 "$contents/MacOS/mew-desktop" "$contents/MacOS/mew" \
            "$contents/MacOS/mew-cef-host-helper" \
            "$frameworks"/*/Contents/MacOS/*
        echo "✓ packaged $app"
        ;;
    *)
        echo "native desktop binaries are built in target/release; app bundling is currently macOS-only" >&2
        ;;
esac
