## Install

### macOS

Download the `Grapevine-v<version>-universal.zip` asset below, unzip it, and drag
`Grapevine.app` to `/Applications`.

The app is signed with an Apple Developer ID and notarized by Apple, so it opens
normally with no Gatekeeper warning. It runs natively on both Apple silicon and
Intel Macs.

### Ubuntu

Download the `Grapevine-v<version>-amd64.deb` asset below and install it from
the folder you downloaded it to:

```sh
sudo apt-get install ./Grapevine-v<version>-amd64.deb
```

Start Grapevine from the app grid. Clicking the icon in the top panel opens a
menu: "Open Grapevine" toggles the popover and "Quit Grapevine" stops the app.
Current Ubuntu on x86_64 is the only supported Linux target.

### Updates

Installed apps update themselves, on macOS from v0.6.0 onward: Grapevine checks
for new releases in the background and offers a one-click restart when one is
ready, so this manual download is only needed the first time. On Ubuntu, the
restart asks for your password, because installing a `.deb` needs administrator
rights.
