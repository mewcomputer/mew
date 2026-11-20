import contextlib
import concurrent.futures
import io
import json
import os
import sys
import threading
import time
import traceback

protocol = os.fdopen(os.dup(1), "w", encoding="utf-8", buffering=1)
with open(os.devnull, "w") as null:
    os.dup2(null.fileno(), 1)

catalog = {}
next_id = 0
active_cell = None
active_token = None
closing = False
images = []
call_lock = threading.Lock()
image_lock = threading.Lock()
thread_cell = threading.local()
tool_threads = set()
pool_tasks = {}
pool_lock = threading.Lock()
start_thread = threading.Thread.start
submit_thread = concurrent.futures.ThreadPoolExecutor.submit
add_done_callback = concurrent.futures.Future.add_done_callback

# Retain the starting cell even when a thread runs during a later request.
def start_in_cell(thread):
    owner = getattr(thread_cell, "token", None)
    lineage = getattr(thread_cell, "lineage", ())
    parent_task = getattr(thread_cell, "task", None)
    if parent_task is not None:
        lineage += (parent_task,)
    run = thread.run
    def run_in_cell():
        thread_cell.token = owner
        thread_cell.lineage = lineage
        try:
            return run()
        finally:
            del thread_cell.token
            del thread_cell.lineage
    thread.run = run_in_cell
    return start_thread(thread)

threading.Thread.start = start_in_cell

# A pool's workers outlive individual tasks, so bind each submission to its
# submitting cell rather than to the cell that created the worker thread.
def submit_in_cell(pool, function, /, *args, **kwargs):
    owner = getattr(thread_cell, "token", None)
    finished = threading.Event()
    with pool_lock:
        if owner is active_token and owner is not None:
            pool_tasks.setdefault(owner, set()).add(finished)
    def run():
        previous_lineage = getattr(thread_cell, "lineage", ())
        thread_cell.token = owner
        thread_cell.task = finished
        try:
            return function(*args, **kwargs)
        finally:
            finished.set()
            del thread_cell.task
            thread_cell.lineage = previous_lineage
    try:
        future = submit_thread(pool, run)
    except BaseException:
        finished.set()
        raise
    add_done_callback(future, lambda _: finished.set())
    return future

concurrent.futures.ThreadPoolExecutor.submit = submit_in_cell

def callback_in_cell(future, callback):
    owner = getattr(thread_cell, "token", None)
    finished = threading.Event()
    with pool_lock:
        if owner is active_token and owner is not None:
            pool_tasks.setdefault(owner, set()).add(finished)
    def run(result):
        previous = getattr(thread_cell, "token", None)
        previous_task = getattr(thread_cell, "task", None)
        thread_cell.token = owner
        thread_cell.task = finished
        try:
            return callback(result)
        finally:
            finished.set()
            thread_cell.token = previous
            if previous_task is None:
                del thread_cell.task
            else:
                thread_cell.task = previous_task
    try:
        return add_done_callback(future, run)
    except BaseException:
        finished.set()
        raise

concurrent.futures.Future.add_done_callback = callback_in_cell

def send(value):
    value["cell"] = active_cell
    protocol.write(json.dumps(value, ensure_ascii=False, separators=(",", ":")) + "\n")
    protocol.flush()

def call(name, arguments):
    global next_id
    with call_lock:
        if active_token is None or getattr(thread_cell, "token", None) is not active_token:
            raise RuntimeError("tool calls are only available in the cell that started this thread")
        thread = threading.current_thread()
        task = getattr(thread_cell, "task", thread)
        lineage = getattr(thread_cell, "lineage", ())
        with pool_lock:
            tasks = pool_tasks.get(active_token, set())
            admitted = task in tasks or any(parent in tasks for parent in lineage)
        if closing and task not in tool_threads and not admitted:
            raise RuntimeError("tool calls are unavailable after the cell finishes")
        if name not in catalog:
            raise ValueError("unknown or unavailable tool: " + name)
        if thread is not threading.main_thread():
            tool_threads.add(task)
            tool_threads.update(lineage)
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
    with image_lock:
        if active_token is None or getattr(thread_cell, "token", None) is not active_token:
            raise RuntimeError("images can only be emitted in the cell that started this work")
        cell_images = images
    if not isinstance(image, dict):
        raise ValueError("image must have mime and base64 data strings")
    mime, data = image.get("mime"), image.get("data")
    if not isinstance(mime, str) or not isinstance(data, str):
        raise ValueError("image must have mime and base64 data strings")
    with image_lock:
        if sum(len(item["data"]) for item in cell_images) + len(data) > 12_000_000:
            raise ValueError("emitted images are too large")
        cell_images.append({"mime": mime, "data": data})

def finish_cell(output, error=None):
    global active_token, closing
    with call_lock:
        closing = True
    deadline = time.monotonic() + 1
    def done(thread):
        return thread.is_set() if isinstance(thread, threading.Event) else not thread.is_alive()
    while True:
        with pool_lock:
            tasks = list(pool_tasks.get(active_token, ()))
        threads = list(tool_threads) + tasks
        if all(done(thread) for thread in threads):
            break
        if call_lock.locked():
            deadline = time.monotonic() + 1
        if time.monotonic() >= deadline:
            break
        time.sleep(0.01)
    with call_lock:
        with pool_lock:
            threads = list(tool_threads) + list(pool_tasks.pop(active_token, ()))
        active_token = None
    if any(not done(thread) for thread in threads):
        send({"type": "error", "reset": True, "error": "background thread that called a tool did not finish; Python session state was lost. Join tool threads before returning", "output": output.render()})
    elif error is not None:
        send({"type": "error", "error": error, "output": output.render()})
    else:
        send({"type": "result", "output": output.render(), "images": images})

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

class CellOutput:
    def __init__(self, output, token):
        self.output = output
        self.token = token

    def write(self, value):
        if getattr(thread_cell, "token", None) is self.token:
            return self.output.write(value)
        return len(value)

    def flush(self):
        pass

    def __getattr__(self, name):
        return getattr(self.output, name)

scope = {"tools": Tools(), "text": text, "emit_image": emit_image, "ALL_TOOLS": []}
send({"type": "ready"})
for line in sys.stdin:
    output = CapturedOutput()
    try:
        request = json.loads(line)
        if request.get("type") != "exec":
            raise ValueError("expected an exec request")
        with image_lock:
            active_cell = request["cell"]
            active_token = object()
            images = []
        closing = False
        thread_cell.token = active_token
        tool_threads.clear()
        entries = request.get("tools", [])
        catalog = {entry["tool_name"]: entry["tool_name"] for entry in entries}
        catalog.update({entry["name"]: entry["tool_name"] for entry in entries})
        scope["ALL_TOOLS"] = entries
        captured = CellOutput(output, active_token)
        with contextlib.redirect_stdout(captured), contextlib.redirect_stderr(captured):
            try:
                exec(compile(request["source"], "<mew-python>", "exec"), scope)
            except BaseException as error:
                finish_cell(output, "".join(traceback.format_exception_only(type(error), error)).strip())
            else:
                finish_cell(output)
    except BaseException as error:
        finish_cell(output, "".join(traceback.format_exception_only(type(error), error)).strip())
