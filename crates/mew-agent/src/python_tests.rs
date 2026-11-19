use serde_json::{json, Value};
use std::path::PathBuf;

use crate::python::{PythonOptions, PythonWorker};

async fn worker(isolated: bool) -> PythonWorker {
    PythonWorker::start(&PythonOptions {
        executable: PathBuf::from("python3"),
        isolated,
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn python_start_reports_when_the_interpreter_exits_immediately() {
    let result = PythonWorker::start(&PythonOptions {
        executable: PathBuf::from("/usr/bin/false"),
        isolated: false,
    })
    .await;
    assert!(result.is_err());
}

async fn run(worker: &mut PythonWorker, source: &str) -> Value {
    worker
        .send(&json!({"type":"exec", "cell":"test-cell", "source":source, "tools":[]}))
        .await
        .unwrap();
    let result = worker.receive().await.unwrap();
    assert_eq!(result["cell"], "test-cell");
    result
}

#[tokio::test]
async fn python_variables_survive_calls_and_exceptions_but_not_other_sessions() {
    let mut first = worker(false).await;
    let mut second = worker(false).await;
    assert_eq!(
        run(&mut first, "saved = 41\ntext(saved)").await["output"],
        "41\n"
    );
    assert_eq!(
        run(&mut first, "raise ValueError('bad')").await["type"],
        "error"
    );
    assert_eq!(run(&mut first, "text(saved + 1)").await["output"], "42\n");
    assert_eq!(
        run(&mut second, "text('saved' in globals())").await["output"],
        "false\n"
    );
}

#[tokio::test]
async fn python_tools_send_arguments_and_receive_the_host_result() {
    let mut worker = worker(false).await;
    worker
        .send(&json!({
            "type":"exec",
            "cell":"tool-cell",
            "source":"result = tools.read({'path':'file.txt'})\ntext(result['output'])",
            "tools":[{"name":"read","tool_name":"read"}]
        }))
        .await
        .unwrap();
    let call = worker.receive().await.unwrap();
    assert_eq!(call["type"], "tool");
    assert_eq!(call["cell"], "tool-cell");
    assert_eq!(call["name"], "read");
    assert_eq!(call["input"], json!({"path":"file.txt"}));
    worker.send(&json!({"type":"tool_result","cell":"tool-cell","id":call["id"],"result":{"output":"hello","error":""}})).await.unwrap();
    let result = worker.receive().await.unwrap();
    assert_eq!(result["type"], "result");
    assert_eq!(result["cell"], "tool-cell");
    assert_eq!(result["output"], "hello\n");
}

#[tokio::test]
async fn python_output_is_bounded_and_later_calls_still_work() {
    let mut worker = worker(false).await;
    let result = run(&mut worker, "print('x' * 1000000)").await;
    let output = result["output"].as_str().unwrap();
    assert!(output.len() < 150_000);
    assert!(output.contains("[output truncated]"));
    assert_eq!(
        run(&mut worker, "text('still running')").await["output"],
        "still running\n"
    );
}

#[tokio::test]
async fn python_tool_calls_from_threads_receive_their_own_results() {
    let mut worker = worker(false).await;
    worker.send(&json!({
        "type":"exec", "cell":"threads",
        "source":"from concurrent.futures import ThreadPoolExecutor\ndef run(x): return tools.echo({'input':x})['output']\nwith ThreadPoolExecutor(max_workers=2) as pool:\n text(sorted(pool.map(run, ['one','two'])))",
        "tools":[{"name":"echo","tool_name":"echo"}]
    })).await.unwrap();
    for _ in 0..2 {
        let call = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(call["type"], "tool");
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        let response = json!({"type":"tool_result","cell":"threads","id":call["id"],"result":{"output":call["input"]["input"],"error":""}});
        worker.send(&response).await.unwrap();
    }
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result["type"], "result");
    assert_eq!(result["output"], "[\"one\",\"two\"]\n");
}

#[tokio::test]
async fn python_can_reuse_a_thread_pool_for_tool_calls_across_cells() {
    let mut worker = worker(false).await;
    let tools = json!([{"name":"echo","tool_name":"echo"}]);
    for (cell, source, expected) in [
        ("first", "from concurrent.futures import ThreadPoolExecutor\npool=ThreadPoolExecutor(max_workers=1)\ntext(pool.submit(tools.echo, {'input':'one'}).result()['output'])", "one\n"),
        ("second", "text(pool.submit(tools.echo, {'input':'two'}).result()['output'])\npool.shutdown()", "two\n"),
    ] {
        worker.send(&json!({"type":"exec","cell":cell,"tools":tools,"source":source})).await.unwrap();
        let call = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive()).await.unwrap().unwrap();
        assert_eq!(call["type"], "tool");
        assert_eq!(call["cell"], cell);
        worker.send(&json!({"type":"tool_result","cell":cell,"id":call["id"],"result":{"output":expected.trim(),"error":""}})).await.unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive()).await.unwrap().unwrap();
        assert_eq!(result["type"], "result", "{result}");
        assert_eq!(result["output"], expected);
    }
}

#[tokio::test]
async fn python_waits_for_nested_pool_tasks_that_call_tools() {
    let mut worker = worker(false).await;
    worker.send(&json!({"type":"exec","cell":"first","tools":[{"name":"echo","tool_name":"echo"}],"source":"from concurrent.futures import ThreadPoolExecutor\nimport threading\npool=ThreadPoolExecutor(max_workers=2)\nstarted=threading.Event()\nclass Arguments(dict):\n def items(self):\n  started.set()\n  return super().items()\ndef child():\n return tools.echo({'input':'two'})\ndef parent():\n tools.echo(Arguments(input='one'))\n return pool.submit(child).result()\nfuture=pool.submit(parent)\nassert started.wait(2)\ntext('submitted')"})).await.unwrap();
    for expected in ["one", "two"] {
        let call = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(call["type"], "tool", "{call}");
        assert_eq!(call["input"]["input"], expected);
        worker.send(&json!({"type":"tool_result","cell":"first","id":call["id"],"result":{"output":expected,"error":""}})).await.unwrap();
    }
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result["type"], "result", "{result}");
    assert_eq!(result["output"], "submitted\n");
    assert_eq!(
        run(
            &mut worker,
            "text(future.result()['output'])\npool.shutdown()"
        )
        .await["output"],
        "two\n"
    );
}

#[tokio::test]
async fn python_resets_when_queued_pool_work_outlives_its_cell() {
    let mut worker = worker(false).await;
    let tools = json!([{"name":"echo","tool_name":"echo"}]);
    worker.send(&json!({"type":"exec","cell":"first","tools":tools,"source":"from concurrent.futures import ThreadPoolExecutor\nimport threading\npool=ThreadPoolExecutor(max_workers=1)\nstarted=threading.Event()\nrelease=threading.Event()\nclass Arguments(dict):\n def items(self):\n  started.set()\n  return super().items()\ndef later():\n release.wait()\n return tools.echo({'input':'late'})\nfirst=pool.submit(tools.echo, Arguments(input='one'))\nassert started.wait(2)\nlate=pool.submit(later)\ntext('submitted')"})).await.unwrap();
    let call = worker.receive().await.unwrap();
    assert_eq!(call["type"], "tool");
    worker.send(&json!({"type":"tool_result","cell":"first","id":call["id"],"result":{"output":"one","error":""}})).await.unwrap();
    let first = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(first["type"], "error", "{first}");
    assert_eq!(first["reset"], true);
    assert!(
        first["error"].as_str().unwrap().contains("background"),
        "{first}"
    );
}

#[tokio::test]
async fn python_does_not_capture_old_thread_output_in_a_later_cell() {
    let mut worker = worker(false).await;
    let first = run(&mut worker, "import threading\nrelease=threading.Event()\ndone=threading.Event()\ndef later():\n release.wait()\n print('old output')\n done.set()\nthreading.Thread(target=later,daemon=True).start()\ntext('first')").await;
    assert_eq!(first["output"], "first\n");
    let second = run(
        &mut worker,
        "release.set()\nassert done.wait(2)\ntext('second')",
    )
    .await;
    assert_eq!(second["output"], "second\n");
}

#[tokio::test]
async fn python_rejects_images_from_a_previous_cell() {
    let mut worker = worker(false).await;
    let first = run(&mut worker, "import threading\nrelease=threading.Event()\ndone=threading.Event()\ndef later():\n release.wait()\n try:\n  emit_image({'mime':'image/png','data':'YWJj'})\n except RuntimeError:\n  pass\n finally:\n  done.set()\nthreading.Thread(target=later,daemon=True).start()").await;
    assert_eq!(first["type"], "result");
    let second = run(
        &mut worker,
        "release.set()\nassert done.wait(2)\ntext('second')",
    )
    .await;
    assert_eq!(second["output"], "second\n");
    assert_eq!(second["images"], json!([]));
}

#[tokio::test]
async fn python_does_not_emit_a_previous_cells_inflight_image() {
    let mut worker = worker(false).await;
    let first = run(&mut worker, "import threading\nstarted=threading.Event()\nrelease=threading.Event()\ndone=threading.Event()\nclass Image(dict):\n def get(self, key, *args):\n  if key == 'mime':\n   started.set()\n   release.wait()\n  return super().get(key, *args)\ndef later():\n try:\n  emit_image(Image(mime='image/png',data='YWJj'))\n finally:\n  done.set()\nthreading.Thread(target=later,daemon=True).start()\nassert started.wait(2)").await;
    assert_eq!(first["type"], "result");
    let second = run(
        &mut worker,
        "release.set()\nassert done.wait(2)\ntext('second')",
    )
    .await;
    assert_eq!(second["output"], "second\n");
    assert_eq!(second["images"], json!([]));
}

#[tokio::test]
async fn python_captures_future_callback_output_in_its_originating_cell() {
    let mut worker = worker(false).await;
    worker.send(&json!({"type":"exec","cell":"first","tools":[{"name":"echo","tool_name":"echo"}],"source":"from concurrent.futures import ThreadPoolExecutor\nimport threading, time\npool=ThreadPoolExecutor(max_workers=1)\nstarted=threading.Event()\nclass Arguments(dict):\n def items(self):\n  started.set()\n  return super().items()\nf=pool.submit(tools.echo, Arguments(input='hi'))\nassert started.wait(2)\nf.add_done_callback(lambda _: (time.sleep(.2), text('callback output')))"})).await.unwrap();
    let call = worker.receive().await.unwrap();
    assert_eq!(call["type"], "tool");
    worker.send(&json!({"type":"tool_result","cell":"first","id":call["id"],"result":{"output":"hi","error":""}})).await.unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result["type"], "result", "{result}");
    assert_eq!(result["output"], "callback output\n");
}

#[tokio::test]
async fn python_background_threads_cannot_call_tools_in_a_later_cell() {
    let mut worker = worker(false).await;
    let tools = json!([{"name":"echo","tool_name":"echo"}]);
    worker.send(&json!({
        "type":"exec", "cell":"first", "tools":tools,
        "source":"import threading\nrelease = threading.Event()\ndone = threading.Event()\nresult = []\ndef later():\n release.wait()\n try:\n  tools.echo({'from':'first'})\n  result.append('called')\n except RuntimeError:\n  result.append('rejected')\n finally:\n  done.set()\nthreading.Thread(target=later, daemon=True).start()\nsaved = 42\ntext('ready')"
    })).await.unwrap();
    let first = worker.receive().await.unwrap();
    assert_eq!(first["type"], "result");
    assert_eq!(first["cell"], "first");
    worker.send(&json!({
        "type":"exec", "cell":"second", "tools":tools,
        "source":"release.set()\nassert done.wait(2), 'background thread did not finish'\ntext([result, saved])"
    })).await.unwrap();
    let second = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        second["type"], "result",
        "stale thread sent a tool call: {second}"
    );
    assert_eq!(second["cell"], "second");
    assert_eq!(second["output"], "[[\"rejected\"],42]\n");
}

#[tokio::test]
async fn python_waits_for_a_background_tool_call_before_finishing_its_cell() {
    let mut worker = worker(false).await;
    worker.send(&json!({
        "type":"exec", "cell":"first",
        "tools":[{"name":"echo","tool_name":"echo"}],
        "source":"import threading\nstarted = threading.Event()\nclass Arguments(dict):\n def items(self):\n  started.set()\n  return super().items()\ndef later():\n r = tools.echo(Arguments(value='hello'))\n r = tools.echo({'value':r['output']})\n text('received ' + r['output'])\nthreading.Thread(target=later, daemon=True).start()\nassert started.wait(2), 'background call did not start'\nsaved = 42\ntext('finished')"
    })).await.unwrap();
    let call = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(call["type"], "tool");
    assert_eq!(call["cell"], "first");
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), worker.receive())
            .await
            .is_err(),
        "cell finished before its tool call"
    );
    worker.send(&json!({"type":"tool_result","cell":"first","id":call["id"],"result":{"output":"ok","error":""}})).await.unwrap();
    let second_call = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(second_call["type"], "tool", "{second_call}");
    assert_eq!(second_call["input"]["value"], "ok");
    worker.send(&json!({"type":"tool_result","cell":"first","id":second_call["id"],"result":{"output":"ok","error":""}})).await.unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result["type"], "result");
    assert_eq!(result["output"], "finished\nreceived ok\n");
    assert_eq!(run(&mut worker, "text(saved)").await["output"], "42\n");
}

#[tokio::test]
async fn python_reports_unfinished_tool_thread_and_requires_a_reset() {
    let mut worker = worker(false).await;
    worker.send(&json!({
        "type":"exec", "cell":"first", "tools":[{"name":"echo","tool_name":"echo"}],
        "source":"import threading\nstarted=threading.Event()\nrelease=threading.Event()\nclass Arguments(dict):\n def items(self):\n  started.set()\n  return super().items()\ndef later():\n tools.echo(Arguments(value='hello'))\n release.wait()\nthreading.Thread(target=later, daemon=True).start()\nassert started.wait(2)"
    })).await.unwrap();
    let call = worker.receive().await.unwrap();
    assert_eq!(call["type"], "tool");
    worker.send(&json!({"type":"tool_result","cell":"first","id":call["id"],"result":{"output":"ok","error":""}})).await.unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(3), worker.receive())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result["type"], "error");
    assert_eq!(result["reset"], true);
    assert!(result["error"].as_str().unwrap().contains("background"));
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn isolated_python_cannot_open_host_files_connect_to_localhost_or_spawn() {
    let secret = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(secret.path(), "untouched").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let mut worker = worker(true).await;
    let code = format!(
        "import os, socket, subprocess\ntry:\n open({0:?}).read()\n file_ok=True\nexcept OSError:\n file_ok=False\ntry:\n os.symlink({0:?}, os.path.join(os.environ['HOME'], 'link'))\n open(os.path.join(os.environ['HOME'], 'link')).read()\n link_ok=True\nexcept OSError:\n link_ok=False\ntry:\n open({0:?}, 'w').write('changed')\nexcept OSError:\n pass\ntry:\n s=socket.socket()\n s.settimeout(1)\n s.connect(('127.0.0.1', {1}))\n net_ok=True\nexcept OSError:\n net_ok=False\ntry:\n subprocess.run(['/usr/bin/true'], check=True)\n spawn_ok=True\nexcept OSError:\n spawn_ok=False\ntext([file_ok,link_ok,net_ok,spawn_ok])",
        secret.path().to_string_lossy(), address.port()
    );
    assert_eq!(
        run(&mut worker, &code).await["output"],
        "[false,false,false,false]\n"
    );
    assert_eq!(std::fs::read_to_string(secret.path()).unwrap(), "untouched");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[tokio::test]
async fn isolated_python_uses_the_selected_virtual_environment() {
    let dir = tempfile::tempdir().unwrap();
    let status = tokio::process::Command::new("python3")
        .args(["-I", "-m", "venv"])
        .arg(dir.path())
        .status()
        .await
        .unwrap();
    assert!(status.success());
    let output = tokio::process::Command::new(dir.path().join("bin/python3"))
        .args([
            "-I",
            "-c",
            "import sysconfig; print(sysconfig.get_path('purelib'))",
        ])
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    let packages = PathBuf::from(String::from_utf8(output.stdout).unwrap().trim());
    std::fs::write(packages.join("mew_isolation_test.py"), "value = 42\n").unwrap();
    let mut worker = PythonWorker::start(&PythonOptions {
        executable: dir.path().join("bin/python3"),
        isolated: true,
    })
    .await
    .unwrap();
    let result = run(&mut worker, "import sys; text(sys.prefix)").await;
    assert_eq!(
        std::fs::canonicalize(result["output"].as_str().unwrap().trim()).unwrap(),
        std::fs::canonicalize(dir.path()).unwrap()
    );
    assert_eq!(
        run(
            &mut worker,
            "import mew_isolation_test; text(mew_isolation_test.value)"
        )
        .await["output"],
        "42\n"
    );
    let secret = dir.path().join("private.txt");
    std::fs::write(&secret, "private").unwrap();
    let secret = std::fs::canonicalize(secret).unwrap();
    let source = format!(
        "try:\n open({:?}).read()\n text('readable')\nexcept OSError:\n text('denied')",
        secret.to_string_lossy()
    );
    assert_eq!(run(&mut worker, &source).await["output"], "denied\n");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn dropping_isolated_python_terminates_its_background_processes() {
    use std::io::Read;
    let mut worker = worker(true).await;
    let code = "import os, time\nr, w = os.pipe()\npid = os.fork()\nif pid == 0:\n os.close(r)\n f = open(os.path.join(os.environ['HOME'], 'late'), 'w')\n os.write(w, b'x')\n time.sleep(1.5)\n f.write('escaped')\n f.flush()\n os._exit(0)\nos.close(w)\nos.read(r, 1)\ntext('ready')";
    assert_eq!(run(&mut worker, code).await["output"], "ready\n");
    let mut file = std::fs::File::open(worker.scratch_path().join("late")).unwrap();
    drop(worker);
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();
    assert!(contents.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn dropping_unrestricted_python_terminates_its_background_processes() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("late");
    let mut worker = worker(false).await;
    let code = format!("import os, time\nr, w = os.pipe()\npid = os.fork()\nif pid == 0:\n os.close(r)\n f = open({:?}, 'w')\n os.write(w, b'x')\n time.sleep(1.5)\n f.write('escaped')\n f.flush()\n os._exit(0)\nos.close(w)\nos.read(r, 1)\ntext('ready')", marker.to_string_lossy());
    assert_eq!(run(&mut worker, &code).await["output"], "ready\n");
    drop(worker);
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    assert_eq!(std::fs::read_to_string(marker).unwrap(), "");
}
