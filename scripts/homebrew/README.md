# Homebrew tap

The public tap lives at [mewcomputer/homebrew-tap](https://github.com/mewcomputer/homebrew-tap).
Install or upgrade the CLI with Homebrew.

```sh
brew tap mewcomputer/tap
brew install mewcomputer/tap/mew
brew upgrade mewcomputer/tap/mew
```

## Nightly updates

After publishing the rolling nightly, the Nightly workflow publishes a
versioned prerelease with four CLI archives, their checksums, and the generated
formula. The tap checks for new snapshots twice an hour and only moves the
formula forward. Each workflow can write to its own repository.

## Generate a formula for a stable release

Run the generator after the stable release has all four CLI archives and
`SHA256SUMS`.

```sh
scripts/generate-homebrew-formula.sh v0.2.0 > ../homebrew-tap/Formula/mew.rb
```

Test the formula before publishing it.
