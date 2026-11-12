<!-- LOGO -->
<h1>
<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" alt="Grapevine" width="128">
  <br>Grapevine
</h1>
  <p align="center">
    Know which pull requests are waiting on you, without opening GitHub.
    <br />
    A small menubar app for macOS and Ubuntu.
    <br />
    <a href="#what-it-shows-you">Features</a>
    ·
    <a href="#install">Install</a>
    ·
    <a href="#setup">Setup</a>
    ·
    <a href="#development">Development</a>
  </p>
</p>

![Grapevine demo: the menubar icon opens a popover of fictional pull requests, and nine labels land on it one at a time: the tray number is the PRs waiting on you, your turn comes first with how long it has waited, only required checks count as failing, ready to merge, merge queue position, your team was asked to review, the filter across every repo, unread from people and never bots, and snooze until it changes. The loop rests on all nine, with macOS and Ubuntu badges under the tagline.](docs/assets/grapevine.gif)

## How it works

1. Give Grapevine a GitHub token and the repos you care about.
2. It checks them every few minutes.
3. The number in your menubar is how many PRs are waiting on you. Click it to
   see the list.

No notifications, no dashboard to keep open, and on macOS no Dock icon.

## What it shows you

- **Your turn first.** PRs where the next move is yours sit at the top, each
  with how long it has been waiting on you.
- **What's blocking a PR.** A pill names the problem: merge conflict, CI
  failing, changes requested, unresolved threads, or behind base. Only
  required checks count as failing.
- **What's ready.** A green check once required approvals are in, and the
  position of a queued PR in the merge queue.
- **Team requests.** PRs where your team, not just you, was asked to review.
- **Unread activity.** New comments and reviews from people since you last
  looked. Bots never badge.
- **Three sections.** Mine (you opened it), Participated (you are involved),
  and All (everything else, quiet and browsable). Merged PRs stay in their own
  section until you dismiss them.
- **Filter and snooze.** Type to filter across every repo. Snooze a PR until
  it changes.

## Install

### macOS

1. Download `Grapevine-v<version>-universal.zip` from the latest
   [release](https://github.com/khiet/grapevine/releases).
2. Unzip it and drag `Grapevine.app` to `/Applications`.

Runs on Apple silicon and Intel. Releases are signed and notarized, so the
first launch opens with no Gatekeeper warning.

### Ubuntu

Supported on current Ubuntu, x86_64. Other distributions and architectures
may work, but are not tested.

1. Download `Grapevine-v<version>-amd64.deb` from the latest
   [release](https://github.com/khiet/grapevine/releases).
2. Install it from the folder you downloaded it to:

   ```sh
   sudo apt-get install ./Grapevine-v<version>-amd64.deb
   ```

3. Start Grapevine from the app grid. A grape icon appears in the top panel.

Ubuntu's tray does not report clicks on the icon, so it works a little
differently from macOS:

- Clicking the icon opens a menu. "Open Grapevine" toggles the popover, and
  "Quit Grapevine" stops the app.
- To open the popover in one step, click Grapevine in the dock or switch to
  it with Super+Tab or Alt+Tab. It is in both for as long as it runs.
- The popover opens in the top-right corner of the primary monitor, not under
  the icon.

### Updates

Grapevine downloads new releases in the background and offers a restart when
one is ready. You only download it by hand the first time.

On Ubuntu, the restart asks for your password, because installing a `.deb`
needs administrator rights. Nothing installs until you restart, and
cancelling the prompt keeps the current version running.

## Setup

Open the popover and go to settings.

1. **Create a token.** The "Create one on GitHub" link opens
   [the new-token page](https://github.com/settings/tokens/new?scopes=repo&description=Grapevine)
   for a classic personal access token, with `repo` pre-ticked.
2. **Paste it into settings.** It is stored in the macOS Keychain or the
   Ubuntu system keyring, never in `settings.json`.
3. **Add repos** as `owner/name`, one at a time.

About token scopes:

- `repo` is needed for private repos. It grants read *and write* access to
  all of them, but Grapevine only ever reads.
- If you only watch public repos, `public_repo` is the tighter choice.
  Grapevine tells you when a saved token cannot see private repos.
- Fine-grained tokens are not recommended. Each one covers a single user or
  organization, and Grapevine holds only one token.

## What it won't do

Grapevine stays small on purpose. It does not, and will not:

- Track issues. Pull requests only.
- Offer saved views or custom filters.
- Send system notifications. The menubar number is the whole signal.
- Support GitHub Enterprise, or more than one GitHub account.
- Run anywhere other than a Mac menubar or an Ubuntu tray. No Windows, mobile,
  or CLI.
- Ship on the App Store.

## Development

Grapevine is built with [Tauri v2](https://tauri.app) and a React +
TypeScript + Vite frontend. Hand-written Rust is limited to tray setup and
popover window toggling (`src-tauri/src/lib.rs`).

### Prerequisites

- macOS with Xcode Command Line Tools (`xcode-select --install`), or Ubuntu
  with these system packages:

  ```sh
  sudo apt-get install build-essential pkg-config file \
    libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
    libxdo-dev libssl-dev libdbus-1-dev
  ```

- [mise](https://mise.jdx.dev), which installs the pinned Node and Rust
  toolchains:

  ```sh
  mise install
  ```

### Run and test

```sh
npm install
npm run tauri dev  # run the app
npm test           # frontend tests
npm run test:rust  # Rust tests
```

What the suites cannot reach on Ubuntu is verified by hand with the
[Ubuntu manual checklist](docs/ubuntu-checklist.md).

### Production build

```sh
npm run tauri build                  # macOS: bundle/macos/Grapevine.app
npm run tauri build -- --bundles deb # Ubuntu: bundle/deb/
```

Both land under `src-tauri/target/release/`. CI also builds an x86_64 `.deb`
for every pull request as the `grapevine-deb-x86_64` workflow artifact. It is
for testing only: it carries no updater signature and is not a release.

### Releasing

Releases are automated with
[release-please](https://github.com/googleapis/release-please). Merging a
Conventional Commit to `main` updates a standing release PR that bumps the
version in `package.json` and writes `CHANGELOG.md`. Merging *that* PR tags
the release, builds the macOS app and the Ubuntu `.deb` onto it, and publishes
the updater manifest once both have built. `package.json` is the only place
the app version lives, so never bump a version by hand.

## Contributing

Bug reports and bug-fix pull requests are welcome.

For anything that adds behaviour, open an issue before writing code. Anything
on the [What it won't do](#what-it-wont-do) list is a no by default, and the
bar for a new feature is high on purpose: staying small is the point of this
app.

## Credits

Inspired by [Trailer](https://github.com/ptsochantaris/trailer). Grapevine
shares no code with it.

## License

[MIT](LICENSE).
