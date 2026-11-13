// A row's right-click menu, drawn in the webview: the same actions as the
// row's action bar, on a route that covers none of the row's text.

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { RowActionId } from "./popoverView";

export interface Point {
  x: number;
  y: number;
}

interface Size {
  width: number;
  height: number;
}

// How far the menu keeps from the window's edges.
const EDGE = 4;

/** Where a menu of `menu` size asked for at `at` goes so that all of it is
 * inside `viewport`: at the point itself when it fits there. */
export function placeMenu(at: Point, menu: Size, viewport: Size): Point {
  const fit = (want: number, size: number, room: number) =>
    Math.max(EDGE, Math.min(want, room - size - EDGE));
  return {
    x: fit(at.x, menu.width, viewport.width),
    y: fit(at.y, menu.height, viewport.height),
  };
}

/** What a key pressed on item `index` of a `count`-item menu does: move focus
 * to another item, close the menu, or nothing the menu handles. */
export function menuKey(
  key: string,
  index: number,
  count: number,
): { focus: number } | "close" | null {
  switch (key) {
    case "ArrowDown":
      return { focus: (index + 1) % count };
    case "ArrowUp":
      return { focus: (index - 1 + count) % count };
    case "Home":
      return { focus: 0 };
    case "End":
      return { focus: count - 1 };
    case "Escape":
    case "Tab":
      return "close";
    default:
      return null;
  }
}

/** Whether a key press asks for the context menu. macOS keyboards have no
 * context-menu key, so Shift+F10 is the route there. */
export function opensMenu(event: { key: string; shiftKey: boolean }): boolean {
  return event.key === "ContextMenu" || (event.key === "F10" && event.shiftKey);
}

export interface MenuItem {
  id: RowActionId;
  label: string;
}

/** The open menu. Mount it to open and unmount it to close: it takes focus
 * when it mounts, and asks to be closed through `onClose`, whose argument
 * says whether focus should go back to the row (a close from the keyboard
 * or from picking an item) or stay where the user put it (a click
 * elsewhere). Picking does not close the menu; the caller decides, since
 * an item can need a second pick. */
function RowMenu({
  at,
  items,
  onPick,
  onClose,
}: {
  at: Point;
  items: MenuItem[];
  onPick: (id: RowActionId) => void;
  onClose: (refocus: boolean) => void;
}) {
  const menu = useRef<HTMLDivElement>(null);
  const [placed, setPlaced] = useState(at);

  // Before paint, so the menu is never seen hanging off the window.
  useLayoutEffect(() => {
    const box = menu.current?.getBoundingClientRect();
    if (!box) return;
    setPlaced(
      placeMenu(at, box, { width: window.innerWidth, height: window.innerHeight }),
    );
  }, [at.x, at.y]);

  useEffect(() => {
    menu.current?.querySelector("button")?.focus();
  }, []);

  // Read through a ref so the listeners below are set once per menu, not
  // once per render of the row.
  const close = useRef(onClose);
  close.current = onClose;
  useEffect(() => {
    const outside = (event: Event) => {
      if (!menu.current?.contains(event.target as Node)) close.current(false);
    };
    const gone = () => close.current(false);
    document.addEventListener("pointerdown", outside);
    // Fixed to the window, the menu would stay put while its row scrolled
    // away; capture, because scroll does not bubble out of the list.
    document.addEventListener("scroll", gone, true);
    window.addEventListener("resize", gone);
    // The popover hides when it loses focus, and must not reopen on a menu.
    window.addEventListener("blur", gone);
    return () => {
      document.removeEventListener("pointerdown", outside);
      document.removeEventListener("scroll", gone, true);
      window.removeEventListener("resize", gone);
      window.removeEventListener("blur", gone);
    };
  }, []);

  return (
    <div
      ref={menu}
      className="pr-menu"
      role="menu"
      style={{ left: placed.x, top: placed.y }}
    >
      {items.map((item, index) => (
        <button
          key={item.id}
          type="button"
          role="menuitem"
          className="pr-menu-item"
          onClick={() => onPick(item.id)}
          onKeyDown={(event) => {
            const result = menuKey(event.key, index, items.length);
            if (result === null) return;
            event.preventDefault();
            // The row listens for keys too, and would act on these.
            event.stopPropagation();
            if (result === "close") onClose(true);
            else menu.current?.querySelectorAll("button")[result.focus]?.focus();
          }}
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}

export default RowMenu;
