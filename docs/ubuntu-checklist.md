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

## Second launch

Run on the default Wayland session, starting with Grapevine not running
(`pgrep -x grapevine` prints nothing).

1. Start Grapevine from the app grid and wait for the glyph. **Expect:** one
   glyph in the top panel and no popover.
2. Start Grapevine from the app grid again. **Expect:** still exactly one
   glyph, and the popover appears in the top-right corner of the primary
   monitor, where "Open Grapevine" puts it.
3. Run `pgrep -xc grapevine`. **Expect:** `1`.
4. Click on the desktop or in another window. **Expect:** the popover hides.
5. Run `grapevine; echo "exit $?"` in a terminal. **Expect:** the command
   returns by itself within a second or two and prints `exit 0`, the popover
   appears in the same corner, and there is still one glyph.
6. Hide the popover, click the glyph and choose "Open Grapevine". **Expect:**
   the popover shows; the tray menu still works after a second launch.
7. Choose "Quit Grapevine", then start Grapevine from the app grid.
   **Expect:** it starts normally with one glyph and no popover; the earlier
   copy left nothing behind that blocks a fresh start.

## Appearance

Run with a token saved and at least one repository watched, so the popover has
rows to show. `gsettings get org.gnome.desktop.interface font-name` prints the
desktop's interface font, "Ubuntu Sans" unless it has been changed. Take a
screenshot at each step marked **Screenshot** and attach it to the pull
request.

1. Run `xdg-settings get default-web-browser` and note the browser it names.
2. Open the popover. **Expect:** all text is set in the interface font, the
   same typeface as the top panel's clock, not DejaVu Sans or another fallback.
   **Screenshot.**
3. Scroll the whole list. **Expect:** no title, repository name, timestamp or
   "waiting" label is cut off mid-glyph or overlaps its neighbour; long ones
   end in an ellipsis. The red unread badge, the square org badge and the
   "bot" tag sit on the avatar's corners. Pills ("Draft", "CI failing",
   "Ready to merge" and the like) and the glyphs beside them share one line
   at the row's right edge. **Screenshot** of each screenful.
4. Click each section header in turn, twice. **Expect:** the section folds
   and unfolds, the chevron turns with it, and the count pill, plus the red
   unread pill on a folded section with unread PRs, stay on the header's line.
5. Move the pointer over a row in "Mine" or "Participated". **Expect:** the
   row tints blue and a moon appears at its right edge.
6. Click the moon. **Expect:** the row folds into a "1 snoozed" line at the
   foot of its section, level with the rows above it. Click the line, then
   the moon on the row it reveals, to wake the PR.
7. Rest the pointer on an avatar, then on a glyph or a "+1" pill at a row's
   right edge. **Expect:** each shows a dark bubble with a pointer after a
   short delay; the bubble stays inside the popover and its text is not cut
   off.
8. Type in "Filter" until nothing matches. **Expect:** "No matches" is centred
   in the popover. Press Escape. **Expect:** the field clears and the list
   returns.
9. Click a row. **Expect:** the PR opens in the browser from step 1.
10. Turn networking off and wait for the next sync. **Expect:** an amber
    message joins "Synced ..." on the line under the list, ending in an
    ellipsis if it does not fit. Turn networking back on.
11. Open Settings. **Expect:** the token field, the repository filter and
    checkboxes, the "Refresh pull requests every" menu, the "Launch at login"
    switch and "Check for updates" all draw completely, with their labels on
    the same line as the control. **Screenshot** of the top and of the bottom
    of the view.
12. Open the "Refresh pull requests every" menu and pick another value.
    **Expect:** the menu opens next to the control and the new value shows in
    it.
13. Choose "Create one on GitHub". **Expect:** GitHub's new token page opens
    in the browser from step 1.
14. Switch the desktop to the dark style (Settings, Appearance) and open the
    popover, then Settings. **Expect:** both views switch to the dark palette
    with no light-on-light or dark-on-dark text. **Screenshot** both.
15. Run `gsettings set org.gnome.desktop.interface font-name 'DejaVu Serif 11'`,
    quit Grapevine and start it again. **Expect:** the popover text is set in
    the serif font, like the top panel. Run
    `gsettings reset org.gnome.desktop.interface font-name` afterwards.
