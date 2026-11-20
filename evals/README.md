# Mew evaluations

`run.py` runs tasks through the real `mew run` command, saves the event stream
and session history, and executes an optional checker. It requires Python 3.11
or newer. A task can use real MCP servers, local data, or both.

## Run the example

Build Mew and configure a provider credential in the environment. This example
uses an OpenCode Go key and starts the pinned filesystem MCP server with `npx`:

```sh
cargo build --locked -p mew
export MEW_CRED_OPENCODE_ZEN="your-api-key"
python3 evals/run.py evals/examples/wine/suite.toml \
  --mew target/debug/mew \
  --model opencode-go/hy4-flash-free \
  --output /tmp/mew-eval-wine
```

The output directory must not exist. The example contains the first 20 rows of
the red [Wine Quality dataset](https://doi.org/10.24432/C56S3T) by P. Cortez,
A. Cerdeira, F. Almeida, T. Matos and J. Reis (2009), distributed by the UCI
Machine Learning Repository under [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
Its checker verifies the computed values and that the run used an MCP read.
Node.js and `npx` are required; the first run may download the server package.

Repeat `--model provider/model` to compare models in the same output directory,
and use `--repeats 3` for multiple runs per task and model. Runs execute
sequentially. `--provider` can supply the provider for unqualified model names.
The default is full code mode with Python isolation, 200 model responses, and a
600-second timeout. Use `--mode full --mode off` to compare code mode with
direct tools, `--no-isolation` to disable the Python sandbox, or
`--python /path/to/venv/bin/python` to select a Python environment with
scientific packages.
The example checks for a Python-to-MCP call, so use a mode-neutral task and
checker when comparing `full` and `off`.

The runner creates a fresh workspace and temporary Mew configuration, data,
cache and session directories for each run. It passes `MEW_CRED_*` variables
from the environment. For a Mew Codex OAuth login, pass `--auth-file` with the
path to Mew's `auth/codex.json`. The runner copies a valid access token into
temporary state with the refresh token removed. It rejects a token that expires
within five minutes, and does not refresh or alter the original login. Runs
that outlast the access token can fail.

For custom providers or other settings, pass `--config /path/to/config.toml`.
The runner copies that configuration into temporary state. It must omit
`[code_mode]`, which the runner supplies from its flags. Results record hashes
of the Mew binary and supplied configuration.

Each case retains its prompt, workspace, `events.jsonl`, stderr, session
history and checker output. `results.json` contains the complete summaries;
`results.csv` contains one row per case for comparison. The output can contain
task data, model responses and MCP configuration, so keep it somewhere
appropriate for that data.
`passed` means the run and checker succeeded, `ungraded` means no checker was
configured, and `failed` or `timeout` means the case did not pass. The command
exits nonzero if a case fails or times out.

Mew's noninteractive command automatically allows permission prompts. MCP
servers and checkers execute local commands with the runner's permissions.
Use trusted manifests, fixtures and MCP configurations.

## Write a suite

A TOML manifest contains one or more tasks:

```toml
[[tasks]]
id = "example"
prompt = "prompt.txt"
fixture = "fixture"
mcp = "mcp.json"
checker = ["python3", "${manifest_dir}/check.py"]
```

Paths are relative to the manifest. `prompt` is required; `fixture`, `mcp`
and `checker` are optional. A fixture directory is copied into a new workspace
for every case. An MCP JSON file is copied to that workspace as `mcp.json`;
`${workspace}` and `${manifest_dir}` are expanded in its string values and in
checker arguments. The checker runs in the workspace without a shell and must
exit zero for a pass. It can read `../events.jsonl` to check tool use.

Model response, tool-call and usage counts come from the `run_finished` event.
They cover the top-level agent; child subagent usage is not included. Token and
cost fields are provider-reported estimates, not verified billing figures.

Run the runner tests with:

```sh
just test-evals
```
