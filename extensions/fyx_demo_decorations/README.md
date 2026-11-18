# TODO highlights demo (`fyx-demo-decorations`)

A Fyx UI extension that places an editor decoration, a block above the line, on every `TODO`,
`FIXME` and `HACK` comment it finds. It shows `fyx:ui/decorations` from `fyx:ui` 0.2
([design, section L](../../docs/fyx/wit-0.2/design.md)).

The left-dock panel **TODO highlights** has:

- a summary (`N comments in F files, H highlighted`);
- **Highlight all** and **Clear**;
- a status line that shows the host's error when a call is refused;
- one row per comment (`path:line` and the text). A row click places or removes that comment's
  block.

Each block shows the kind and the comment, and a **Done** button that removes it.

**The files are mocked.** Extensions cannot read the worktree, so the comments come from the same
four fixed files as the TODO demo: `src/main.rs`, `src/render.rs`, `web/app.ts` and
`docs/README.md`. Open a project with files at those paths to see the blocks.

**Decorations need `fyx:ui` 0.2.** The default build exports 0.1, which Fyx runs today. There the
panel works, but placing a block reports `unavailable`. Build the 0.2 component with
`script/fyx-build-demos --fyx-ui-0-2`.

How to write your own: [docs/fyx/extension-authoring.md](../../docs/fyx/extension-authoring.md).
