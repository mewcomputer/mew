// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { MewClient, MewClientEvents, MewEventName } from "@mew/web-client";
import {
  TerminalPanel,
  type GhosttyFitAddonInstance,
  type GhosttyModule,
  type GhosttyTerminalInstance,
} from "../components/terminal-panel";

type HandlerMap = {
  [K in MewEventName]?: Set<MewClientEvents[K]>;
};

function createMockClient() {
  const handlers: HandlerMap = {};
  const terminalOpen = vi.fn();
  const terminalInput = vi.fn();
  const terminalResize = vi.fn();
  const terminalClose = vi.fn();

  const client = {
    terminalOpen,
    terminalInput,
    terminalResize,
    terminalClose,
    on: vi.fn(<E extends MewEventName>(event: E, cb: MewClientEvents[E]) => {
      let set = handlers[event] as Set<MewClientEvents[E]> | undefined;
      if (!set) {
        set = new Set();
        handlers[event] = set as HandlerMap[E];
      }
      set.add(cb);
    }),
    off: vi.fn(<E extends MewEventName>(event: E, cb: MewClientEvents[E]) => {
      (handlers[event] as Set<MewClientEvents[E]> | undefined)?.delete(cb);
    }),
  } as unknown as MewClient;

  function emit<E extends MewEventName>(
    event: E,
    ...args: Parameters<MewClientEvents[E]>
  ) {
    const set = handlers[event] as Set<(...a: Parameters<MewClientEvents[E]>) => void> | undefined;
    if (!set) return;
    for (const fn of [...set]) {
      fn(...args);
    }
  }

  return {
    client,
    emit,
    terminalOpen,
    terminalInput,
    terminalResize,
    terminalClose,
  };
}

function createFakeGhosttyHarness() {
  const instances: Array<{
    term: GhosttyTerminalInstance;
    fit: GhosttyFitAddonInstance;
    triggerData: (data: string) => void;
    triggerResize: (size: { cols: number; rows: number }) => void;
    written: Array<string | Uint8Array>;
    focused: number;
    disposed: number;
    fitted: number;
  }> = [];

  const loadGhostty = async (): Promise<GhosttyModule> => ({
    init: async () => {},
    Terminal: class FakeTerminal implements GhosttyTerminalInstance {
      cols = 96;
      rows = 30;
      options = { theme: {} };
      private dataListeners = new Set<(data: string) => void>();
      private resizeListeners = new Set<(size: { cols: number; rows: number }) => void>();
      private record = {
        term: this as GhosttyTerminalInstance,
        fit: null as unknown as GhosttyFitAddonInstance,
        triggerData: (data: string) => {
          for (const listener of this.dataListeners) listener(data);
        },
        triggerResize: (size: { cols: number; rows: number }) => {
          this.cols = size.cols;
          this.rows = size.rows;
          for (const listener of this.resizeListeners) listener(size);
        },
        written: [] as Array<string | Uint8Array>,
        focused: 0,
        disposed: 0,
        fitted: 0,
      };

      constructor() {
        instances.push(this.record);
      }

      open(): void {}
      write(data: string | Uint8Array): void {
        this.record.written.push(data);
      }
      focus(): void {
        this.record.focused += 1;
      }
      dispose(): void {
        this.record.disposed += 1;
      }
      loadAddon(addon: GhosttyFitAddonInstance): void {
        this.record.fit = addon;
      }
      onData(listener: (data: string) => void) {
        this.dataListeners.add(listener);
        return { dispose: () => this.dataListeners.delete(listener) };
      }
      onResize(listener: (size: { cols: number; rows: number }) => void) {
        this.resizeListeners.add(listener);
        return { dispose: () => this.resizeListeners.delete(listener) };
      }
    },
    FitAddon: class FakeFitAddon implements GhosttyFitAddonInstance {
      private record = instances[instances.length - 1];
      activate(): void {}
      dispose(): void {}
      fit(): void {
        if (this.record) this.record.fitted += 1;
      }
    },
  });

  return { loadGhostty, instances };
}

afterEach(() => {
  cleanup();
});

describe("TerminalPanel", () => {
  it("opens a PTY, streams output, forwards input and resize, and closes on unmount", async () => {
    const mock = createMockClient();
    const harness = createFakeGhosttyHarness();

    const { unmount } = render(
      <TerminalPanel
        client={mock.client}
        connected
        active
        sessionId="sess-1"
        sessionCwd="/Users/natalie/code/mew"
        jobCount={0}
        loadGhostty={harness.loadGhostty}
      />,
    );

    await waitFor(() => {
      expect(mock.terminalOpen).toHaveBeenCalledWith(30, 96);
    });

    const inst = harness.instances[0];
    if (!inst) throw new Error("expected terminal instance");
    act(() => {
      mock.emit("terminal-opened", { terminal_id: "term-1" });
    });

    expect(mock.terminalResize).toHaveBeenCalledWith("term-1", 30, 96);
    expect(inst.focused).toBeGreaterThanOrEqual(1);

    act(() => {
      mock.emit("terminal-output", { terminal_id: "term-1", bytes: [104, 105] });
      mock.emit("terminal-output", { terminal_id: "other-term", bytes: [120] });
    });

    expect(inst.written).toHaveLength(1);
    expect(Array.from(inst.written[0] as Uint8Array)).toEqual([104, 105]);

    act(() => {
      inst.triggerData("ls\n");
      inst.triggerResize({ cols: 110, rows: 40 });
    });

    expect(mock.terminalInput).toHaveBeenCalledTimes(1);
    const inputCall = mock.terminalInput.mock.calls[0];
    if (!inputCall) throw new Error("expected terminalInput call");
    expect(inputCall[0]).toBe("term-1");
    expect(Array.from(inputCall[1] as Uint8Array)).toEqual([108, 115, 10]);
    expect(mock.terminalResize).toHaveBeenLastCalledWith("term-1", 40, 110);

    unmount();
    expect(mock.terminalClose).toHaveBeenCalledWith("term-1");
    expect(inst.disposed).toBe(1);
  });

  it("shows exit status, restarts cleanly, and toggles background jobs", async () => {
    const mock = createMockClient();
    const harness = createFakeGhosttyHarness();

    render(
      <TerminalPanel
        client={mock.client}
        connected
        active
        sessionId="sess-1"
        sessionCwd="/Users/natalie/code/mew"
        jobCount={2}
        jobsPanel={<div>job-row-1</div>}
        loadGhostty={harness.loadGhostty}
      />,
    );

    await waitFor(() => expect(mock.terminalOpen).toHaveBeenCalledTimes(1));

    act(() => {
      mock.emit("terminal-opened", { terminal_id: "term-1" });
      mock.emit("terminal-exited", { terminal_id: "term-1", status: "exited with code 0" });
    });

    expect(screen.getByText("exited with code 0")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Jobs (2)" }));
    expect(screen.getByText("job-row-1")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Restart terminal" }));
    await waitFor(() => expect(mock.terminalOpen).toHaveBeenCalledTimes(2));
  });

  it("renders unavailable state when disconnected", () => {
    const mock = createMockClient();
    const harness = createFakeGhosttyHarness();

    render(
      <TerminalPanel
        client={mock.client}
        connected={false}
        active
        sessionId={null}
        sessionCwd={null}
        jobCount={0}
        loadGhostty={harness.loadGhostty}
      />,
    );

    expect(screen.getByText("Terminal unavailable")).toBeTruthy();
    expect(mock.terminalOpen).not.toHaveBeenCalled();
  });
});
