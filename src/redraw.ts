// Keeps the popover's time-based labels (the merge queue ETA, the waiting
// age) current: they are derived from the clock at render, so the popover
// renders again when it opens and every TICK_MS while it stays open. The
// popover hides when it loses focus, so focus is what "open" means here.

import { useEffect, useReducer } from "react";

export const TICK_MS = 30_000;

type FocusTarget = Pick<Window, "addEventListener" | "removeEventListener">;

/** Calls `tick` each time `target` gains focus, then every TICK_MS until it
 * loses focus. `focused` says whether it has focus already, which starts
 * the interval without a first tick. Returns the function that stops it all. */
export function tickWhileFocused(
  target: FocusTarget,
  focused: boolean,
  tick: () => void,
): () => void {
  let timer: ReturnType<typeof setInterval> | null = null;
  const stopTimer = () => {
    if (timer !== null) clearInterval(timer);
    timer = null;
  };
  const startTimer = () => {
    stopTimer();
    timer = setInterval(tick, TICK_MS);
  };
  const onFocus = () => {
    tick();
    startTimer();
  };
  target.addEventListener("focus", onFocus);
  target.addEventListener("blur", stopTimer);
  if (focused) startTimer();
  return () => {
    target.removeEventListener("focus", onFocus);
    target.removeEventListener("blur", stopTimer);
    stopTimer();
  };
}

/** Renders the caller again when the popover opens and every TICK_MS while
 * it is open. */
export function useRedrawWhileOpen(): void {
  const [, redraw] = useReducer((n: number) => n + 1, 0);
  useEffect(() => tickWhileFocused(window, document.hasFocus(), redraw), []);
}
