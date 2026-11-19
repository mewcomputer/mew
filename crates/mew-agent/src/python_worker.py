import contextlib
import io
import json
import os
import sys
import threading
import traceback

protocol = os.fdopen(os.dup(1), "w", encoding="utf-8", buffering=1)
with open(os.devnull, "w") as null:
    os.dup2(null.fileno(), 1)

catalog = {}
next_id = 0
active_cell = None
images = []
call_lock = threading.Lock()

def send(value):
    value["cell"] = active_cell
    protocol.write(json.dumps(value, ensure_ascii=False, separators=(",", ":")) + "\n")
    protocol.flush()

def call(name, arguments):
    global next_id
    if name not in catalog:
        raise ValueError("unknown or unavailable tool: " + name)
    with call_lock:
        next_id += 1
        call_id = next_id
        send({"type": "tool", "id": call_id, "name": catalog[name], "input": arguments})
        line = sys.stdin.readline()
        if not line:
            raise EOFError("mew tool bridge disconnected")
        result = json.loads(line)
        if result.get("type") != "tool_result" or result.get("id") != call_id or result.get("cell") != active_cell:
            raise ValueError("invalid tool bridge response")
        return result["result"]

class Tools:
    def __getattr__(self, name):
        if name.startswith("_"):
            raise AttributeError(name)
        return lambda arguments: call(name, arguments)

    def call(self, name, arguments):
        return call(name, arguments)

def text(value):
    print(value if isinstance(value, str) else json.dumps(value, ensure_ascii=False, separators=(",", ":")))

def emit_image(image):
    if not isinstance(image, dict) or not isinstance(image.get("mime"), str) or not isinstance(image.get("data"), str):
        raise ValueError("image must have mime and base64 data strings")
    if sum(len(item["data"]) for item in images) + len(image["data"]) > 12_000_000:
        raise ValueError("emitted images are too large")
    images.append({"mime": image["mime"], "data": image["data"]})

class CapturedOutput(io.StringIO):
    def __init__(self):
        super().__init__()
        self.truncated = False

    def write(self, value):
        remaining = max(0, 100_000 - self.tell())
        super().write(value[:remaining])
        self.truncated |= len(value) > remaining
        return len(value)

    def render(self):
        return self.getvalue() + ("\n[output truncated]\n" if self.truncated else "")

scope = {"tools": Tools(), "text": text, "emit_image": emit_image, "ALL_TOOLS": []}
send({"type": "ready"})
for line in sys.stdin:
    output = CapturedOutput()
    images = []
    try:
        request = json.loads(line)
        if request.get("type") != "exec":
            raise ValueError("expected an exec request")
        active_cell = request["cell"]
        entries = request.get("tools", [])
        catalog = {entry["tool_name"]: entry["tool_name"] for entry in entries}
        catalog.update({entry["name"]: entry["tool_name"] for entry in entries})
        scope["ALL_TOOLS"] = entries
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(output):
            exec(compile(request["source"], "<mew-python>", "exec"), scope)
        send({"type": "result", "output": output.render(), "images": images})
    except BaseException as error:
        send({"type": "error", "error": "".join(traceback.format_exception_only(type(error), error)).strip(), "output": output.render()})
