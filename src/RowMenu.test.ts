import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import RowMenu, { menuKey, opensMenu, placeMenu } from "./RowMenu";

test("the arrow keys walk the items and wrap at either end", () => {
  expect(menuKey("ArrowDown", 0, 3)).toEqual({ focus: 1 });
  expect(menuKey("ArrowDown", 2, 3)).toEqual({ focus: 0 });
  expect(menuKey("ArrowUp", 1, 3)).toEqual({ focus: 0 });
  expect(menuKey("ArrowUp", 0, 3)).toEqual({ focus: 2 });
});

test("Home and End jump to the first and last item", () => {
  expect(menuKey("Home", 2, 3)).toEqual({ focus: 0 });
  expect(menuKey("End", 0, 3)).toEqual({ focus: 2 });
});

// Tab closes rather than moving on: the menu floats outside the tab order of
// the list, so walking out of it would strand it open.
test("Escape and Tab close the menu", () => {
  expect(menuKey("Escape", 1, 3)).toBe("close");
  expect(menuKey("Tab", 1, 3)).toBe("close");
});

// Enter and Space are left to the item, which is a button.
test("any other key is left alone", () => {
  expect(menuKey("Enter", 0, 3)).toBeNull();
  expect(menuKey(" ", 0, 3)).toBeNull();
  expect(menuKey("a", 0, 3)).toBeNull();
});

test("the context-menu key and Shift+F10 open the menu", () => {
  expect(opensMenu({ key: "ContextMenu", shiftKey: false })).toBe(true);
  expect(opensMenu({ key: "F10", shiftKey: true })).toBe(true);
  expect(opensMenu({ key: "F10", shiftKey: false })).toBe(false);
  expect(opensMenu({ key: "Enter", shiftKey: false })).toBe(false);
});

const VIEWPORT = { width: 360, height: 440 };
const MENU = { width: 150, height: 90 };

test("the menu opens with its corner at the pointer when there is room", () => {
  expect(placeMenu({ x: 100, y: 200 }, MENU, VIEWPORT)).toEqual({ x: 100, y: 200 });
});

test("the menu is pulled back inside the window, 4px clear of its edges", () => {
  expect(placeMenu({ x: 350, y: 430 }, MENU, VIEWPORT)).toEqual({ x: 206, y: 346 });
  expect(placeMenu({ x: 0, y: 0 }, MENU, VIEWPORT)).toEqual({ x: 4, y: 4 });
});

test("the menu lists each action as a named menu item, in order", () => {
  const html = renderToStaticMarkup(
    createElement(RowMenu, {
      at: { x: 10, y: 10 },
      items: [
        { id: "snooze", label: "Snooze" },
        { id: "dismiss", label: "Dismiss" },
      ],
      onPick: () => {},
      onClose: () => {},
    }),
  );
  expect(html).toMatch(/^<div[^>]* role="menu"/);
  const items = [...html.matchAll(/<button[^>]* role="menuitem"[^>]*>([^<]*)</g)];
  expect(items.map((m) => m[1])).toEqual(["Snooze", "Dismiss"]);
});
