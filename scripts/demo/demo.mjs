// Capture the real popover against a mocked backend and render the README GIF.
//
// Setup (Playwright is deliberately not a project dependency):
//     npm ci && npm i --no-save playwright && npx playwright install chromium
// Run from the repo (needs ffmpeg on PATH):
//     node scripts/demo/demo.mjs [--output docs/assets/grapevine.gif]
//
// Every PR below is fictional; the people and repo owners are real GitHub
// accounts whose avatars are fetched at capture time, so capturing needs
// network. Nothing else touches GitHub, and nothing touches the Keychain or
// the Rust backend: the Tauri IPC is replaced in the page, so the demo shows
// exactly what the frontend renders for a given snapshot.

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
const at = (time) => `2026-09-${time}+01:00`;

function pr(repo, number, title, author, updated, extra = {}) {
  const org = repo.split("/")[0];
  return {
    number,
    title,
    url: `https://github.com/${repo}/pull/${number}`,
    repo,
    author,
    avatar_url: AVATARS[author],
    owner_avatar_url: AVATARS[org],
    created_at: at("20T09:00:00"),
    updated_at: at(updated),
    section: "participated",
    blocked_reasons: [],
    is_draft: false,
    review_requested: false,
    awaiting_review: false,
    approved: false,
    unread_count: 0,
    ...extra,
  };
}

const ME = "khiet";
// Avatars for every author and repo owner, keyed by login and inlined as data
// URIs so every frame renders them without waiting on a load.
const AVATARS = {};
const ACCOUNT_IDS = {
  [ME]: 310614,
  doutatsu: 4270980,
  MarcusAl: 71977744,
  teampave: 40337291,
  fuseinsight: 176946223,
};
for (const [login, id] of Object.entries(ACCOUNT_IDS)) {
  const url = `https://avatars.githubusercontent.com/u/${id}?v=4`;
  const avatar = await fetch(url);
  if (!avatar.ok) throw new Error(`fetching ${url}: ${avatar.status}`);
  AVATARS[login] = `data:${avatar.headers.get("content-type")};base64,${Buffer.from(await avatar.arrayBuffer()).toString("base64")}`;
}
const mine = { section: "mine" };
const all = { section: "all" };
const SNAPSHOT = {
  has_synced: true,
  last_sync_at: Date.parse(at("30T15:41:00")),
  sync_error: null,
  prs: [
    pr("teampave/checkout-api", 1284, "Retry failed webhook deliveries with exponential backoff", "doutatsu", "30T15:31:00", { unread_count: 3, review_requested: true }),
    pr("teampave/checkout-api", 1291, "Add idempotency keys to POST /payments", ME, "30T15:12:00", { ...mine, unread_count: 1, awaiting_review: true }),
    pr("teampave/web-app", 2317, "Add saved payment methods to the checkout form", "MarcusAl", "30T14:58:00", { unread_count: 1, blocked_reasons: ["threads"] }),
    pr("fuseinsight/ingest", 412, "Stream podcast RSS imports instead of buffering them", "doutatsu", "30T12:10:00", { approved: true }),
    pr("teampave/web-app", 2320, "Upgrade to React 19 and drop legacy context", ME, "30T11:40:00", { ...mine, approved: true }),
    pr("khiet/design-tokens", 88, "Rename the spacing scale to t-shirt sizes", "MarcusAl", "29T17:20:00", { review_requested: true }),
    pr("fuseinsight/ingest", 415, "Parse chapter markers from ID3 tags", ME, "29T16:05:00", { ...mine, blocked_reasons: ["ci"] }),
    pr("teampave/checkout-api", 1279, "Fix rounding in multi-currency refunds", "doutatsu", "28T10:45:00", { unread_count: 2, blocked_reasons: ["conflict", "behind"] }),
    pr("teampave/web-app", 2322, "Spike: dark mode for the receipt page", ME, "26T14:30:00", { ...mine, is_draft: true }),
    pr("teampave/web-app", 2315, "Lazy-load the order history table", "MarcusAl", "30T10:20:00", all),
    pr("teampave/checkout-api", 1270, "Bump stripe-node to 17.4", "doutatsu", "29T08:00:00", all),
    pr("fuseinsight/ingest", 409, "Move transcoding jobs to the shared queue", "MarcusAl", "27T13:15:00", all),
  ],
  merged: [
    { number: 2309, title: "Show delivery ETA on the order confirmation", repo: "teampave/web-app", author: ME },
    { number: 85, title: "Add focus ring tokens", repo: "khiet/design-tokens", author: "MarcusAl" },
  ].map((m, i) => ({
    ...m,
    url: `https://github.com/${m.repo}/pull/${m.number}`,
    avatar_url: AVATARS[m.author],
    owner_avatar_url: AVATARS[m.repo.split("/")[0]],
    merged_at: at(i === 0 ? "30T13:05:00" : "29T15:50:00"),
  })),
};

// Runs in every frame before the app loads; only the popover iframe (the real
// index.html) gets a backend. The stage's tray count mirrors the unread total
// the way the Rust side sets the tray title.
function installBackend(initial) {
  if (location.pathname !== "/index.html") return;
  let snapshot = initial;
  const callbacks = new Map();
  const listeners = new Map();
  let nextId = 1;
  const unread = () => snapshot.prs.reduce((sum, p) => sum + p.unread_count, 0);
  const publish = () => {
    parent.document.getElementById("tray-count").textContent = unread() || "";
    for (const id of listeners.get("prs-updated") ?? []) {
      callbacks.get(id)?.({ event: "prs-updated", id, payload: snapshot });
    }
  };
  const commands = {
    get_prs: () => snapshot,
    "plugin:event|listen": ({ event, handler }) => {
      listeners.set(event, [...(listeners.get(event) ?? []), handler]);
      return handler;
    },
    "plugin:event|unlisten": () => null,
    "plugin:opener|open_url": () => null,
    mark_read: ({ key }) => {
      snapshot = {
        ...snapshot,
        prs: snapshot.prs.map((p) =>
          `${p.repo}#${p.number}` === key ? { ...p, unread_count: 0 } : p,
        ),
      };
      publish();
      return null;
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} };
  window.__TAURI_INTERNALS__ = {
    metadata: {
      currentWindow: { label: "main" },
      currentWebview: { windowLabel: "main", label: "main" },
    },
    transformCallback(callback) {
      const id = nextId++;
      callbacks.set(id, callback);
      return id;
    },
    unregisterCallback(id) {
      callbacks.delete(id);
    },
    async invoke(cmd, args) {
      const handler = commands[cmd];
      if (!handler) throw new Error(`demo backend: unmocked command ${cmd}`);
      return handler(args ?? {});
    },
  };
  addEventListener("DOMContentLoaded", publish);
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
  await page.addInitScript(installBackend, SNAPSHOT);
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

  await caption("1", "Click", "Pull requests, in the menubar.", "The count is new comments and reviews on PRs you wrote or joined.");
  await setOpen(false);
  await pointAt(page.locator("#tray"));
  await snap(2000);
  await setOpen(true);
  await hideCursor();
  await popover.waitFor();
  await snap(3400);
  if ((await trayCount()) !== "7") throw new Error("tray should start at 7");

  await caption("2", "Click", "Catch up where someone spoke.", "Red badges count unread activity. Opening the PR on GitHub clears it.");
  const webhookRow = app.locator(".pr-row", { hasText: "#1284" });
  await pointAt(webhookRow);
  await snap(1800);
  await webhookRow.click();
  await app.locator(".pr-row", { hasText: "#1284" }).locator(".pr-unread").waitFor({ state: "detached" });
  if ((await trayCount()) !== "4") throw new Error("tray should drop to 4");
  await snap(2800);

  await caption("3", "Click", "Your own PRs, and what blocks them.", "Approved, awaiting review, CI failing, a draft: each row says what comes next.");
  const mineToggle = app.locator(".pr-section-toggle", { hasText: "Mine" });
  await pointAt(mineToggle);
  await snap(1400);
  await mineToggle.click();
  await app.locator(".pr-row", { hasText: "#415" }).waitFor();
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
