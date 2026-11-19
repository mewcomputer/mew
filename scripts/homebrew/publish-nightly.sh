#!/usr/bin/env bash
set -euo pipefail

: "${GITHUB_REPOSITORY:?}" "${GITHUB_RUN_ID:?}" "${GITHUB_RUN_NUMBER:?}" "${GITHUB_SHA:?}"
artifacts="${1:-artifacts}"
run_date="$(gh api "repos/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID" --jq '.created_at[:10]' | tr -d '-')"
base_version="$(awk '/^\[workspace\.package\]$/ { package=1; next } package && /^version = / { gsub(/"/, "", $3); print $3; exit }' Cargo.toml)"
if [[ ! "$run_date" =~ ^[0-9]{8}$ || ! "$base_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ || ! "$GITHUB_RUN_NUMBER" =~ ^[0-9]+$ ]]; then
    echo 'Expected a release version and a dated GitHub Actions run' >&2
    exit 1
fi

version="v$base_version-pre.$run_date.$GITHUB_RUN_NUMBER"
tag="nightly-$version"
draft="$(gh release view "$tag" --repo "$GITHUB_REPOSITORY" --json isDraft --jq .isDraft 2>/dev/null || true)"
if [[ "$draft" == false ]]; then
    echo "$tag is already published."
    exit 0
fi

work="$(mktemp -d "${artifacts%/}/homebrew-release.XXXXXX")"
trap 'rm -rf "$work"' EXIT
targets=(aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu)
for target in "${targets[@]}"; do
    ln "$artifacts/mew-nightly-$target.tar.gz" "$work/mew-$version-$target.tar.gz"
done
(cd "$work" && shasum -a 256 mew-*.tar.gz > SHA256SUMS)
bash scripts/generate-homebrew-formula.sh "$tag" "$GITHUB_REPOSITORY" "$work/SHA256SUMS" > "$work/mew.rb"

if [[ "$draft" != true ]]; then
    gh release create "$tag" --repo "$GITHUB_REPOSITORY" --target "$GITHUB_SHA" --draft --prerelease \
        --title "Homebrew nightly $version" \
        --notes "Pinned CLI archives for the Homebrew tap from https://github.com/$GITHUB_REPOSITORY/actions/runs/$GITHUB_RUN_ID."
fi
gh release upload "$tag" --repo "$GITHUB_REPOSITORY" --clobber "$work"/*
gh release edit "$tag" --repo "$GITHUB_REPOSITORY" --draft=false
