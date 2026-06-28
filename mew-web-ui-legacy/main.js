// Minimal mew web client — vanilla JS, no framework.
// Speaks the wire protocol directly. A typed TypeScript library
// (`mew-web-client`) will replace this; see CLAUDE.md architecture.
//
// Wire format reminder (see crates/mew-protocol/src/lib.rs):
//   - JSON over WebSocket text frames
//   - Each message has a `type` tag (e.g. `"type":"Prompt"`, `"type":"Provider"`)
//
//   ClientMessage variants we send:
//     NewSession { cwd: string|null }
//     Prompt     { text, attachments }
//     Cancel
//     PermissionResponse { request_id, decision }
//     SlashCommand { command }
//
//   ServerMessage variants we handle:
//     SessionReady      { session_id }
//     Provider          { event: ProviderEventWire }
//     ToolStart/End     { call_id, success? }
//     PartUpdated       { part_id, part }
//     Error/ErrorEvent  { message }
//     PermissionRequest { request_id, tool_name, input }
//     WorkspacePermissionRequest { request_id, path }
//     SlashResult       { text }

const messagesEl = document.getElementById("messages");
const inputEl = document.getElementById("input");
const sendBtn = document.getElementById("send");
const statusEl = document.getElementById("status");

let ws = null;
let sessionId = null;
let streamingText = null; // current assistant TextPart being streamed
let nextRequestId = 1;

function setStatus(text, cls = "") {
  statusEl.textContent = text;
  statusEl.className = "status " + cls;
}

function appendMessage(role, content, opts = {}) {
  const div = document.createElement("div");
  div.className = `msg ${role}`;
  if (opts.label) {
    const label = document.createElement("div");
    label.className = "label";
    label.textContent = opts.label;
    div.appendChild(label);
  }
  if (content) {
    const body = document.createElement("div");
    body.className = "body";
    body.textContent = content;
    div.appendChild(body);
  }
  messagesEl.appendChild(div);
  messagesEl.scrollTop = messagesEl.scrollHeight;
  return div;
}

function appendPermissionRequest({ request_id, tool_name, input }) {
  const div = document.createElement("div");
  div.className = "perm";
  const label = document.createElement("div");
  label.className = "label";
  label.textContent = `Permission requested: ${tool_name}`;
  div.appendChild(label);
  const body = document.createElement("div");
  body.textContent = JSON.stringify(input, null, 2);
  div.appendChild(body);
  const actions = document.createElement("div");
  actions.className = "actions";
  for (const [cls, decision, label] of [
    ["allow", "allow_once", "Allow once"],
    ["allow", "allow_session", "Allow session"],
    ["deny", "deny", "Deny"],
  ]) {
    const btn = document.createElement("button");
    btn.className = cls;
    btn.textContent = label;
    btn.onclick = () => {
      sendClient({ type: "PermissionResponse", request_id, decision });
      div.remove();
    };
    actions.appendChild(btn);
  }
  div.appendChild(actions);
  messagesEl.appendChild(div);
  messagesEl.scrollTop = messagesEl.scrollHeight;
}

function sendClient(msg) {
  if (!ws || ws.readyState !== WebSocket.OPEN) {
    appendMessage("error", "Not connected");
    return;
  }
  ws.send(JSON.stringify(msg));
}

function handleServerMessage(msg) {
  switch (msg.type) {
    case "SessionReady":
      sessionId = msg.session_id;
      setStatus(`connected · session ${sessionId.slice(0, 8)}…`, "connected");
      inputEl.disabled = false;
      sendBtn.disabled = false;
      inputEl.focus();
      break;

    case "Error":
    case "ErrorEvent":
      appendMessage("error", msg.message ?? "(unknown error)");
      break;

    case "Provider": {
      const ev = msg.event;
      switch (ev.type) {
        case "PartStart": {
          const part = ev.part;
          if (part.type === "Text") {
            streamingText = appendMessage("assistant", "");
            streamingText._text = "";
          } else if (part.type === "ToolCall") {
            appendMessage(
              "tool",
              `${part.tool_name}(${JSON.stringify(part.state.input ?? {})})`,
              { label: "tool call" },
            );
          }
          break;
        }
        case "PartDelta": {
          if (streamingText && ev.field === "text") {
            streamingText._text = (streamingText._text ?? "") + ev.delta;
            const body = streamingText.querySelector(".body");
            if (body) body.textContent = streamingText._text;
            messagesEl.scrollTop = messagesEl.scrollHeight;
          }
          break;
        }
        case "PartEnd":
          if (streamingText) {
            streamingText = null;
          }
          break;
        case "MessageEnd":
          // End of assistant message; nothing extra to render.
          break;
        case "RetryWait":
          setStatus(`retrying (${ev.attempt}/${ev.max_attempts})…`);
          break;
      }
      break;
    }

    case "ToolStart":
    case "ToolEnd":
    case "PartUpdated":
      // Already rendered as part of the streaming message; safe to ignore.
      break;

    case "PermissionRequest":
    case "SubagentPermissionRequest":
      appendPermissionRequest(msg);
      break;

    case "WorkspacePermissionRequest":
      appendPermissionRequest({
        request_id: msg.request_id,
        tool_name: "bash",
        input: { path: msg.path },
      });
      break;

    case "AskUserRequest":
      // For now, just log it; a richer UI would render the question cards.
      console.log("AskUserRequest", msg);
      break;

    case "SubagentStart":
      appendMessage("tool", `${msg.name} (${msg.display_name ?? "—"}) started`, {
        label: "subagent",
      });
      break;

    case "SubagentStatus":
      // Could be appended as a small status line; skipping for now.
      break;

    case "SubagentEnd":
      appendMessage("tool", `${msg.child_session_id} → ${msg.outcome.type}`, {
        label: "subagent done",
      });
      break;

    case "TodosUpdated":
      appendMessage("tool", `${msg.todos.length} todo(s)`, { label: "todos" });
      break;

    case "PersonaSwitchRequested":
      setStatus(`persona → ${msg.name}`);
      break;

    case "JobUpdate":
      // Could render bg-job list; skipping for now.
      break;

    case "SlashResult":
      appendMessage("tool", msg.text, { label: "slash" });
      break;
  }
}

function connect() {
  const url = `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/`;
  ws = new WebSocket(url);
  setStatus("connecting…");

  ws.addEventListener("open", () => {
    sendClient({ type: "NewSession", cwd: null });
  });

  ws.addEventListener("message", (ev) => {
    let msg;
    try {
      msg = JSON.parse(ev.data);
    } catch (e) {
      appendMessage("error", `bad json from daemon: ${ev.data}`);
      return;
    }
    handleServerMessage(msg);
  });

  ws.addEventListener("close", () => {
    setStatus("disconnected — retrying in 2s", "error");
    inputEl.disabled = true;
    sendBtn.disabled = true;
    setTimeout(connect, 2000);
  });

  ws.addEventListener("error", () => {
    setStatus("connection error", "error");
  });
}

function submitPrompt() {
  const text = inputEl.value.trim();
  if (!text) return;
  inputEl.value = "";
  appendMessage("user", text);
  sendClient({ type: "Prompt", text, attachments: [] });
}

sendBtn.addEventListener("click", submitPrompt);
inputEl.addEventListener("keydown", (e) => {
  if (e.key === "Enter" && !e.shiftKey) {
    e.preventDefault();
    submitPrompt();
  }
});

connect();