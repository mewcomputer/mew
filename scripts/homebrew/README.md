# Homebrew tap

The official tap lives at `github.com/mewcomputer/homebrew-mew`. To publish a new formula after a release:

1. Create the tap repo if it does not exist:
   ```sh
   mkdir -p homebrew-mew/Formula
   cd homebrew-mew
   git init
   git remote add origin git@github.com:mewcomputer/homebrew-mew.git
   ```

2. Generate the formula from the GitHub release:
   ```sh
   ../mew/scripts/generate-homebrew-formula.sh v0.2.0 > Formula/mew.rb
   ```

3. Commit and push:
   ```sh
   git add Formula/mew.rb
   git commit -m "mew v0.2.0"
   git push origin main
   ```

Users can then install with:

```sh
brew tap mewcomputer/mew
brew install mew
```
