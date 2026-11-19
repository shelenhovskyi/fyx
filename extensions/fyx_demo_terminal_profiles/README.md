# Terminal profiles demo (`fyx-demo-terminal-profiles`)

A Fyx UI extension for the staged `fyx:ui` 0.4 whose manifest adds two entries to the terminal
panel's **+** menu ([design, sections B.1 and B.6](../../docs/fyx/wit-0.4/design.md)):

- **Git history — Terminal profiles demo** runs `git log --graph --oneline --decorate -n 200`
  in the active worktree.
- **Python with a welcome — Terminal profiles demo** runs
  `python -i ${package}/scripts/welcome.py` in the home folder. Fyx puts the extension's
  installed folder in place of `${package}` (`package:files`), so the script ships with the
  extension; `fyx-ext package` packs the folders `${package}/...` names.

Both entries are marked "runs any command you type": Python runs what you type, and git's pager
runs `!command`. The terminals belong to the user: the extension cannot read or type into them.
It needs `ui:panel`, `terminal:profiles` (T1) and `package:files` (T0). Its one panel,
**Terminal profiles** (right dock), lists the profiles.

## Build and run

```sh
script/fyx-build-demos --fyx-ui-0-4
```

builds it with the other demos into `<target>/fyx-demos-v0_4/`. `fyx_ext_harness` checks the
profiles and starts fake profile terminals headless (`tests/demos_v0_4.rs`). Fyx itself shows
the profiles once its host has terminal profiles for extensions (plan 5.8).
