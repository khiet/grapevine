import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { TICK_MS, tickWhileFocused } from "./redraw";

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

const focusable = () => {
  const target = new EventTarget() as unknown as Window;
  return {
    target,
    focus: () => target.dispatchEvent(new Event("focus")),
    blur: () => target.dispatchEvent(new Event("blur")),
  };
};

test("opening the popover redraws at once, then every tick while it stays open", () => {
  const { target, focus } = focusable();
  const tick = vi.fn();
  tickWhileFocused(target, false, tick);

  vi.advanceTimersByTime(TICK_MS * 3);
  expect(tick).not.toHaveBeenCalled();

  focus();
  expect(tick).toHaveBeenCalledTimes(1);
  vi.advanceTimersByTime(TICK_MS * 2);
  expect(tick).toHaveBeenCalledTimes(3);
});

test("a closed popover stops redrawing until it opens again", () => {
  const { target, focus, blur } = focusable();
  const tick = vi.fn();
  tickWhileFocused(target, true, tick);

  vi.advanceTimersByTime(TICK_MS);
  expect(tick).toHaveBeenCalledTimes(1);

  blur();
  vi.advanceTimersByTime(TICK_MS * 3);
  expect(tick).toHaveBeenCalledTimes(1);

  focus();
  expect(tick).toHaveBeenCalledTimes(2);
});

test("a repeated focus keeps one interval, and stopping ends it", () => {
  const { target, focus } = focusable();
  const tick = vi.fn();
  const stop = tickWhileFocused(target, false, tick);

  focus();
  focus();
  vi.advanceTimersByTime(TICK_MS);
  expect(tick).toHaveBeenCalledTimes(3);

  stop();
  focus();
  vi.advanceTimersByTime(TICK_MS * 2);
  expect(tick).toHaveBeenCalledTimes(3);
});
