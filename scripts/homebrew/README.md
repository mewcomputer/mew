# Homebrew tap

The public tap lives at [mewcomputer/homebrew-tap](https://github.com/mewcomputer/homebrew-tap).
Install or upgrade the CLI with Homebrew.

```sh
brew tap mewcomputer/tap
brew install mewcomputer/tap/mew
brew upgrade mewcomputer/tap/mew
```

## Publish a formula

Run the generator from the Mew checkout after the release has all four CLI
archives and `SHA256SUMS`. For a stable release in the main repository, use:

```sh
scripts/generate-homebrew-formula.sh v0.2.0 > ../homebrew-tap/Formula/mew.rb
```

For a dated nightly snapshot, verify the upstream archives against the nightly
checksums, then publish copies in a versioned prerelease on the tap repository.
Name each archive `mew-<tag>-<target>.tar.gz` and publish `SHA256SUMS` under
the same tag. Generate the formula with the tap repository as the second argument.

```sh
scripts/generate-homebrew-formula.sh v0.2.0-pre.20261118 mewcomputer/homebrew-tap > ../homebrew-tap/Formula/mew.rb
```

The dated prerelease keeps Homebrew's download URLs and checksums valid across
nightly rebuilds. Test the formula before pushing it to the tap's default branch.
