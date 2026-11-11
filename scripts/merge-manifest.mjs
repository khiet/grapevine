#!/usr/bin/env node
// Merges the per-platform updater manifest fragments a release's build jobs
// produce into the one manifest installed apps poll, printed on stdout.
//
//   node scripts/merge-manifest.mjs <fragment.json>... > latest.json
//
// A fragment is { version, platforms: { <key>: { signature, url } } }.
//
// Exits non-zero, printing nothing on stdout, unless every platform Grapevine
// ships is covered by fragments of one version. A partial manifest would make
// the updater on the missing platform fail, or hand it another release's
// artifact, so the caller must publish only what this prints on success.

import { readFileSync } from "node:fs";

// Every platform key a release must carry. Both macOS keys point at the same
// universal tarball; the Linux key is the updater's deb-specific one.
const REQUIRED_PLATFORMS = ["darwin-aarch64", "darwin-x86_64", "linux-x86_64-deb"];

function readFragment(path) {
  let fragment;
  try {
    fragment = JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    throw new Error(`cannot read fragment ${path}: ${error.message}`);
  }
  if (typeof fragment.version !== "string" || fragment.version === "") {
    throw new Error(`fragment ${path} has no version`);
  }
  if (typeof fragment.platforms !== "object" || fragment.platforms === null) {
    throw new Error(`fragment ${path} has no platforms`);
  }
  return fragment;
}

function merge(paths) {
  let version;
  const platforms = {};

  for (const path of paths) {
    const fragment = readFragment(path);

    version ??= fragment.version;
    if (fragment.version !== version) {
      throw new Error(
        `fragment ${path} is version ${fragment.version}, expected ${version}`,
      );
    }

    for (const [key, entry] of Object.entries(fragment.platforms)) {
      if (key in platforms) {
        throw new Error(`fragment ${path} repeats platform ${key}`);
      }
      if (!entry?.signature || !entry?.url) {
        throw new Error(`fragment ${path} has no signature or url for ${key}`);
      }
      platforms[key] = { signature: entry.signature, url: entry.url };
    }
  }

  const missing = REQUIRED_PLATFORMS.filter((key) => !(key in platforms));
  if (missing.length > 0) {
    throw new Error(`no fragment covers ${missing.join(", ")}`);
  }

  return {
    version,
    pub_date: new Date().toISOString(),
    platforms,
  };
}

try {
  const manifest = merge(process.argv.slice(2));
  process.stdout.write(`${JSON.stringify(manifest, null, 2)}\n`);
} catch (error) {
  console.error(`merge-manifest: ${error.message}`);
  process.exit(1);
}
