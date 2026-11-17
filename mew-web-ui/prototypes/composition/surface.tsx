import { createElement, useEffect, useRef, useState } from "react";
import { copyBgraFrame, queryHost, type CompositionBootstrap } from "./app";

export function BrowserSurface({ bootstrap }: { bootstrap: CompositionBootstrap }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const element = canvas.current;
    if (!element || bootstrap.engine !== "cef-osr") return;
    const context = element.getContext("2d");
    if (!context) { setError("Canvas rendering is unavailable."); return; }
    let disposed = false;
    let frame = 0;
    let etag = "";
    let pixels: ImageData | null = null;
    const abort = new AbortController();
    const fail = (error: unknown) => { if (!disposed) setError(error instanceof Error ? error.message : String(error)); };
    const send = (event: object) => { void queryHost({ type: "input", event }).catch(fail); };
    const modifiers = (event: MouseEvent | KeyboardEvent) => ({ shiftKey: event.shiftKey, ctrlKey: event.ctrlKey, altKey: event.altKey, metaKey: event.metaKey });
    const point = (event: MouseEvent) => { const rect = element.getBoundingClientRect(); return { x: event.clientX - rect.left, y: event.clientY - rect.top }; };
    const resize = new ResizeObserver(() => {
      const rect = element.getBoundingClientRect();
      if (rect.width > 0 && rect.height > 0) void queryHost({ type: "resize", width: Math.round(rect.width), height: Math.round(rect.height), deviceScaleFactor: window.devicePixelRatio }).catch(fail);
    });
    resize.observe(element);
    const paint = async () => {
      try {
        if (!element.closest("[inert]")) {
          const response = await fetch("/frame", { signal: abort.signal, cache: "no-store", headers: etag ? { "If-None-Match": etag } : {} });
          if (response.status !== 304 && response.status !== 204) {
            if (!response.ok) throw new Error(`Frame transport returned ${response.status}.`);
            const width = Number(response.headers.get("X-Frame-Width"));
            const height = Number(response.headers.get("X-Frame-Height"));
            if (![width, height].every((size) => Number.isInteger(size) && size > 0 && size <= 4096)) throw new Error("Invalid CEF frame dimensions.");
            const buffer = await response.arrayBuffer();
            if (disposed) return;
            if (!pixels || pixels.width !== width || pixels.height !== height) {
              element.width = width; element.height = height;
              pixels = context.createImageData(width, height);
            }
            copyBgraFrame(buffer, width, height, pixels.data);
            context.putImageData(pixels, 0, 0);
            etag = response.headers.get("ETag") ?? "";
          }
        }
      } catch (error) { fail(error); }
      if (!disposed) frame = requestAnimationFrame(() => { void paint(); });
    };
    const down = (event: PointerEvent) => { event.preventDefault(); element.focus(); element.setPointerCapture(event.pointerId); send({ kind: "pointer", phase: "down", ...point(event), button: event.button, buttons: event.buttons, clickCount: event.detail || 1, ...modifiers(event) }); };
    const up = (event: PointerEvent) => send({ kind: "pointer", phase: "up", ...point(event), button: event.button, buttons: event.buttons, clickCount: event.detail || 1, ...modifiers(event) });
    const move = (event: PointerEvent) => send({ kind: "pointer", phase: "move", ...point(event), button: event.buttons & 1 ? 0 : event.buttons & 2 ? 2 : undefined, buttons: event.buttons, ...modifiers(event) });
    const leave = (event: PointerEvent) => send({ kind: "pointer", phase: "leave", ...point(event), ...modifiers(event) });
    const wheel = (event: WheelEvent) => { event.preventDefault(); send({ kind: "wheel", ...point(event), deltaX: event.deltaX, deltaY: event.deltaY, ...modifiers(event) }); };
    const keydown = (event: KeyboardEvent) => {
      if (event.isComposing) return;
      event.preventDefault();
      send({ kind: "key", phase: "down", key: event.key, code: event.code, keyCode: event.keyCode, ...modifiers(event) });
      if (event.key.length === 1 && !event.metaKey && !event.ctrlKey && !event.altKey) send({ kind: "text", text: event.key });
    };
    const keyup = (event: KeyboardEvent) => { if (!event.isComposing) { event.preventDefault(); send({ kind: "key", phase: "up", key: event.key, code: event.code, keyCode: event.keyCode, ...modifiers(event) }); } };
    const focus = () => send({ kind: "focus", focused: true });
    const blur = () => send({ kind: "focus", focused: false });
    const composition = (event: CompositionEvent) => { if (event.data) send({ kind: "text", text: event.data }); };
    element.addEventListener("pointerdown", down); element.addEventListener("pointerup", up);
    element.addEventListener("pointermove", move); element.addEventListener("pointerleave", leave);
    element.addEventListener("wheel", wheel, { passive: false });
    element.addEventListener("keydown", keydown); element.addEventListener("keyup", keyup);
    element.addEventListener("focus", focus); element.addEventListener("blur", blur);
    element.addEventListener("compositionend", composition);
    void paint();
    return () => {
      disposed = true; abort.abort(); cancelAnimationFrame(frame); resize.disconnect();
      element.removeEventListener("pointerdown", down); element.removeEventListener("pointerup", up);
      element.removeEventListener("pointermove", move); element.removeEventListener("pointerleave", leave);
      element.removeEventListener("wheel", wheel); element.removeEventListener("keydown", keydown); element.removeEventListener("keyup", keyup);
      element.removeEventListener("focus", focus); element.removeEventListener("blur", blur); element.removeEventListener("compositionend", composition);
    };
  }, [bootstrap.engine]);
  return <>{error && <div role="alert" className="composition-error">{error}</div>}{bootstrap.engine === "cef-osr"
    ? <canvas ref={canvas} data-browser-surface tabIndex={0} aria-label="CEF browser canvas" />
    : createElement("webview", { "data-browser-surface": "", src: bootstrap.fixtureUrl, webpreferences: "contextIsolation=yes,sandbox=yes,nodeIntegration=no", "aria-label": "Electron browser page", style: { display: "flex", width: "100%", height: "100%" } })}</>;
}
