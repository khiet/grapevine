# Ubuntu manual checklist

Checks with no automated seam, run by hand on the Ubuntu test machine. The
machine is install-only: download the `grapevine-deb-x86_64` artifact from the
pull request's CI run and install it, never compile there.

```sh
sudo apt-get install ./grapevine_*.deb
```

Run every step in a section in order; later steps depend on earlier ones.

## Launch at login

The switch is under Settings, "Launch at login". The entry it manages is
`~/.config/autostart/grapevine.desktop`.

1. Remove any entry left by an earlier run: `rm -f ~/.config/autostart/grapevine.desktop`.
2. Start Grapevine and open Settings. **Expect:** "Launch at login" is off.
3. Turn the switch on. **Expect:** it stays on and no error appears.
4. Run `cat ~/.config/autostart/grapevine.desktop`. **Expect:** the file exists
   and its `Exec=` line is `/usr/bin/grapevine`.
5. Quit Grapevine, start it again and open Settings. **Expect:** the switch is
   still on.
6. Log out and log back in. **Expect:** the Grapevine tray icon appears without
   starting it by hand, and only one icon is shown.
7. Open Settings and turn the switch off. **Expect:** it stays off.
8. Run `ls ~/.config/autostart/grapevine.desktop`. **Expect:** "No such file or
   directory".
9. Log out and log back in. **Expect:** no Grapevine tray icon.
10. Start Grapevine by hand and open Settings. **Expect:** the switch is off.

## Tray and popover

Run on the default Wayland session (the login screen's gear menu shows
"Ubuntu", not "Ubuntu on Xorg") with at least one PR waiting on you, so the
tray has a count to show. Steps 13 and 14 need `xlsclients`, from the
`x11-utils` package.

1. Start Grapevine from the app grid. **Expect:** a white grape glyph appears
   in the top panel and is easy to read against it.
2. Wait for the first sync. **Expect:** the count of PRs waiting on you shows
   as a label next to the glyph.
3. Click the glyph. **Expect:** a menu opens with "Open Grapevine" above "Quit
   Grapevine" and nothing else.
4. Choose "Open Grapevine". **Expect:** the popover appears in the top-right
   corner of the primary monitor, just below the panel, with a small gap to
   the panel and to the right edge. It does not overlap the panel.
5. Hide the popover (step 7), move another app's window so it covers the
   top-right corner, and choose "Open Grapevine" again. **Expect:** the
   popover draws above that window.
6. With the popover open, look at the dock and press Super+Tab, then Alt+Tab.
   **Expect:** Grapevine is in none of them.
7. Click on the desktop or in another window. **Expect:** the popover hides.
8. Open the popover again, then click the glyph and choose "Open Grapevine".
   **Expect:** the popover ends up hidden, not hidden and shown again.
9. Choose "Open Grapevine" once more. **Expect:** the popover shows in the
   same corner as in step 4.
10. With a second monitor attached, open the popover while the pointer is on
    the non-primary monitor. **Expect:** it still appears on the primary one.
11. Snooze or deal with every PR waiting on you, then wait for the next sync.
    **Expect:** the label next to the glyph disappears, leaving no gap or
    stray "0".
12. Choose "Quit Grapevine" from the tray menu. **Expect:** the glyph leaves
    the panel and `pgrep grapevine` prints nothing.
13. Run `GDK_BACKEND=wayland grapevine` from a terminal and run
    `xlsclients | grep -i grapevine` in another. **Expect:** no output; the
    backend you chose was kept. The popover may open in the wrong place in
    this mode, which is accepted. Quit it.
14. Run `grapevine` with no variable set and repeat the `xlsclients` command.
    **Expect:** a `grapevine` line; the app is running through XWayland.
