# Grapevine

A minimal macOS menubar GitHub PR watcher. Grapevine lives in the menubar only
(no Dock icon): clicking the tray icon toggles a popover with the PR list, and
clicking away dismisses it. It replaces the slice of
[Trailer](https://github.com/ptsochantaris/trailer) actually in use.

## Stack

- [Tauri v2](https://tauri.app) shell; hand-written Rust is limited to tray
  setup and popover window toggling (`src-tauri/src/lib.rs`)
- React + TypeScript + Vite frontend in the popover window

## Development

```sh
npm install
npm run tauri dev
```

## Production build

```sh
npm run tauri build
```

The runnable app lands in `src-tauri/target/release/bundle/macos/Grapevine.app`.
