import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { expect, test } from "vitest";

const script = fileURLToPath(new URL("./merge-manifest.mjs", import.meta.url));
const fixture = (name) =>
  fileURLToPath(new URL(`./fixtures/manifest/${name}`, import.meta.url));

// Runs the script the way the release workflow does: fragment paths in,
// manifest on stdout, non-zero exit on anything that must block publishing.
const run = (...fragments) =>
  spawnSync(process.execPath, [script, ...fragments], { encoding: "utf8" });

test("merge-manifest merges the per-platform fragments into one manifest", () => {
  const result = run(fixture("darwin.json"), fixture("linux.json"));
  const manifest = JSON.parse(result.stdout);

  expect(result.status).toBe(0);
  expect(manifest.version).toBe("1.2.3");
  expect(Object.keys(manifest.platforms).sort()).toEqual([
    "darwin-aarch64",
    "darwin-x86_64",
    "linux-x86_64-deb",
  ]);
  expect(manifest.platforms["linux-x86_64-deb"]).toEqual({
    signature: "linux-signature",
    url: "https://example.test/releases/download/grapevine-v1.2.3/Grapevine-v1.2.3-amd64.deb",
  });
  expect(manifest.platforms["darwin-aarch64"].signature).toBe(
    "darwin-signature",
  );
  expect(Number.isNaN(Date.parse(manifest.pub_date))).toBe(false);
});

test("merge-manifest fails when a platform's fragment was not passed", () => {
  const result = run(fixture("darwin.json"));

  expect(result.status).not.toBe(0);
  expect(result.stdout).toBe("");
  expect(result.stderr).toContain("linux-x86_64-deb");
});

test("merge-manifest fails when a fragment file does not exist", () => {
  const result = run(fixture("darwin.json"), fixture("absent.json"));

  expect(result.status).not.toBe(0);
  expect(result.stdout).toBe("");
  expect(result.stderr).toContain("absent.json");
});

test("merge-manifest fails when the fragments come from different versions", () => {
  const result = run(
    fixture("darwin.json"),
    fixture("linux-other-version.json"),
  );

  expect(result.status).not.toBe(0);
  expect(result.stdout).toBe("");
  expect(result.stderr).toContain("1.2.4");
});

test("merge-manifest fails when a platform's signature is empty", () => {
  const result = run(fixture("darwin.json"), fixture("linux-unsigned.json"));

  expect(result.status).not.toBe(0);
  expect(result.stdout).toBe("");
  expect(result.stderr).toContain("linux-x86_64-deb");
});
