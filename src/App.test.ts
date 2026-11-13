import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import { HeaderActions } from "./App";

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
