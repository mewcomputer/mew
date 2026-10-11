#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
source "$repo_root/scripts/cef-runtime.sh"
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

cef_framework="$app/Contents/Frameworks/Chromium Embedded Framework.framework"
cef_version="$(cef_runtime_version "$repo_root")"
if ! cef_framework_matches "$cef_framework" "$cef_version"; then
    echo "release app requires CEF runtime $cef_version; rebuild the app bundle first" >&2
    exit 1
fi
cef_credits="$(dirname "$cef_framework")/CREDITS.html"
if [[ ! -f "$cef_credits" ]]; then
    echo "bundled CEF CREDITS.html was not found; rebuild the app bundle first" >&2
    exit 1
fi

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

cp "$cef_credits" "$stage/CEF-CREDITS.html"

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
