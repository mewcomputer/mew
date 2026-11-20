import "@testing-library/jest-dom/vitest";

// Node's experimental localStorage can shadow the jsdom storage object.
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  value: (globalThis as typeof globalThis & { jsdom: { window: Window } }).jsdom.window.localStorage,
});

// jsdom doesn't implement matchMedia — required by use-mobile hook.
Object.defineProperty(window, "matchMedia", {
  writable: true,
  value: (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addListener: () => {},
    removeListener: () => {},
    addEventListener: () => {},
    removeEventListener: () => {},
    dispatchEvent: () => false,
  }),
});

// cmdk observes its list even while the command dialog is closed. jsdom has
// no layout engine, so a no-op observer keeps that behavior testable.
class TestResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

globalThis.ResizeObserver = TestResizeObserver as typeof ResizeObserver;
Element.prototype.scrollIntoView = () => {};
