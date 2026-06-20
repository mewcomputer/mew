import { useEffect, useState } from "react";

const STATIC_LINES = [
  { text: '~/code/myapp $ mew "convert the auth module to argon2"', kind: "prompt" },
  { text: "", kind: "empty" },
  { text: "thinking...", kind: "thinking" },
  { text: "", kind: "empty" },
  { text: "the current bcrypt implementation lives in src/auth/password.rs.", kind: "text" },
  { text: "let me look at the hashing setup first.", kind: "text" },
  { text: "", kind: "empty" },
  { text: "○ grep -n \"bcrypt\" src/auth/", kind: "tool" },
  { text: "  src/auth/password.rs:4:use bcrypt::{hash, verify, BcryptError};", kind: "output" },
  { text: "  src/auth/password.rs:18:    hash(plain, DEFAULT_COST)?", kind: "output" },
  { text: "  src/auth/password.rs:24:    verify(plain, hashed)?", kind: "output" },
  { text: "", kind: "empty" },
  { text: "allow write src/auth/password.rs? [y/n] y", kind: "permission" },
  { text: "", kind: "empty" },
  { text: "○ write src/auth/password.rs", kind: "tool" },
  { text: "  - use bcrypt::{hash, verify, BcryptError};", kind: "diff-del" },
  { text: "  + use argon2::{Argon2, PasswordHash, PasswordVerifier, PasswordHasher};", kind: "diff-add" },
  { text: "", kind: "empty" },
  { text: "the module uses argon2 now. the verify call returns Result<(), _>", kind: "text" },
  { text: "so the error handling stays the same — i just swapped the hash/verify", kind: "text" },
] as const;

const LINE_COLORS: Record<string, string> = {
  prompt: "var(--color-accent)",
  thinking: "var(--color-text-dim)",
  text: "var(--color-text)",
  tool: "var(--color-accent)",
  output: "var(--color-text-dim)",
  permission: "var(--color-accent-dim)",
  "diff-del": "#c06060",
  "diff-add": "#60a060",
  empty: "transparent",
};

const LAST_LINE = "functions. tests pass.";

export default function TerminalDemo() {
  const [typed, setTyped] = useState("");
  const [cursorVisible, setCursorVisible] = useState(true);

  useEffect(() => {
    let i = 0;
    const typeTimer = setInterval(() => {
      if (i <= LAST_LINE.length) {
        setTyped(LAST_LINE.slice(0, i));
        i++;
      } else {
        clearInterval(typeTimer);
      }
    }, 50);

    return () => clearInterval(typeTimer);
  }, []);

  useEffect(() => {
    const blink = setInterval(() => setCursorVisible((v) => !v), 530);
    return () => clearInterval(blink);
  }, []);

  const cursor = (
    <span
      className="inline-block w-[0.6em] h-[1.1em] align-text-bottom ml-px"
      style={{
        backgroundColor: cursorVisible ? "var(--color-accent)" : "transparent",
        transition: "background-color 100ms",
      }}
    />
  );

  return (
    <div
      className="terminal mx-auto my-10 max-w-2xl rounded-lg border border-[var(--color-border)] px-6 py-5 leading-[1.6] tracking-[0.01em] shadow-lg"
      style={{ backgroundColor: "var(--color-bg-terminal)" }}
    >
      {STATIC_LINES.map((line, i) =>
        line.text === "" ? (
          <div key={i} className="h-[1.6em]" />
        ) : (
          <div
            key={i}
            className="terminal-line whitespace-pre-wrap"
            style={{ color: LINE_COLORS[line.kind] }}
          >
            {line.text}
          </div>
        )
      )}
      <div className="terminal-line terminal-line-last whitespace-pre-wrap">
        {typed}
        {cursor}
      </div>
    </div>
  );
}
