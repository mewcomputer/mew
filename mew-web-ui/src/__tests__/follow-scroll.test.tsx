import { act, cleanup, fireEvent, renderHook } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { useFollowScroll } from "../lib/use-follow-scroll";

afterEach(() => cleanup());

it("preserves scrolling performed while older messages are loading", () => {
  let height = 1000;
  const element = document.createElement("div");
  Object.defineProperties(element, {
    scrollHeight: { get: () => height },
    clientHeight: { value: 300 },
  });
  element.scrollTop = 200;
  const { result, rerender } = renderHook(({ revision }) => useFollowScroll<HTMLDivElement>([revision]), {
    initialProps: { revision: 0 },
  });
  act(() => result.current.setRef(element));
  act(() => result.current.holdPosition());
  element.scrollTop = 100;
  fireEvent.scroll(element);
  height = 1400;
  rerender({ revision: 1 });
  expect(element.scrollTop).toBe(500);
});

it("stops following as soon as the reader scrolls upward", () => {
  const element = document.createElement("div");
  Object.defineProperties(element, {
    scrollHeight: { value: 1000 },
    clientHeight: { value: 300 },
  });
  element.scrollTop = 700;
  const { result, rerender } = renderHook(({ revision }) => useFollowScroll<HTMLDivElement>([revision]), {
    initialProps: { revision: 0 },
  });
  act(() => result.current.setRef(element));
  fireEvent.wheel(element, { deltaY: -80 });
  element.scrollTop = 680;
  fireEvent.scroll(element);
  rerender({ revision: 1 });
  expect(element.scrollTop).toBe(680);
});
