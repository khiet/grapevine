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
