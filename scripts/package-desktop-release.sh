#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
profile="${MEW_DESKTOP_PROFILE:-release}"
if [[ "$profile" != "release" ]]; then
    echo "desktop release packaging requires MEW_DESKTOP_PROFILE=release" >&2
    exit 1
fi

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

case "$(uname -s)" in
    Darwin) ;;
    *)
        echo "desktop release packaging currently supports macOS only" >&2
        exit 1
        ;;
esac

case "$(uname -m)" in
    arm64) arch="aarch64" ;;
    x86_64) arch="x86_64" ;;
    *)
        echo "unsupported macOS architecture: $(uname -m)" >&2
        exit 1
        ;;
esac

app="$repo_root/target/$profile/bundle/macos/mew.app"
if [[ ! -d "$app" ]]; then
    echo "release app not found at $app; run just desktop-build first" >&2
    exit 1
fi

for path in \
    "$app/Contents/MacOS/mew-desktop" \
    "$app/Contents/MacOS/mew" \
    "$app/Contents/Frameworks/Chromium Embedded Framework.framework"; do
    if [[ ! -e "$path" ]]; then
        echo "release app is incomplete: $path" >&2
        exit 1
    fi
done

output_dir="${MEW_DESKTOP_OUTPUT_DIR:-$repo_root/target/$profile/dist}"
artifact_base="mew-desktop-v${version}-${arch}-apple-darwin"
stage="$output_dir/$artifact_base"
zip_path="$output_dir/$artifact_base.zip"
dmg_path="$output_dir/$artifact_base.dmg"
checksums_path="$output_dir/SHA256SUMS"

mkdir -p "$output_dir"
rm -rf "$stage"
rm -f "$zip_path" "$dmg_path" "$checksums_path"
mkdir -p "$stage"

ditto "$app" "$stage/mew.app"
if [[ -f "$repo_root/LICENSE" ]]; then
    cp "$repo_root/LICENSE" "$stage/LICENSE"
fi

cef_parent="$(dirname "$app/Contents/Frameworks/Chromium Embedded Framework.framework")"
if [[ -f "$cef_parent/CREDITS.html" ]]; then
    cp "$cef_parent/CREDITS.html" "$stage/CEF-CREDITS.html"
else
    cef_source="${MEW_CEF_FRAMEWORK_SOURCE:-}"
    if [[ -n "$cef_source" && -d "$cef_source/Chromium Embedded Framework.framework" ]]; then
        cef_source="$(dirname "$cef_source")"
    elif [[ -n "$cef_source" && "$(basename "$cef_source")" == "Chromium Embedded Framework.framework" ]]; then
        cef_source="$(dirname "$cef_source")"
    fi
    if [[ -z "$cef_source" ]]; then
        cef_path="${CEF_PATH:-$HOME/.local/share/cef}"
        for candidate in "$cef_path"/*/cef_macos_*; do
            if [[ -f "$candidate/CREDITS.html" ]]; then
                cef_source="$candidate"
                break
            fi
        done
    fi
    if [[ -z "$cef_source" ]]; then
        for candidate in "$repo_root/target/$profile"/build/cef-dll-sys-*/out/cef_macos_*; do
            if [[ -f "$candidate/CREDITS.html" ]]; then
                cef_source="$candidate"
                break
            fi
        done
    fi
    if [[ -n "$cef_source" && -f "$cef_source/CREDITS.html" ]]; then
        cp "$cef_source/CREDITS.html" "$stage/CEF-CREDITS.html"
    else
        echo "CEF CREDITS.html was not found; refusing to ship an incomplete release" >&2
        exit 1
    fi
fi

cat > "$stage/RELEASE-METADATA.txt" <<EOF
mew desktop ${version}
architecture: ${arch}
signing: unsigned (not notarized)
EOF

ditto -c -k --norsrc --keepParent "$stage" "$zip_path"

dmg_stage="$output_dir/.${artifact_base}-dmg"
rm -rf "$dmg_stage"
mkdir -p "$dmg_stage"
ditto "$app" "$dmg_stage/mew.app"
for notice in LICENSE CEF-CREDITS.html RELEASE-METADATA.txt; do
    if [[ -f "$stage/$notice" ]]; then
        cp "$stage/$notice" "$dmg_stage/$notice"
    fi
done
ln -s /Applications "$dmg_stage/Applications"
hdiutil create -volname "mew ${version}" -srcfolder "$dmg_stage" -ov -format UDZO "$dmg_path" >/dev/null
rm -rf "$dmg_stage"

(
    cd "$output_dir"
    shasum -a 256 "$(basename "$zip_path")" "$(basename "$dmg_path")" > "$(basename "$checksums_path")"
)

echo "✓ packaged $zip_path"
echo "✓ packaged $dmg_path"
echo "✓ wrote $checksums_path"
