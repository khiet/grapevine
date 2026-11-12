// Capture the real popover on the fake backend and render the README GIF.
//
// Setup (Playwright is deliberately not a project dependency):
//     npm ci && npm i --no-save playwright && npx playwright install chromium
// Run from the repo (needs ffmpeg on PATH):
//     node scripts/demo/demo.mjs [--output docs/assets/grapevine.gif]
//
// Every PR in fixture.ts is fictional; the people and repo owners are real GitHub
// accounts whose avatars are fetched at capture time, so capturing needs
// network. Nothing else touches GitHub, and nothing touches the Keychain or
// the Rust backend: popover.tsx runs the real App on the fake backend from
// src/backend, so the demo shows exactly what the frontend renders for the
// Snapshot in fixture.ts.

import { execFileSync } from "node:child_process";
import { mkdtempSync, mkdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";
import { createServer } from "vite";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const outputArg = process.argv.indexOf("--output");
const OUTPUT = resolve(
  REPO,
  outputArg > -1 ? process.argv[outputArg + 1] : "docs/assets/grapevine.gif",
);
const TZ = "Europe/London";
const NOW = "2026-09-30T15:42:00+01:00";

// Avatars for every author and repo owner in fixture.ts, keyed by login and
// inlined as data URIs so every frame renders them without waiting on a load.
const AVATARS = {};
const ACCOUNT_IDS = {
  khiet: 310614,
  doutatsu: 4270980,
  MarcusAl: 71977744,
  dependabot: 49699333,
  teampave: 40337291,
  fuseinsight: 176946223,
};
for (const [login, id] of Object.entries(ACCOUNT_IDS)) {
  const url = `https://avatars.githubusercontent.com/u/${id}?v=4`;
  const avatar = await fetch(url);
  if (!avatar.ok) throw new Error(`fetching ${url}: ${avatar.status}`);
  AVATARS[login] = `data:${avatar.headers.get("content-type")};base64,${Buffer.from(await avatar.arrayBuffer()).toString("base64")}`;
}

async function capture(baseUrl, dir) {
  const browser = await chromium.launch();
  const context = await browser.newContext({
    viewport: { width: 1120, height: 760 },
    // Drawn at 2x so text stays sharp where the README shows it downscaled.
    deviceScaleFactor: 2,
    timezoneId: TZ,
    locale: "en-GB",
    colorScheme: "light",
  });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (msg) => msg.type() === "error" && errors.push(msg.text()));
  await page.clock.setFixedTime(new Date(NOW));
  // Read by popover.tsx, which builds the fixture Snapshot around them.
  await page.addInitScript((avatars) => {
    window.DEMO_AVATARS = avatars;
  }, AVATARS);
  await page.goto(`${baseUrl}/scripts/demo/demo.html`);
  const app = page.frameLocator("#popover");
  await app.locator(".pr-row").first().waitFor({ state: "attached" });

  const frames = [];
  async function pointAt(locator) {
    const box = await locator.boundingBox();
    await page.evaluate(
      ([x, y]) => {
        const cursor = document.getElementById("cursor");
        cursor.style.left = `${x - 3.7}px`;
        cursor.style.top = `${y - 1.8}px`;
        cursor.removeAttribute("hidden");
      },
      [box.x + box.width / 2, box.y + box.height / 2],
    );
  }
  const hideCursor = () =>
    page.evaluate(() => document.getElementById("cursor").setAttribute("hidden", ""));
  const trayCount = () => page.locator("#tray-count").textContent();
  async function snap(duration) {
    const file = join(dir, `frame-${String(frames.length).padStart(3, "0")}.png`);
    await page.screenshot({ path: file });
    frames.push({ file, duration });
  }
  const setOpen = (open) =>
    page.evaluate((open) => {
      document.getElementById("popover").hidden = !open;
      document.getElementById("tray").classList.toggle("is-open", open);
    }, open);
  // Each label stays once it has landed, so the last frame carries them all.
  async function label(target, side, text, chipY) {
    await page.evaluate((args) => window.callout(...args), [
      await target.boundingBox(),
      side,
      text,
      chipY ?? null,
    ]);
  }
  const row = (number) => app.locator("li", { hasText: number });

  // Mine starts folded; it is opened off-camera so one view shows both
  // sections and the popover never has to move.
  await setOpen(true);
  await app.locator(".pr-section-toggle", { hasText: "Mine" }).click();
  await row("#415").waitFor();
  await page.mouse.move(2, 400);
  // Let the chevron finish its rotation before any frame is taken.
  await page.waitForTimeout(400);
  await setOpen(false);

  const tray = page.locator("#tray");
  await pointAt(tray);
  await snap(900);
  await setOpen(true);
  await hideCursor();
  if ((await trayCount()) !== "4") throw new Error("tray should show 4");
  // The first four targets sit close together, so their chips are spread by hand.
  await label(tray, "right", "4 PRs waiting on you", 62);
  await snap(1700);
  await label(row("#415").locator(".pr-updated"), "right", "Your turn first, and for how long", 112);
  await snap(1600);
  await label(row("#415").locator(".pr-marks"), "right", "Only required checks count", 158);
  await snap(1500);
  await label(row("#1295").locator(".pr-marks"), "right", "Ready to merge", 208);
  await snap(1500);
  await label(row("#2320").locator(".pr-glyph-count"), "right", "Merge queue position");
  await snap(1500);
  await label(row("#2317").locator(".pr-glyph-wide"), "right", "Your team was asked to review");
  await snap(1500);
  await label(app.locator(".header-search"), "left", "Filter every repo");
  await snap(1500);
  await label(row("#1291").locator(".pr-unread"), "left", "Unread from people, never bots");
  await snap(1500);
  // The snooze button only shows on a hovered row.
  await row("#1284").hover();
  await label(row("#1284").locator(".pr-snooze"), "right", "Snooze until it changes");
  await snap(1800);
  await page.evaluate(() => document.getElementById("os").removeAttribute("hidden"));
  // The loop rests on the full summary so every label can be read.
  await snap(5000);

  // Nothing was clicked, so the count must not have moved.
  if ((await trayCount()) !== "4") throw new Error("tray should still show 4");
  const chips = await page.locator(".chip").evaluateAll((nodes) =>
    nodes.map((node) => {
      const { left, top, right, bottom } = node.getBoundingClientRect();
      return { text: node.textContent, left, top, right, bottom };
    }),
  );
  if (chips.length !== 9) throw new Error(`want 9 labels, got ${chips.length}`);
  const popoverBox = await page.locator("#popover").boundingBox();
  for (const [i, a] of chips.entries()) {
    if (a.left < 0 || a.right > 1120) throw new Error(`"${a.text}" runs off the stage`);
    if (a.right > popoverBox.x && a.left < popoverBox.x + popoverBox.width) {
      throw new Error(`"${a.text}" covers the popover`);
    }
    for (const b of chips.slice(i + 1)) {
      if (a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom) {
        throw new Error(`"${a.text}" overlaps "${b.text}"`);
      }
    }
  }
  const total = frames.reduce((sum, frame) => sum + frame.duration, 0);
  if (total !== 20000) throw new Error(`loop is ${total}ms, want 20000`);

  await browser.close();
  if (errors.length) throw new Error(`page errors:\n${errors.join("\n")}`);
  return frames;
}

function encode(frames, dir) {
  // The concat demuxer takes per-frame durations; the last entry is repeated
  // because the demuxer ignores the final duration.
  const list = frames
    .map(({ file, duration }) => `file '${file}'\nduration ${duration / 1000}`)
    .concat(`file '${frames.at(-1).file}'`)
    .join("\n");
  const listFile = join(dir, "frames.txt");
  writeFileSync(listFile, list);
  mkdirSync(dirname(OUTPUT), { recursive: true });
  // One palette across all frames avoids colour flicker between frames.
  execFileSync("ffmpeg", [
    "-v", "error", "-y",
    "-f", "concat", "-safe", "0", "-i", listFile,
    "-filter_complex",
    "[0:v]split[a][b];[a]palettegen=max_colors=192:stats_mode=full[p];[b][p]paletteuse=dither=none:diff_mode=rectangle",
    "-fps_mode", "vfr", "-loop", "0",
    OUTPUT,
  ]);
}

const server = await createServer({
  root: REPO,
  configFile: join(REPO, "vite.config.ts"),
  server: { port: 1430, strictPort: false },
  logLevel: "error",
});
await server.listen();
const dir = mkdtempSync(join(tmpdir(), "grapevine-demo-"));
try {
  const frames = await capture(server.resolvedUrls.local[0].replace(/\/$/, ""), dir);
  encode(frames, dir);
  const seconds = frames.reduce((sum, f) => sum + f.duration, 0) / 1000;
  console.log(
    `${OUTPUT}: ${seconds.toFixed(1)}s, ${frames.length} frames, ${Math.round(statSync(OUTPUT).size / 1024)} KiB`,
  );
} finally {
  rmSync(dir, { recursive: true, force: true });
  await server.close();
}
