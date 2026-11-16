#!/usr/bin/env bash
set -euo pipefail

# Match the CEF version in Cargo.lock; the bundled downloader's index size limit
# is smaller than the current upstream index.
destination="$1"
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64)
        platform=linux64
        sha1=2e2f9915f134d237ac4fd7b8c6ad893bfc6cc544
        ;;
    Darwin-arm64)
        platform=macosarm64
        sha1=47baa745213c938861861f0f3f65c6dd03e7254e
        ;;
    *)
        echo "unsupported CEF CI platform: $(uname -s)-$(uname -m)" >&2
        exit 1
        ;;
esac

archive="cef_binary_150.0.14+g7c1aa68+chromium-150.0.7871.129_${platform}_minimal.tar.bz2"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
curl --fail --location --retry 3 \
    "https://cef-builds.spotifycdn.com/$archive" --output "$work/$archive"
printf '%s  %s\n' "$sha1" "$work/$archive" | shasum -a 1 -c -
tar -xjf "$work/$archive" -C "$work"
source_dir="$work/${archive%.tar.bz2}"
mkdir -p "$destination"
mv "$source_dir/Release/"* "$destination/"
if [[ "$platform" == linux64 ]]; then
    mv "$source_dir/Resources/"* "$destination/"
fi
mv "$source_dir/CMakeLists.txt" "$source_dir/cmake" "$source_dir/include" \
    "$source_dir/libcef_dll" "$source_dir/CREDITS.html" "$destination/"
printf '{"type":"minimal","name":"%s","sha1":"%s"}\n' "$archive" "$sha1" > "$destination/archive.json"
