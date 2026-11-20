#!/usr/bin/env python3
"""Run a manifest of real tasks through mew and check their artifacts."""

import argparse
import copy
import csv
import hashlib
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path


def file_sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def make_config(base, code_mode):
    if "code_mode" in tomllib.loads(base):
        raise ValueError(
            "eval config must omit [code_mode]; use the runner's mode options"
        )
    # JSON strings and booleans are also valid TOML values.
    return (
        base
        + "\n[code_mode]\n"
        + "".join(f"{key} = {json.dumps(value)}\n" for key, value in code_mode.items())
    )


def parse_models(provider, models):
    result = []
    for value in models:
        if "/" in value:
            selected_provider, model = value.split("/", 1)
            if provider and selected_provider != provider:
                raise ValueError(
                    f"model {value!r} does not match provider {provider!r}"
                )
        else:
            selected_provider, model = provider, value
        if not selected_provider or not model:
            raise ValueError("use provider/model or specify --provider for each model")
        if (selected_provider, model) in result:
            raise ValueError(f"duplicate model: {selected_provider}/{model}")
        result.append((selected_provider, model))
    return result


def parse_modes(modes):
    modes = modes or ["full"]
    if len(set(modes)) != len(modes):
        raise ValueError("duplicate code mode")
    return modes


def write_results(output, results):
    (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    columns = [
        "case",
        "task",
        "repeat",
        "provider",
        "model",
        "mode",
        "isolated",
        "status",
        "elapsed_seconds",
        "responses",
        "tool_calls",
        "failed_tool_calls",
        "nested_tool_calls",
        "failed_nested_tool_calls",
        "max_tool_output_bytes",
        "max_nested_output_bytes",
        "input_tokens",
        "output_tokens",
        "reasoning_tokens",
        "cache_read_tokens",
        "cache_write_tokens",
        "reported_cost",
    ]
    with (output / "results.csv").open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=columns)
        writer.writeheader()
        for result in results:
            run = result.get("run", {})
            usage = run.get("usage", {})
            writer.writerow(
                {
                    **{key: result.get(key) for key in columns},
                    "responses": run.get("responses"),
                    "tool_calls": run.get("tool_calls"),
                    "failed_tool_calls": run.get("failed_tool_calls"),
                    "nested_tool_calls": run.get("nested_tool_calls"),
                    "failed_nested_tool_calls": run.get("failed_nested_tool_calls"),
                    "max_tool_output_bytes": run.get("max_tool_output_bytes"),
                    "max_nested_output_bytes": run.get("max_nested_output_bytes"),
                    "input_tokens": usage.get("input"),
                    "output_tokens": usage.get("output"),
                    "reasoning_tokens": usage.get("reasoning"),
                    "cache_read_tokens": usage.get("cache_read"),
                    "cache_write_tokens": usage.get("cache_write"),
                    "reported_cost": run.get("cost"),
                }
            )


def load_manifest(path):
    path = Path(path).resolve()
    with path.open("rb") as stream:
        manifest = tomllib.load(stream)
    tasks = manifest.get("tasks", [])
    if not isinstance(tasks, list) or not tasks:
        raise ValueError("manifest must contain at least one [[tasks]] entry")
    seen = set()
    for task in tasks:
        if not isinstance(task, dict):
            raise TypeError("each task must be a table")
        name = task.get("id")
        if (
            not isinstance(name, str)
            or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_-]*", name)
            or name in seen
        ):
            raise ValueError(f"invalid or duplicate task id: {name!r}")
        seen.add(name)
        for key in ("prompt", "fixture", "mcp"):
            if key in task:
                task[key] = (path.parent / task[key]).resolve()
        if "prompt" not in task or not task["prompt"].is_file():
            raise ValueError(f"{name}: prompt must name an existing file")
        if "fixture" in task and not task["fixture"].is_dir():
            raise ValueError(f"{name}: fixture must name an existing directory")
        if "mcp" in task and not task["mcp"].is_file():
            raise ValueError(f"{name}: mcp must name an existing JSON file")
        if "checker" in task and (
            not isinstance(task["checker"], list)
            or not task["checker"]
            or not all(isinstance(x, str) for x in task["checker"])
        ):
            raise ValueError(f"{name}: checker must be a nonempty array of strings")
        task["manifest_dir"] = path.parent
    return tasks


def read_stream(text):
    try:
        records = [json.loads(line) for line in text.splitlines() if line.strip()]
    except json.JSONDecodeError as error:
        raise ValueError(f"invalid JSONL: {error}") from error
    if not all(isinstance(record, dict) for record in records):
        raise ValueError("JSONL records must be objects")
    starts = [record for record in records if record.get("type") == "run_started"]
    ends = [record for record in records if record.get("type") == "run_finished"]
    if (
        len(starts) != 1
        or starts[0].get("version") != 1
        or len(ends) != 1
        or ends[0].get("status") not in ("completed", "failed")
    ):
        raise ValueError("stream must contain one version 1 start and one finish")
    if records[0] != starts[0] or records[-1] != ends[0]:
        raise ValueError("start and finish must bracket the event stream")
    return {**ends[0], "session_id": starts[0]["session_id"]}


def run_checker(command, workspace):
    return subprocess.run(
        command, cwd=workspace, capture_output=True, text=True, timeout=120, check=False
    )


def stop_process(process):
    if process.poll() is not None:
        return
    try:
        process.send_signal(signal.SIGINT)
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()
    except ProcessLookupError:
        process.wait()


def copy_auth_file(source, target):
    try:
        auth = json.loads(Path(source).read_text())
        tokens = auth["tokens"]
        if not isinstance(tokens["access_token"], str) or not isinstance(
            tokens["expires_at"], int
        ):
            raise TypeError("invalid token fields")
        if tokens["expires_at"] <= time.time() + 300:
            raise ValueError("OAuth access token expires within five minutes")
        tokens["refresh_token"] = ""
    except (KeyError, TypeError, json.JSONDecodeError) as error:
        raise ValueError("invalid Mew OAuth token file") from error
    fd = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as stream:
        json.dump(auth, stream)


def expand(value, workspace, manifest_dir):
    if isinstance(value, str):
        return value.replace("${workspace}", str(workspace)).replace(
            "${manifest_dir}", str(manifest_dir)
        )
    if isinstance(value, list):
        return [expand(item, workspace, manifest_dir) for item in value]
    if isinstance(value, dict):
        return {
            key: expand(item, workspace, manifest_dir) for key, item in value.items()
        }
    return value


def run_case(task, options, output):
    workspace = output / "workspace"
    if "fixture" in task:
        shutil.copytree(task["fixture"], workspace)
    else:
        workspace.mkdir()
    if "mcp" in task:
        if (workspace / "mcp.json").exists():
            raise ValueError(f"{task['id']}: fixture already has mcp.json")
        config = json.loads(task["mcp"].read_text())
        (workspace / "mcp.json").write_text(
            json.dumps(expand(config, workspace, task["manifest_dir"]))
        )
    (output / "prompt.txt").write_text(task["prompt"].read_text())
    with tempfile.TemporaryDirectory(prefix="mew-eval-state-") as temporary:
        state = Path(temporary)
        for directory in ("config", "data", "cache", "sessions"):
            (state / directory).mkdir()
        config = {"mode": options.mode, "isolated": options.isolated}
        if options.python:
            config["python"] = str(Path(options.python).resolve())
        (state / "config/config.toml").write_text(
            make_config(options.config_text, config)
        )
        if options.auth_file and options.provider == "codex":
            auth = state / "config/auth"
            auth.mkdir(mode=0o700)
            copy_auth_file(options.auth_file, auth / "codex.json")
        env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith("MEW_") or key.startswith("MEW_CRED_")
        }
        env.update(
            {
                "MEW_CONFIG_DIR": str(state / "config"),
                "MEW_DATA_DIR": str(state / "data"),
                "MEW_CACHE_DIR": str(state / "cache"),
                "MEW_SESSION_DIR": str(state / "sessions"),
                "MEW_MAX_TURNS": str(options.max_turns),
            }
        )
        command = [
            str(options.mew),
            "run",
            "--format",
            "jsonl",
            "--provider",
            options.provider,
            "--model",
            options.model,
            (output / "prompt.txt").read_text(),
        ]
        started = time.monotonic()
        timed_out = False
        with (
            (output / "events.jsonl").open("w") as stdout,
            (output / "stderr.log").open("w") as stderr,
        ):
            process = subprocess.Popen(
                command,
                cwd=workspace,
                env=env,
                stdout=stdout,
                stderr=stderr,
                start_new_session=True,
            )
            try:
                exit_code = process.wait(timeout=options.timeout)
            except subprocess.TimeoutExpired:
                stop_process(process)
                exit_code = process.returncode
                timed_out = True
            except BaseException:
                stop_process(process)
                shutil.copytree(state / "sessions", output / "sessions")
                raise
        shutil.copytree(state / "sessions", output / "sessions")
        result = {
            "status": "timeout" if timed_out else "failed",
            "exit_code": exit_code,
            "elapsed_seconds": time.monotonic() - started,
        }
        try:
            result["run"] = read_stream((output / "events.jsonl").read_text())
        except (ValueError, KeyError) as error:
            result["error"] = str(error)
            return result
        if timed_out or exit_code != 0 or result["run"]["status"] != "completed":
            return result
        if "checker" in task:
            checker = expand(task["checker"], workspace, task["manifest_dir"])
            try:
                checked = run_checker(checker, workspace)
            except (OSError, subprocess.TimeoutExpired) as error:
                result["error"] = f"checker failed: {error}"
                return result
            (output / "checker.stdout").write_text(checked.stdout)
            (output / "checker.stderr").write_text(checked.stderr)
            result["checker_exit_code"] = checked.returncode
            if checked.returncode != 0:
                return result
        result["status"] = "passed" if "checker" in task else "ungraded"
        return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--mew", type=Path, required=True)
    parser.add_argument("--provider", help="provider for unqualified model names")
    parser.add_argument(
        "--model",
        action="append",
        required=True,
        help="provider/model; repeat to compare models",
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=1)
    parser.add_argument("--max-turns", type=int, default=200)
    parser.add_argument("--timeout", type=int, default=600)
    parser.add_argument(
        "--mode",
        action="append",
        choices=("full", "off"),
        help="repeat to compare code and direct modes",
    )
    parser.add_argument(
        "--no-isolation", dest="isolated", action="store_false", default=True
    )
    parser.add_argument("--python", type=Path)
    parser.add_argument(
        "--config", type=Path, help="copy a TOML config that omits [code_mode]"
    )
    parser.add_argument(
        "--auth-file",
        type=Path,
        help="copy a Mew Codex OAuth token file into temporary state",
    )
    options = parser.parse_args()
    if options.repeats < 1 or options.max_turns < 1 or options.timeout < 1:
        parser.error("repeats, max-turns, and timeout must be positive")
    options.mew = options.mew.resolve(strict=True)
    binary_sha256 = file_sha256(options.mew)
    config_bytes = options.config.read_bytes() if options.config else b""
    options.config_text = config_bytes.decode("utf-8")
    config_sha256 = hashlib.sha256(config_bytes).hexdigest() if options.config else None
    modes = parse_modes(options.mode)
    make_config(options.config_text, {"mode": modes[0], "isolated": options.isolated})
    tasks = load_manifest(options.manifest)
    models = parse_models(options.provider, options.model)
    output = options.output.resolve()
    output.mkdir(mode=0o700, parents=True, exist_ok=False)
    results = []
    for task in tasks:
        for model_index, (provider, model) in enumerate(models, 1):
            model_options = copy.copy(options)
            model_options.provider, model_options.model = provider, model
            for mode in modes:
                model_options.mode = mode
                for repeat in range(1, options.repeats + 1):
                    if file_sha256(options.mew) != binary_sha256:
                        raise ValueError("mew binary changed during the evaluation")
                    case = output / f"{task['id']}-{mode}-{model_index:02}-{repeat:03}"
                    case.mkdir(mode=0o700)
                    result = run_case(task, model_options, case)
                    result.update(
                        {
                            "case": case.name,
                            "task": task["id"],
                            "repeat": repeat,
                            "provider": provider,
                            "model": model,
                            "mode": mode,
                            "isolated": options.isolated,
                            "max_turns": options.max_turns,
                            "timeout_seconds": options.timeout,
                            "python": str(options.python.resolve())
                            if options.python
                            else None,
                            "mew_sha256": binary_sha256,
                            "config_sha256": config_sha256,
                        }
                    )
                    results.append(result)
                    write_results(output, results)
                    print(f"{case.name}: {result['status']}", flush=True)
    return (
        1
        if any(
            result["status"] == "failed" or result["status"] == "timeout"
            for result in results
        )
        else 0
    )


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("eval: interrupted", file=sys.stderr)
        sys.exit(130)
    except (ValueError, TypeError, OSError, tomllib.TOMLDecodeError) as error:
        print(f"eval: {error}", file=sys.stderr)
        sys.exit(2)
