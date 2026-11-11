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
  async function caption(step, key, headline, detail) {
    await page.evaluate(
      ([step, key, headline, detail]) => {
        document.getElementById("step").textContent = step;
        document.getElementById("key").textContent = key;
        document.getElementById("headline").textContent = headline;
        document.getElementById("detail").textContent = detail;
      },
      [step, key, headline, detail],
    );
  }
  async function pointAt(locator) {
    const box = await locator.boundingBox();
    await page.evaluate(
      ([x, y]) => {
        const cursor = document.getElementById("cursor");
        cursor.style.left = `${x - 3.7}px`;
        cursor.style.top = `${y - 1.8}px`;
        cursor.removeAttribute("hidden");
      },
      [box.x + Math.min(box.width / 2, 60), box.y + box.height / 2],
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
  const popover = page.locator("#popover");
  const setOpen = (open) =>
    page.evaluate((open) => {
      document.getElementById("popover").hidden = !open;
      document.getElementById("tray").classList.toggle("is-open", open);
    }, open);

  await caption("1", "Click", "Pull requests, in the menubar.", "The number is how many PRs are waiting on you.");
  await setOpen(false);
  await pointAt(page.locator("#tray"));
  await snap(2000);
  await setOpen(true);
  await hideCursor();
  await popover.waitFor();
  await snap(3400);
  if ((await trayCount()) !== "3") throw new Error("tray should start at 3");

  await caption("2", "Click", "Catch up where someone spoke.", "Red badges count unread activity. Opening the PR on GitHub clears it.");
  const webhookRow = app.locator(".pr-row", { hasText: "#1284" });
  await pointAt(webhookRow);
  await snap(1800);
  await webhookRow.click();
  await app.locator(".pr-row", { hasText: "#1284" }).locator(".pr-unread").waitFor({ state: "detached" });
  // Reading is not acting: the PR still waits on you.
  if ((await trayCount()) !== "3") throw new Error("tray should stay at 3");
  await snap(2800);

  await caption("3", "Click", "Your own PRs, and what blocks them.", "Approved, awaiting review, CI failing, a draft: each row says what comes next.");
  const mineToggle = app.locator(".pr-section-toggle", { hasText: "Mine" });
  await pointAt(mineToggle);
  await snap(1400);
  await mineToggle.click();
  await app.locator(".pr-row", { hasText: "#415" }).waitFor();
  // #2320 is the only queued PR in the fixture, so it alone carries the mark.
  const queued = app.locator(".pr-row", {
    has: app.locator('[aria-label$="in merge queue"]'),
  });
  if ((await queued.allTextContents()).join().match(/#\d+/g)?.join() !== "#2320") {
    throw new Error("only #2320 should show the merge queue mark");
  }
  // Let the chevron finish its rotation before the frame is taken.
  await page.waitForTimeout(400);
  await hideCursor();
  await snap(4200);

  await caption("4", "Type", "Filter across every repo.", "Matches title, repo, number, or author as you type.");
  const search = app.locator(".header-search-input");
  for (const char of "checkout") {
    await search.press(char === " " ? "Space" : char);
    await snap(140);
  }
  frames.at(-1).duration = 4200;
  if ((await app.locator(".pr-row", { hasText: "#412" }).count()) !== 0) {
    throw new Error("filter should hide non-matching rows");
  }

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
