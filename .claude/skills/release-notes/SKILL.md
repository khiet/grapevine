---
name: release-notes
description: Rewrite the open release PR's CHANGELOG entries into user-facing notes by reading the actual diff since the last release.
---

# Polishing the release notes

release-please builds its changelog by concatenating commit subjects. Those are
written for the person who made the change, not the person deciding whether to
download the app. This rewrites them against the real diff.

Run it on the open release PR, immediately before merging. The release workflow
lifts the top section of `CHANGELOG.md` onto the GitHub release page, so the
changelog is the only file worth editing.

## Find the release PR

```sh
gh pr list --label "autorelease: pending" --json number,headRefName,title
```

If nothing comes back there is no release pending, so stop. Check out the head
branch and work there; every edit belongs on the PR branch, never on `main`.

## Read what actually shipped

```sh
git describe --tags --abbrev=0   # previous release; empty on the first one
git log --no-merges <prev>..main
git diff <prev>..main -- src src-tauri/src
```

Read the diff, not just the subjects. A subject like "fix: clear the tray count
with an empty title instead of None" describes the patch; the diff shows the
user-visible effect, which is that a stale unread count no longer sticks in the
menubar. Write the second one. On the first release there is no previous tag,
so diff from the root commit.

Only `feat`, `fix`, and `perf` reach the changelog; the config hides everything
else. If a hidden commit changed behaviour anyway, it was typed wrong at commit
time and belongs in the notes regardless.

## Write the entries

- Rewrite only the section for the version being released. Sections for past
  versions are already published; leave them exactly as they are.
- Keep release-please's `## [x.y.z](compare-link) (date)` heading byte for
  byte. The workflow matches on it to find the section, and the compare link
  is generated.
- One bullet per user-facing change. Say what it does for someone running the
  app. Drop bullets whose only effect is internal.
- Keep the trailing `([abc1234](commit-link))` reference on each bullet.
- Match the README's voice: plain ASCII, no em dashes, no smart quotes, no
  exclamation marks.
- Never describe behaviour you have not confirmed in the diff. If a change is
  unclear, read the code around it rather than guessing from the subject.

Then commit to the PR branch and push:

```sh
git commit -m "docs: rewrite the changelog for <version> as user-facing notes"
git push
```

Leave `package.json` and `.release-please-manifest.json` alone. Those are
release-please's, and editing them desynchronises its version state.

## Gotchas

- **release-please force-pushes this branch.** Any push to `main` makes it
  regenerate the release PR from scratch, discarding the rewrite. Run this
  last, right before merging. If anything lands on `main` afterwards, the work
  is gone and this has to run again.
- **The GitHub release notes do not come from the PR body.** release-please
  derives its own body, but the workflow overwrites the published notes with
  this changelog section plus the install footer. Editing the PR body achieves
  nothing.
- **An unmatched heading silently falls back.** If the `## [x.y.z]` heading
  gets reformatted, the workflow cannot find the section and publishes
  release-please's generated notes instead, with a warning in the job log.
- Install instructions are appended by the workflow from
  `.github/release-footer.md`. Do not repeat them in the changelog.
