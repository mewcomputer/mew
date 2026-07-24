# Plan: Add a hidden `mew funfact` CLI subcommand

**Goal:** Add a hidden CLI subcommand that prints a random fun fact from an in-code list. The command is intentionally omitted from `mew --help` and shell completions as a small easter egg.

## Steps

1. **Add the `Funfact` variant to the CLI enum in `crates/mew/src/cli.rs`.**
   - Insert a new `Commands::Funfact` variant with `#[command(hide = true)]` so it parses but does not appear in help or completions.
   - Keep it simple: no arguments.

2. **Create `crates/mew/src/commands/funfact.rs`.**
   - Define a `const FACTS: &[&str]` with 5–8 short, inoffensive, true-ish fun facts.
   - Add a public `funfact_cmd() -> anyhow::Result<()>` that selects an entry and prints it to stdout.
   - Use `std::time::SystemTime` to pick an index without adding a `rand` dependency.

3. **Register the module in `crates/mew/src/commands/mod.rs`.**
   - Add `pub mod funfact;` next to the other command modules.

4. **Wire the subcommand in `crates/mew/src/main.rs`.**
   - Import `commands::funfact::funfact_cmd`.
   - Add `Some(Commands::Funfact) => funfact_cmd(),` in the `async_main` match expression before the `None` default arm.
   - No provider health check is needed for this command, so leave the `needs_provider_health_check` match unchanged.

5. **Add behavior tests in `crates/mew/src/commands/funfact.rs`.**
   - Test that the output is one of the known facts in the constant list.
   - Test that `Cli::try_parse_from(["mew", "funfact"])` succeeds and produces the right variant.
   - (Optional) Test that the command is hidden from the clap help output by inspecting the built command tree.

6. **Verify with a quick smoke test.**
   - `cargo build -p mew`
   - `cargo run -p mew -- funfact` prints a fact.
   - `cargo run -p mew -- --help` does not list `funfact`.
   - `cargo test -p mew` passes.

## Files touched

- `crates/mew/src/cli.rs` — add `Funfact` variant.
- `crates/mew/src/commands/funfact.rs` — new module with facts and command logic.
- `crates/mew/src/commands/mod.rs` — register the module.
- `crates/mew/src/main.rs` — dispatch to the new command.

## Risks / tradeoffs

- **No new dependencies:** Using `SystemTime` for randomness is low-quality but fine for an easter egg. If we later want cryptographic randomness, we can pull in `rand`.
- **Hidden from help but still discoverable:** Anyone reading `cli.rs` can see it. That is intentional for a lightweight hidden command.
- **YAGNI:** Keep the fact list hard-coded and the command self-contained. Do not add config, theming, or protocol support.
