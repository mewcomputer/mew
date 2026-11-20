import csv
import json
import math
from pathlib import Path


def require(condition, message):
    if not condition:
        raise ValueError(message)


with Path("wine.csv").open(newline="") as source:
    rows = list(csv.DictReader(source, delimiter=";"))
answer = json.loads(Path("result.json").read_text())
require(set(answer) == {"rows", "high_quality", "mean_alcohol"}, "incorrect keys")
require(
    type(answer["rows"]) is int and answer["rows"] == len(rows), "incorrect row count"
)
require(
    type(answer["high_quality"]) is int
    and answer["high_quality"] == sum(int(row["quality"]) >= 7 for row in rows),
    "incorrect high-quality count",
)
require(type(answer["mean_alcohol"]) in (int, float), "mean must be numeric")
require(
    math.isclose(
        answer["mean_alcohol"],
        sum(float(row["alcohol"]) for row in rows) / len(rows),
        rel_tol=1e-6,
    ),
    "incorrect alcohol mean",
)
events = [json.loads(line) for line in Path("../events.jsonl").read_text().splitlines()]
reads = {
    event["call_id"]
    for event in events
    if event.get("type") == "nested_tool_start"
    and event.get("tool_name", "").startswith("mcp__files__read")
}
require(
    any(
        event.get("type") == "nested_tool_end"
        and event.get("call_id") in reads
        and event.get("success") is True
        for event in events
    ),
    "expected a successful MCP read",
)
