// Keeps the popover's time-based labels (the merge queue ETA, the waiting
// age) current: they are derived from the clock at render, so the popover
// renders again every TICK_MS, and at once when it gains focus, which is
// when it opens. The tick does not wait on focus: should the window's focus
// event go missing, an opened popover is still at most one tick stale.

import { useEffect, useReducer } from "react";

export const TICK_MS = 30_000;

type FocusTarget = Pick<Window, "addEventListener" | "removeEventListener">;

/** Calls `tick` every TICK_MS, and at once each time `target` gains focus.
 * Returns the function that stops it. */
export function tickAndOnFocus(
  target: FocusTarget,
  tick: () => void,
): () => void {
  const timer = setInterval(tick, TICK_MS);
  target.addEventListener("focus", tick);
  return () => {
    clearInterval(timer);
    target.removeEventListener("focus", tick);
  };
}

/** Renders the caller again when the popover opens and every TICK_MS. */
export function useRedrawWhileOpen(): void {
  const [, redraw] = useReducer((n: number) => n + 1, 0);
  useEffect(() => tickAndOnFocus(window, redraw), []);
}
