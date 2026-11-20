import csv
import importlib.util
import json
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from types import SimpleNamespace

spec = importlib.util.spec_from_file_location(
    "eval_runner", Path(__file__).resolve().parents[1] / "run.py"
)
runner = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runner)


class ManifestTests(unittest.TestCase):
    def test_mode_matrix_defaults_to_full_and_rejects_duplicates(self):
        self.assertEqual(runner.parse_modes(None), ["full"])
        self.assertEqual(runner.parse_modes(["full", "off"]), ["full", "off"])
        with self.assertRaises(ValueError):
            runner.parse_modes(["full", "full"])

    def test_binary_hash_is_recorded_from_file_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "binary"
            path.write_bytes(b"abc")
            self.assertEqual(
                runner.file_sha256(path),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            )

    def test_custom_config_preserves_provider_settings_and_owns_code_mode(self):
        base = '[providers.example]\nshape = "openai"\nbase_url = "https://example.invalid"\n'
        result = runner.make_config(base, {"mode": "full", "isolated": True})
        parsed = __import__("tomllib").loads(result)
        self.assertEqual(parsed["providers"]["example"]["shape"], "openai")
        self.assertEqual(parsed["code_mode"]["mode"], "full")
        with self.assertRaises(ValueError):
            runner.make_config('[code_mode]\nmode = "off"\n', {"mode": "full"})

    def test_model_matrix_requires_unambiguous_provider_names(self):
        self.assertEqual(
            runner.parse_models(
                None, ["codex/gpt-6-luna", "opencode-go/hy4-flash-free"]
            ),
            [("codex", "gpt-6-luna"), ("opencode-go", "hy4-flash-free")],
        )
        self.assertEqual(
            runner.parse_models("codex", ["gpt-6-luna"]), [("codex", "gpt-6-luna")]
        )
        with self.assertRaises(ValueError):
            runner.parse_models(None, ["gpt-6-luna"])

    def test_manifest_resolves_task_files_relative_to_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "prompt.txt").write_text("count the rows")
            (root / "fixture").mkdir()
            (root / "suite.toml").write_text(
                '[[tasks]]\nid = "count"\nprompt = "prompt.txt"\nfixture = "fixture"\n'
            )
            tasks = runner.load_manifest(root / "suite.toml")
            self.assertEqual(tasks[0]["prompt"], (root / "prompt.txt").resolve())
            self.assertEqual(tasks[0]["fixture"], (root / "fixture").resolve())

    def test_manifest_rejects_duplicate_and_unsafe_ids(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "prompt.txt").write_text("test")
            for task_ids in [("same", "same"), ("../escape",)]:
                manifest = root / "suite.toml"
                manifest.write_text(
                    "\n".join(
                        f'[[tasks]]\nid = "{item}"\nprompt = "prompt.txt"\n'
                        for item in task_ids
                    )
                )
                with self.assertRaises(ValueError):
                    runner.load_manifest(manifest)
            manifest.write_text('tasks = ["not a task table"]\n')
            with self.assertRaises(TypeError):
                runner.load_manifest(manifest)


class StreamTests(unittest.TestCase):
    def test_summary_requires_a_single_terminal_event(self):
        records = [
            {"type": "run_started", "session_id": "id", "version": 1},
            {
                "type": "run_finished",
                "status": "completed",
                "responses": 2,
                "nested_tool_calls": 3,
                "usage": {"input": 30, "output": 4},
            },
        ]
        parsed = runner.read_stream("\n".join(json.dumps(x) for x in records))
        self.assertEqual(parsed["responses"], 2)
        self.assertEqual(parsed["usage"]["input"], 30)
        failed = [records[0], {**records[1], "status": "failed", "error": "turn limit"}]
        self.assertEqual(
            runner.read_stream("\n".join(json.dumps(x) for x in failed))["status"],
            "failed",
        )
        for invalid in [
            records[:-1],
            records + records[-1:],
            [{"type": "run_finished", "status": "failed"}],
            ["garbage"],
        ]:
            with self.assertRaises(ValueError):
                runner.read_stream("\n".join(json.dumps(x) for x in invalid))

    def test_checker_runs_in_workspace_and_reports_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "answer.txt").write_text("42")
            okay = [
                sys.executable,
                "-c",
                "from pathlib import Path; assert Path('answer.txt').read_text() == '42'",
            ]
            bad = [sys.executable, "-c", "raise SystemExit(9)"]
            self.assertEqual(runner.run_checker(okay, root).returncode, 0)
            self.assertEqual(runner.run_checker(bad, root).returncode, 9)

    def test_auth_copy_cannot_rotate_the_original_refresh_token(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "auth.json"
            source.write_text(
                json.dumps(
                    {
                        "tokens": {
                            "access_token": "access",
                            "refresh_token": "original",
                            "expires_at": int(time.time()) + 3600,
                        },
                        "extra_headers": [],
                    }
                )
            )
            target = root / "copy.json"
            runner.copy_auth_file(source, target)
            self.assertEqual(
                json.loads(target.read_text())["tokens"]["refresh_token"], ""
            )
            self.assertEqual(
                json.loads(source.read_text())["tokens"]["refresh_token"], "original"
            )
            expired = json.loads(source.read_text())
            expired["tokens"]["expires_at"] = 0
            source.write_text(json.dumps(expired))
            with self.assertRaises(ValueError):
                runner.copy_auth_file(source, target)

    def test_stopping_a_run_reaps_its_process(self):
        process = subprocess.Popen(
            [sys.executable, "-c", "import time; time.sleep(60)"],
            start_new_session=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        runner.stop_process(process)
        self.assertIsNotNone(process.returncode)

    def test_failed_process_does_not_run_checker(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            prompt = root / "prompt.txt"
            prompt.write_text("test")
            output = root / "output"
            output.mkdir()
            task = {
                "id": "failure",
                "prompt": prompt,
                "manifest_dir": root,
                "checker": [
                    sys.executable,
                    "-c",
                    "from pathlib import Path; Path('checker-ran').touch()",
                ],
            }
            options = SimpleNamespace(
                mode="full",
                isolated=True,
                python=None,
                auth_file=None,
                config_text="",
                mew=Path(sys.executable),
                provider="none",
                model="none",
                max_turns=1,
                timeout=10,
            )
            result = runner.run_case(task, options, output)
            self.assertEqual(result["status"], "failed")
            self.assertFalse((output / "workspace/checker-ran").exists())
            self.assertTrue((output / "sessions").is_dir())

    def test_expands_workspace_in_mcp_configuration(self):
        self.assertEqual(
            runner.expand(
                {"args": ["${workspace}/input", "${manifest_dir}/server"]},
                Path("/tmp/work"),
                Path("/tmp/suite"),
            ),
            {"args": ["/tmp/work/input", "/tmp/suite/server"]},
        )

    def test_results_include_failed_runs_in_csv(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            results = [
                {
                    "task": "one",
                    "provider": "p",
                    "model": "m",
                    "repeat": 1,
                    "status": "passed",
                    "run": {
                        "responses": 3,
                        "nested_tool_calls": 2,
                        "usage": {"input": 42, "output": 7},
                    },
                },
                {
                    "task": "two",
                    "provider": "p",
                    "model": "m",
                    "repeat": 1,
                    "status": "failed",
                },
            ]
            runner.write_results(root, results)
            with (root / "results.csv").open(newline="") as stream:
                rows = list(csv.DictReader(stream))
            self.assertEqual(rows[0]["input_tokens"], "42")
            self.assertEqual(rows[1]["status"], "failed")
            self.assertEqual(rows[1]["input_tokens"], "")

    def test_example_checker_rejects_wrong_answer_with_optimization_enabled(self):
        example = Path(__file__).resolve().parents[1] / "examples/wine"
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            workspace = root / "workspace"
            workspace.mkdir()
            (workspace / "wine.csv").write_bytes(
                (example / "fixture/wine.csv").read_bytes()
            )
            (root / "events.jsonl").write_text(
                "\n".join(
                    [
                        json.dumps(
                            {
                                "type": "nested_tool_start",
                                "call_id": "read",
                                "tool_name": "mcp__files__read_text_file",
                            }
                        ),
                        json.dumps(
                            {
                                "type": "nested_tool_end",
                                "call_id": "read",
                                "success": True,
                            }
                        ),
                    ]
                )
            )
            (workspace / "result.json").write_text(
                json.dumps({"rows": 20, "high_quality": 3, "mean_alcohol": 9.605})
            )
            command = [sys.executable, "-O", str(example / "check.py")]
            self.assertEqual(runner.run_checker(command, workspace).returncode, 0)
            (workspace / "result.json").write_text(
                json.dumps({"rows": 21, "high_quality": 3, "mean_alcohol": 9.605})
            )
            self.assertNotEqual(runner.run_checker(command, workspace).returncode, 0)


if __name__ == "__main__":
    unittest.main()
