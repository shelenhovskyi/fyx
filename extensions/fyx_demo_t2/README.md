# T2 capabilities demo (`fyx-demo-t2`)

A Fyx UI extension for checking the consent flows of the four T2 capabilities of the staged
`fyx:ui` 0.3 by hand ([design, sections 7 to 11](../../docs/fyx/wit-0.3/design.md)). One panel,
**T2 capabilities** (right dock), has a section for each:

| Section | Capability | What its button does |
|---|---|---|
| Fetch | `network:fetch`, domain `api.github.com` | `GET https://api.github.com/zen`, then shows the text or the error |
| Process | `process:spawn`, program `git` with `status --short` | runs it in the first worktree, then shows the output and exit code |
| Edit | `workspace:edit` | inserts a header line at the top of the file named in the input (`README.md`) as one undoable edit; nothing is saved |
| Agent reviews | `agent:observe` | `Start` lists the open agent reviews (file and state) live; `Stop` ends the feed |

Each section shows its permission state, read with `permissions.query` when the panel mounts
and with every event: not asked yet, allowed, denied, or not available here.
- **Request:** the section's **Request …** button (enabled while the state is `ask`) shows Fyx's
  consent prompt with `permissions.request`. The first use of the action shows it too.
- **While a prompt is open:** the section reads `waiting for an answer…`, then follows the
  `permission-changed` event Fyx sends.
- **Changing an answer:** a denied capability changes back only on the extensions page.

**Build it for 0.3 only.** It needs `fyx:ui` 0.3, which dev builds run:
`script/fyx-build-demos --fyx-ui-0-3` puts it in `<target>/fyx-demos-v0_3/fyx_demo_t2/`. The default
and 0.2 modes leave it out; a build without the `fyx-ui-0-3` feature only shows a panel that says
so.

Its headless tests use the harness's test broker and fake programs, so they reach no network and
run no process.
