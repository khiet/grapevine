import { expect, test } from "vitest";
import source from "./App.css?raw";

// The stylesheet is the seam: the design system's rules (tokens only, three
// radii, three type sizes, AA contrast, one focus ring) are properties of
// App.css as written, so the tests read it rather than a rendered page.

const css = source.replace(/\/\*[\s\S]*?\*\//g, "");

interface Rule {
  selector: string;
  body: string;
  dark: boolean;
}

// Flat rule list. The only nesting App.css uses is the dark-mode media block.
function parseRules(text: string, dark = false): Rule[] {
  const rules: Rule[] = [];
  let i = 0;
  while (i < text.length) {
    const open = text.indexOf("{", i);
    if (open === -1) break;
    const selector = text.slice(i, open).trim();
    let depth = 1;
    let close = open + 1;
    for (; depth > 0; close++) {
      if (close >= text.length) throw new Error(`unclosed block: ${selector}`);
      if (text[close] === "{") depth++;
      else if (text[close] === "}") depth--;
    }
    const body = text.slice(open + 1, close - 1);
    if (selector.startsWith("@media")) {
      if (!selector.includes("prefers-color-scheme: dark")) {
        throw new Error(`unexpected at-rule: ${selector}`);
      }
      rules.push(...parseRules(body, true));
    } else {
      rules.push({ selector, body, dark });
    }
    i = close;
  }
  return rules;
}

function declarations(body: string): [string, string][] {
  return body
    .split(";")
    .map((d) => d.trim())
    .filter(Boolean)
    .map((d) => {
      const colon = d.indexOf(":");
      return [d.slice(0, colon).trim(), d.slice(colon + 1).trim().replace(/\s+/g, " ")];
    });
}

const rules = parseRules(css);
const isTokenBlock = (r: Rule) => r.selector === ":root";
const tokensOf = (dark: boolean) =>
  new Map(
    rules
      .filter((r) => isTokenBlock(r) && r.dark === dark)
      .flatMap((r) => declarations(r.body))
      .filter(([name]) => name.startsWith("--")),
  );
const light = tokensOf(false);
const darkOverrides = tokensOf(true);
const dark = new Map([...light, ...darkOverrides]);
const styleRules = rules.filter((r) => !isTokenBlock(r));
const valuesOf = (property: string) =>
  styleRules.flatMap((r) =>
    declarations(r.body)
      .filter(([name]) => name === property)
      .map(([, value]) => value),
  );

const COLOUR_LITERAL = /#[0-9a-f]{3,8}\b|\brgba?\(/i;

test("no colour literal remains outside the token definitions", () => {
  const offenders = styleRules
    .flatMap((r) => declarations(r.body).map(([name, value]) => `${r.selector} { ${name}: ${value} }`))
    .filter((d) => COLOUR_LITERAL.test(d));
  expect(offenders).toEqual([]);
});

test("dark mode is one block that redefines colour tokens only", () => {
  const darkRules = rules.filter((r) => r.dark);
  expect(darkRules.map((r) => r.selector)).toEqual([":root"]);
  expect(darkOverrides.size).toBeGreaterThan(0);
  for (const [name, value] of darkOverrides) {
    expect(light.has(name), `${name} has no light definition`).toBe(true);
    expect(value, `${name} is not a colour`).toMatch(COLOUR_LITERAL);
  }
});

test("only the three radii are in use", () => {
  expect(light.get("--radius-s")).toBe("4px");
  expect(light.get("--radius-m")).toBe("8px");
  expect(light.get("--radius-full")).toBe("999px");
  const radii = new Set(valuesOf("border-radius"));
  expect([...radii].sort()).toEqual([
    "var(--radius-full)",
    "var(--radius-m)",
    "var(--radius-s)",
  ]);
});

test("only the three type sizes and three weights are in use", () => {
  expect(light.get("--text-l")).toBe("13px");
  expect(light.get("--text-m")).toBe("12px");
  expect(light.get("--text-s")).toBe("11px");
  const sizes = new Set(valuesOf("font-size"));
  expect([...sizes].sort()).toEqual(["var(--text-l)", "var(--text-m)", "var(--text-s)"]);
  // The shorthand could smuggle a size in; only inheritance is allowed.
  expect(new Set(valuesOf("font"))).toEqual(new Set(["inherit"]));
  // 400 is the inherited default, so it need not be written out.
  expect(valuesOf("font-weight").filter((w) => !["400", "500", "700"].includes(w))).toEqual([]);
});

// The values the design review settled on, so a retune is a deliberate edit
// here as well as in the stylesheet.
test("the palette carries the agreed accent, secondary, unread and amber values", () => {
  expect(light.get("--accent")).toBe("#5b3fc4");
  expect(dark.get("--accent")).toBe("#b9a6ff");
  expect(light.get("--text-2")).toBe("#636366");
  expect(dark.get("--text-2")).toBe("#aeaeb2");
  expect(light.get("--unread")).toBe("#d92d20");
  expect(dark.get("--on-unread")).toBe("#2b0705");
  expect(light.get("--warn")).toBe("#8a4a00");
  expect(dark.get("--warn")).toBe("#ffc266");
});

test("one focus ring serves every control", () => {
  const outlined = styleRules.filter((r) =>
    declarations(r.body).some(([name, value]) => name === "outline" && value !== "none"),
  );
  expect(outlined).toHaveLength(1);
  const [ring] = outlined;
  expect(ring.selector.split(",").map((s) => s.trim())).toContain(":focus-visible");
  const ringDeclarations = new Map(declarations(ring.body));
  expect(ringDeclarations.get("outline")).toBe("2px solid var(--focus-colour, var(--accent))");
  expect(ringDeclarations.get("outline-offset")).toBe("var(--focus-offset, 2px)");
  // Rows fill their card edge to edge, so their ring draws inside.
  const inset = styleRules.filter((r) =>
    declarations(r.body).some(([name, value]) => name === "--focus-offset" && value === "-2px"),
  );
  expect(inset.flatMap((r) => r.selector.split(",").map((s) => s.trim()))).toContain(".pr-row");
  // The filter field shows the ring on its rounded box, not on the bare
  // input inside it.
  expect(ring.selector).toContain(".header-search:focus-within");
  // The update banner is an accent fill with an inset ring, so its ring
  // takes the colour that reads on the accent.
  const recoloured = styleRules.filter((r) =>
    declarations(r.body).some(
      ([name, value]) => name === "--focus-colour" && value === "var(--on-accent)",
    ),
  );
  expect(recoloured.map((r) => r.selector)).toEqual([".update-banner"]);
});

const ruleFor = (selector: string) => {
  const found = styleRules.filter((r) => r.selector === selector);
  expect(found, selector).toHaveLength(1);
  return new Map(declarations(found[0].body));
};

// A sticky header pins at the scroller's padding edge, so any top padding on
// the list is a strip where scrolled rows show above the header.
test("the list has no top padding for rows to show through", () => {
  expect(ruleFor(".pr-list").get("padding")).toMatch(/^0 /);
  expect(ruleFor(".pr-section-header").get("top")).toBe("0");
});

// What floats over a row (the action bar, a merged row's deploy status) is
// out of the flow, so it takes no width from the title, and hangs from the
// row's foot, away from the timestamp on the first line. Where the two boxes
// still touch (a one-line title), the timestamp is stacked on top, so the
// click there is the row's. The row is its own stacking context so that lift
// cannot reach the sticky section headers.
test("the row's floating controls sit on its last line, under the timestamp", () => {
  const end = ruleFor(".pr-row-end");
  expect(end.get("position")).toBe("absolute");
  expect(end.has("bottom")).toBe(true);
  expect(end.has("z-index")).toBe(false);
  const stamp = ruleFor(".pr-updated");
  expect(stamp.get("position")).toBe("relative");
  expect(stamp.get("z-index")).toBe("1");
  expect(ruleFor(".pr-row-split").get("z-index")).toBe("0");
});

// The timestamp is where a click opens the PR. No state of the row may take
// it away to put a control there.
test("no rule hides the timestamp", () => {
  const hiding = styleRules
    .filter((r) => r.selector.includes(".pr-updated"))
    .flatMap((r) => declarations(r.body))
    .filter(([name]) => ["visibility", "display", "opacity"].includes(name));
  expect(hiding).toEqual([]);
});

const px = (value: string | undefined) => Number(value?.match(/^(\d+)px$/)?.[1]);

test("every row action is a target of at least 24 x 24px", () => {
  const action = ruleFor(".pr-action");
  expect(px(action.get("height"))).toBeGreaterThanOrEqual(24);
  expect(px(action.get("min-width"))).toBeGreaterThanOrEqual(24);
  const item = ruleFor(".pr-menu-item");
  expect(px(item.get("min-height"))).toBeGreaterThanOrEqual(24);
});

// The bar is right-aligned, so a button that narrowed on arming would slide
// its neighbour under the pointer and hand it the confirming click. The old
// label stays in the flow, unseen, with the new one stacked in its cell.
test("an armed action keeps the width of the label it replaced", () => {
  expect(ruleFor(".pr-action.is-armed").get("display")).toBe("grid");
  expect(ruleFor(".pr-action-width,\n.pr-action-over").get("grid-area")).toBe("1 / 1");
  expect(ruleFor(".pr-action-width").get("visibility")).toBe("hidden");
});

test("the action bar shows on hover and on keyboard focus", () => {
  expect(ruleFor(".pr-actions").get("opacity")).toBe("0");
  expect(ruleFor(".pr-row-split:hover .pr-actions").get("opacity")).toBe("1");
  expect(ruleFor(".pr-row-split:has(:focus-visible) .pr-actions").get("opacity")).toBe("1");
});

test("the header icon buttons are 28px square", () => {
  const icon = ruleFor(".header-icon");
  expect(icon.get("width")).toBe("28px");
  expect(icon.get("height")).toBe("28px");
});

test("the waiting label is accent-coloured and bold", () => {
  const label = ruleFor(".pr-title-row.is-waiting .pr-updated");
  expect(label.get("color")).toBe("var(--accent)");
  expect(label.get("font-weight")).toBe("700");
});

// Each tone of a row's state is a text token the contrast pairs below cover
// on the card, plain and hovered; the neutral tone and the "+N" are the
// metadata grey.
test("a row's state takes its tone from the text tokens", () => {
  expect(ruleFor(".pr-state.is-accent").get("color")).toBe("var(--accent)");
  expect(ruleFor(".pr-state.is-warn").get("color")).toBe("var(--warn)");
  expect(ruleFor(".pr-state.is-ok").get("color")).toBe("var(--ok)");
  expect(ruleFor(".pr-state").has("color")).toBe(false);
  expect(ruleFor(".pr-origin").get("color")).toBe("var(--text-2)");
  expect(ruleFor(".pr-state-more").get("color")).toBe("var(--text-2)");
});

// --- Contrast -------------------------------------------------------------

type Rgba = [number, number, number, number];

function parseColour(value: string): Rgba {
  const hex = value.match(/^#([0-9a-f]{6})$/i);
  if (hex) {
    const n = parseInt(hex[1], 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255, 1];
  }
  const rgb = value.match(/^rgba?\(([^)]+)\)$/);
  if (!rgb) throw new Error(`unparsed colour: ${value}`);
  const [r, g, b, a = 1] = rgb[1].split(",").map((part) => Number(part.trim()));
  return [r, g, b, a];
}

const over = (top: Rgba, under: Rgba): Rgba => [
  top[0] * top[3] + under[0] * (1 - top[3]),
  top[1] * top[3] + under[1] * (1 - top[3]),
  top[2] * top[3] + under[2] * (1 - top[3]),
  1,
];

function luminance([r, g, b]: Rgba): number {
  const channel = (c: number) => {
    const s = c / 255;
    return s <= 0.03928 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

function contrast(text: Rgba, background: Rgba): number {
  const [hi, lo] = [luminance(over(text, background)), luminance(background)].sort((a, b) => b - a);
  return (hi + 0.05) / (lo + 0.05);
}

test("the contrast maths matches WCAG's worked values", () => {
  expect(contrast([0, 0, 0, 1], [255, 255, 255, 1])).toBeCloseTo(21, 5);
  // The lightest grey that passes AA on white, a figure WCAG tooling agrees on.
  expect(contrast(parseColour("#767676"), parseColour("#ffffff"))).toBeCloseTo(4.54, 2);
});

// Every text colour against every surface it is drawn on, bottom layer first.
// Hand-kept: a rule that puts a text token on a new surface adds its pair here.
// Surfaces under a row are listed plain and hovered, since the hover tint
// sits under everything in the row.
const CARD = ["--card"];
const HOVERED = ["--card", "--hover"];
const WINDOW = ["--window"];
const onRow = (text: string, ...fills: string[]): [string, string[]][] => [
  [text, [...CARD, ...fills]],
  [text, [...HOVERED, ...fills]],
];
const PAIRS: [string, string[]][] = [
  // Window-level text: section and repo headers, empty states, the footer.
  ["--text", WINDOW],
  ["--text-2", WINDOW],
  ["--accent", WINDOW],
  ["--warn", WINDOW],
  ["--danger", WINDOW],
  ["--ok", WINDOW],
  ["--text", ["--window", "--fill-strong"]],
  ["--text-2", ["--window", "--fill-solid"]],
  ["--text", ["--window", "--fill"]],
  ["--text-2", ["--window", "--fill"]],
  ["--on-unread", ["--unread"]],
  ["--on-accent", ["--accent"]],
  ["--tip-text", ["--card", "--tip-bg"]],
  ["--text", ["--field"]],
  ["--text-2", ["--field"]],
  // Row and card text, plain and hovered.
  ...onRow("--text"),
  ...onRow("--text-2"),
  ...onRow("--accent"),
  ...onRow("--warn"),
  ...onRow("--danger"),
  ...onRow("--ok"),
  ...onRow("--text", "--fill"),
  ...onRow("--text-2", "--fill-solid"),
  // The action bar's labels, plain and under the pointer.
  ...onRow("--text", "--fill-solid"),
  ...onRow("--text", "--fill-solid", "--fill-strong"),
];

function ratios(theme: Map<string, string>): { pair: string; ratio: number }[] {
  const colour = (token: string) => {
    const value = theme.get(token);
    if (!value) throw new Error(`missing token ${token}`);
    return parseColour(value);
  };
  return PAIRS.map(([text, layers]) => {
    const background = layers.map(colour).reduce((under, top) => over(top, under));
    return {
      pair: `${text} on ${layers.join(" + ")}`,
      ratio: Math.round(contrast(colour(text), background) * 100) / 100,
    };
  });
}

test.each([
  ["light", light],
  ["dark", dark],
])("every text and fill pair passes 4.5:1 in the %s theme", (_name, theme) => {
  expect(ratios(theme).filter((r) => r.ratio < 4.5)).toEqual([]);
});

const usedTokens = new Set(
  styleRules.flatMap((r) => [...r.body.matchAll(/var\((--[a-z0-9-]+)/g)].map((m) => m[1])),
);
// Set per control, never defined on :root.
usedTokens.delete("--focus-offset");
usedTokens.delete("--focus-colour");

test("every token a rule reads is defined", () => {
  expect([...usedTokens].filter((name) => !light.has(name))).toEqual([]);
});

// So the token list stays the real palette rather than a superset of it.
test("every defined token is read by a rule", () => {
  expect([...light.keys()].filter((name) => !usedTokens.has(name))).toEqual([]);
});
