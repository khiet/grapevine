---
name: verify
description: Build, launch, and drive the Grapevine menubar app to verify changes at its real surface (tray popover GUI).
---

# Verifying Grapevine

## Build & launch

```bash
npm run tauri dev     # dev loop; run in background, wait for `pgrep -x grapevine`
```

Quit with `pkill -x grapevine` (also ends the dev command). The popover window
is `main`, 360x440, hidden until the tray icon is clicked, and hides on focus
loss.

## Driving the popover

- AXPress on the tray item does NOT work — Tauri only reacts to real mouse
  events. Post a CGEvent click (small `swift` script) at the tray item's
  position: `tell process "grapevine" to get position of menu bar item 1 of
  menu bar 2`. The position moves between launches; resolve it every run.
- The webview's accessibility tree IS exposed (buttons, text fields under
  `UI element 1 of scroll area 1 of group 1 of group 1 of window 1`), so you
  can verify focus (`AXFocusedUIElement` role = `AXTextField`) before typing
  with `keystroke`. Pass secrets via env var, not argv.
- The popover hides whenever anything steals focus, so batch open→click→type→
  screenshot into ONE guarded shell script, checking window visibility before
  each step. Webview state survives hide/show — reopen and screenshot to read
  a result you missed.
- Screenshot region = window position + 360x440 via
  `screencapture -x -R "$X,$Y,360,440"`.

## Gotchas

- **User's live desktop.** This runs on the user's machine; if screenshots
  show them working (Finder, Chrome, dialogs moving), STOP driving and hand
  them manual verification steps instead. Their clicks also blur/hide the
  popover mid-flow.
- **Keychain prompts.** Dev builds are ad-hoc signed, so reading the token
  back after a rebuild triggers a macOS Keychain prompt. SecurityAgent
  blocks ALL synthetic input (secure input mode) while it is up — you cannot
  click Allow/Deny; only the user can. To test flows without the prompt,
  delete the app's entry first: `security delete-generic-password -s
  com.khietle.grapevine -a github-pat` (only if it holds test data).
- **Test credential.** `gh auth token` supplies a real GitHub token for
  success-path testing (validates as the user's login). Never echo it; clean
  up the Keychain copy afterwards or tell the user it's there.
- App state lives in `~/Library/Application Support/com.khietle.grapevine/settings.json`
  (must never contain the token) and the Keychain entry above.
