import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { TICK_MS, tickAndOnFocus } from "./redraw";

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

const focusable = () => {
  const target = new EventTarget() as unknown as Window;
  return { target, focus: () => target.dispatchEvent(new Event("focus")) };
};

test("the popover redraws every tick", () => {
  const { target } = focusable();
  const tick = vi.fn();
  tickAndOnFocus(target, tick);

  vi.advanceTimersByTime(TICK_MS - 1);
  expect(tick).not.toHaveBeenCalled();
  vi.advanceTimersByTime(1);
  expect(tick).toHaveBeenCalledTimes(1);
  vi.advanceTimersByTime(TICK_MS * 2);
  expect(tick).toHaveBeenCalledTimes(3);
});

test("opening the popover redraws at once, without waiting for the tick", () => {
  const { target, focus } = focusable();
  const tick = vi.fn();
  tickAndOnFocus(target, tick);

  vi.advanceTimersByTime(TICK_MS / 2);
  focus();
  expect(tick).toHaveBeenCalledTimes(1);
});

test("stopping ends both the tick and the redraw on focus", () => {
  const { target, focus } = focusable();
  const tick = vi.fn();
  const stop = tickAndOnFocus(target, tick);

  stop();
  focus();
  vi.advanceTimersByTime(TICK_MS * 3);
  expect(tick).not.toHaveBeenCalled();
});
