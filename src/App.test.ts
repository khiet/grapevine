import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import { HeaderActions, SyncStatus } from "./App";

const header = (offerMarkAllRead: boolean, inSettings = false) =>
  renderToStaticMarkup(
    createElement(HeaderActions, {
      inSettings,
      offerMarkAllRead,
      onShow: () => {},
    }),
  );

const buttonNamed = (html: string, name: string) =>
  html.match(new RegExp(`<button[^>]*aria-label="${name}"[^>]*>`))?.[0];

const SYNCED_AT = 1_790_000_000_000;

const footer = (
  error: string | null,
  syncing = false,
  lastSyncAt: number | null = SYNCED_AT,
) =>
  renderToStaticMarkup(
    createElement(SyncStatus, { lastSyncAt, error, syncing }),
  );

const retryButton = (html: string) =>
  html.match(/<button[^>]*>Retry<\/button>/)?.[0];

test("the header's icon buttons carry accessible names", () => {
  const html = header(true);
  expect(buttonNamed(html, "Mark all read")).toBeDefined();
  expect(buttonNamed(html, "Settings")).toBeDefined();
});

test("a dot on Mark all read says there is something to mark", () => {
  const html = header(true);
  expect(html).toContain("header-dot");
  expect(buttonNamed(html, "Mark all read")).not.toContain("disabled");
});

test("with nothing unread, Mark all read is disabled and shows no dot", () => {
  const html = header(false);
  expect(html).not.toContain("header-dot");
  expect(buttonNamed(html, "Mark all read")).toContain("disabled");
  expect(buttonNamed(html, "Settings")).not.toContain("disabled");
});

test("inside Settings the header offers only Done", () => {
  const html = header(true, true);
  expect(html).toContain("Done");
  expect(html).not.toContain("aria-label");
});

test("the footer offers Sync now beside the sync time", () => {
  const html = footer(null);
  expect(html).toContain("Synced ");
  const button = buttonNamed(html, "Sync now");
  expect(button).toContain('data-tip="Sync now"');
  expect(button).not.toContain("aria-disabled");
  expect(retryButton(html)).toBeUndefined();
});

// aria-disabled rather than disabled: the button that started the sync keeps
// keyboard focus through it.
test("while a sync runs the footer says so and Sync now is unavailable", () => {
  const html = footer(null, true);
  expect(html).toContain("Syncing");
  expect(html).not.toContain("Synced ");
  const button = buttonNamed(html, "Syncing");
  expect(button).toContain('aria-disabled="true"');
  expect(button).not.toContain(" disabled");
});

test("a sync error shows its message with a Retry button", () => {
  const html = footer("Could not reach GitHub.");
  expect(html).toContain("Could not reach GitHub.");
  expect(retryButton(html)).toBeDefined();
  expect(retryButton(html)).not.toContain("aria-disabled");
  expect(buttonNamed(html, "Sync now")).toBeDefined();
});

test("Retry is unavailable while the retry runs", () => {
  expect(retryButton(footer("Could not reach GitHub.", true))).toContain(
    'aria-disabled="true"',
  );
});

test("a first sync that failed can be retried, with no sync time to show", () => {
  const html = footer("Could not reach GitHub.", false, null);
  expect(retryButton(html)).toBeDefined();
  expect(buttonNamed(html, "Sync now")).toBeDefined();
  expect(html).not.toContain("Synced");
});

const HINTS = [
  { keys: "S", label: "Snooze" },
  { keys: "Esc", label: "Filter" },
];

const footerWithHints = (error: string | null) =>
  renderToStaticMarkup(
    createElement(SyncStatus, {
      lastSyncAt: SYNCED_AT,
      error,
      syncing: false,
      hints: HINTS,
    }),
  );

test("the footer's left side shows the keys that apply", () => {
  const html = footerWithHints(null);
  expect(html).toContain("<kbd>S</kbd>Snooze");
  expect(html).toContain("<kbd>Esc</kbd>Filter");
  // Left of the sync time.
  expect(html.indexOf("<kbd>")).toBeLessThan(html.indexOf("Synced "));
});

test("a sync error takes the hints' place", () => {
  const html = footerWithHints("Could not reach GitHub.");
  expect(html).not.toContain("<kbd>");
  expect(retryButton(html)).toBeDefined();
});

test("with no keys to hint at, the footer's left side is empty", () => {
  expect(footer(null)).not.toContain("<kbd>");
});
