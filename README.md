<!-- LOGO -->
<h1>
<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" alt="Grapevine" width="128">
  <br>Grapevine
</h1>
  <p align="center">
    A minimal macOS menubar watcher for GitHub pull requests.
    <br />
    Add the repos you care about, watch their PRs. That is the whole app.
    <br />
    <a href="#about">About</a>
    ·
    <a href="#building">Building</a>
    ·
    <a href="#contributing">Contributing</a>
  </p>
</p>

## About

Grapevine lives in the menubar only, with no Dock icon. Clicking the tray icon
toggles a popover with the PR list, and clicking away dismisses it. You give it
a GitHub token and a list of repos; it polls them every few minutes and shows
the open PRs, split into the ones you authored, the ones you are involved in,
and everything else, with a badge for comments and reviews you have not seen
yet. Merged PRs get their own section until you dismiss them.

Grapevine is inspired by [Trailer](https://github.com/ptsochantaris/trailer),
which I used for years and still recommend. It is not a fork or a port, and
shares no code with it: Trailer is Swift and AppKit, Grapevine is Tauri, Rust
and React. Grapevine only reimplements the one slice of Trailer I reached for
daily. If you want issues as well as PRs, filtering, notification rules, GitHub
Enterprise, or any of this on a phone, Trailer already does all of that well
and you should use it instead.

### Non-goals

Grapevine deliberately does not, and will not:

- Track issues. Pull requests only.
- Filter, label, sort, or group beyond the three built-in sections.
- Send system notifications. The badge in the menubar is the whole signal.
- Support GitHub Enterprise or anything other than github.com.
- Handle more than one GitHub account at a time.
- Run anywhere other than a Mac menubar. No iOS, no Android, no CLI.
- Ship on the App Store.

## Building

There are no prebuilt downloads. Grapevine is built from source.

### Stack

- [Tauri v2](https://tauri.app) shell; hand-written Rust is limited to tray
  setup and popover window toggling (`src-tauri/src/lib.rs`)
- React + TypeScript + Vite frontend in the popover window

### Prerequisites

- macOS with Xcode Command Line Tools (`xcode-select --install`)
- [mise](https://mise.jdx.dev), which installs the pinned Node and Rust
  toolchains:

```sh
mise install
```

### Development

```sh
npm install
npm run tauri dev
```

### Production build

```sh
npm run tauri build
```

The runnable app lands in `src-tauri/target/release/bundle/macos/Grapevine.app`.

## Setup

Once the app is running, open the popover and go to settings.

1. Create a [personal access token
   (classic)](https://github.com/settings/tokens) on GitHub. Grant it `repo` if
   you want to watch private repos, or just `public_repo` if you only watch
   public ones. `repo` is a broad scope that carries read and write access to
   all of your private repositories; `public_repo` is the tighter choice when
   it is enough for you.
2. Paste the token into settings. Grapevine stores it in the macOS Keychain and
   never writes it to `settings.json`.
3. Add repos as `owner/name`, one at a time.

Fine-grained tokens are not recommended: they are scoped to a single resource
owner, and Grapevine holds one token, so a fine-grained token can only ever
watch repos belonging to one user or organization.

## Contributing

Bug reports and bug-fix pull requests are welcome.

For anything that adds behaviour, open an issue before writing code. Anything
on the Non-goals list above is a no by default, and the bar for a new feature
is high on purpose: staying small is the point of this app, not an accident of
it being young.

## License

[MIT](LICENSE).
