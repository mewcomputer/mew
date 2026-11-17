import { useState, type ReactNode } from "react";
import { Button } from "../../src/components/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "../../src/components/ui/popover";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "../../src/components/ui/dropdown-menu";

export interface CompositionBootstrap {
  engine: "cef-osr" | "electron-webview";
  fixtureUrl: string;
  versions: Record<string, string>;
}

export function queryHost<T = unknown>(request: object): Promise<T> {
  return new Promise((resolve, reject) => {
    const bridge = (window as Window & { mewQuery?: (query: { request: string; onSuccess: (response: string) => void; onFailure: (code: number, message: string) => void }) => void }).mewQuery;
    if (!bridge) return reject(new Error("The composition host is unavailable."));
    bridge({ request: JSON.stringify(request), onSuccess: (response) => {
      try { resolve(JSON.parse(response) as T); } catch { reject(new Error("Invalid host response.")); }
    }, onFailure: (_code, message) => reject(new Error(message)) });
  });
}

export function copyBgraFrame(buffer: ArrayBuffer, width: number, height: number, target: Uint8ClampedArray) {
  if (![width, height].every((size) => Number.isInteger(size) && size > 0 && size <= 4096)
    || width * height * 4 !== buffer.byteLength || target.byteLength !== buffer.byteLength) {
    throw new Error("Invalid CEF frame dimensions or pixel buffer.");
  }
  const pixels = new Uint8Array(buffer);
  for (let index = 0; index < pixels.length; index += 4) {
    target[index] = pixels[index + 2]!;
    target[index + 1] = pixels[index + 1]!;
    target[index + 2] = pixels[index]!;
    target[index + 3] = pixels[index + 3]!;
  }
}

export function CompositionApp({ bootstrap, surface }: { bootstrap: CompositionBootstrap; surface: ReactNode }) {
  const [open, setOpen] = useState(true);
  const [wide, setWide] = useState(false);
  const [faded, setFaded] = useState(false);
  const [light, setLight] = useState(false);
  const [metrics, setMetrics] = useState<string | null>(null);
  const [action, setAction] = useState("Ready");
  const resize = () => setWide((value) => !value);
  const fade = () => setFaded((value) => !value);
  return <div className="composition-app">
    <header className="composition-header">
      <div><strong>mew / composition lab</strong><span>{bootstrap.engine === "cef-osr" ? "CEF · OSR canvas" : "Electron · DOM webview"}</span></div>
      <div className="flex items-center gap-2">
        <Button variant="ghost" size="sm" onClick={() => {
          const next = !light; setLight(next);
          document.documentElement.dataset.theme = next ? "codex-light" : "codex-dark";
          document.documentElement.classList.toggle("dark", !next);
        }}>{light ? "Dark theme" : "Light theme"}</Button>
        <Button variant="ghost" size="sm" onClick={resize}>Resize pane</Button>
        <Button variant="secondary" size="sm" onClick={() => setOpen((value) => !value)}>{open ? "Close browser pane" : "Open browser pane"}</Button>
      </div>
    </header>
    <main className="composition-split" style={{ gridTemplateColumns: open ? wide ? "minmax(0, .6fr) minmax(0, 1.4fr)" : "minmax(0, 1fr) minmax(0, 1fr)" : "minmax(0, 1fr) minmax(0, 0fr)" }}>
      <section aria-label="Chat" className="composition-chat">
        <div className="composition-transcript">
          <p className="composition-user-message">Can a live browser share the same composition as the chat?</p>
          <p>Open the overlay note or page menu. The live page should stay visible beneath them, including through the translucent note.</p>
          <p>Try resizing from inside the note, then Escape, browser editing, page and nested scrolling, and closing and reopening the pane.</p>
          <p className="text-muted-foreground">The same controls and fixture run in both hosts. The CEF canvas path forwards real input to its offscreen browser.</p>
          <details className="mt-8 text-xs text-muted-foreground"><summary>Runtime versions</summary><pre>{JSON.stringify(bootstrap.versions, null, 2)}</pre></details>
          <Button className="mt-3" variant="outline" size="sm" onClick={() => {
            void queryHost({ type: "metrics" }).then((value) => setMetrics(JSON.stringify(value, null, 2))).catch((error: unknown) => setMetrics(String(error)));
          }}>Sample host metrics</Button>
          {metrics && <pre className="composition-metrics">{metrics}</pre>}
          <output className="mt-4 block text-xs text-muted-foreground" aria-live="polite">{action}</output>
        </div>
        <div className="composition-composer"><textarea aria-label="Chat draft" placeholder="Keep a draft here while testing the browser…" /><div className="text-xs text-muted-foreground">Draft stays in the app</div></div>
      </section>
      <section aria-label="Browser pane" aria-hidden={!open} inert={!open} className="composition-workbench">
        <div className="composition-toolbar">
          <strong>Browser fixture</strong>
          <Popover>
            <PopoverTrigger asChild><Button variant="secondary" size="sm">Overlay note</Button></PopoverTrigger>
            <PopoverContent align="end" className="composition-overlay w-80 space-y-3">
              <p className="text-sm font-medium">A real app overlay</p>
              <p className="text-xs text-muted-foreground">The live browser remains behind this translucent surface.</p>
              <input aria-label="Overlay note" placeholder="Type here, then press Escape" className="composition-input" />
              <div className="flex gap-2"><Button size="sm" variant="secondary" onClick={resize}>Resize underneath</Button><Button size="sm" variant="secondary" onClick={fade}>Fade underneath</Button></div>
            </PopoverContent>
          </Popover>
          <DropdownMenu>
            <DropdownMenuTrigger asChild><Button variant="ghost" size="sm">Page menu</Button></DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-64">
              <DropdownMenuItem onSelect={() => setAction("Page menu action selected")}>Select an app action</DropdownMenuItem>
              <DropdownMenuItem onSelect={(event) => { event.preventDefault(); resize(); }}>Resize while menu stays open</DropdownMenuItem>
              {Array.from({ length: 12 }, (_, index) => <DropdownMenuItem key={index} onSelect={() => setAction(`Menu item ${index + 1} selected`)}>Menu item {index + 1}</DropdownMenuItem>)}
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
        <div className="composition-workbench-scroll" aria-label="Workbench scroll area">
          <div className="composition-stage" style={{ opacity: faded ? .45 : 1 }}>{surface}</div>
          <div className="composition-workbench-note"><p className="mb-3 text-sm">Scroll the surrounding workbench as well as the page.</p><textarea className="composition-input" aria-label="Workbench note" placeholder="Another app field for focus checks" /></div>
        </div>
      </section>
    </main>
  </div>;
}
