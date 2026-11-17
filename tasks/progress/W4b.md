# W4b progress: gallery demo

Branch `claude/fyx-w4b-gallery`, from `origin/integration` (`git switch -c claude/fyx-w4b-gallery
origin/integration`).

## State: done (2026-11-17)

| Item | State |
|---|---|
| `extensions/fyx_demo_gallery` (panel `gallery`, status item `gallery-status`) | done |
| `cargo build -p fyx_demo_gallery --target wasm32-wasip2` | builds (debug, ~4.4 MB) |
| `wasm-tools component wit` (wasm-tools 1.263.0) | 4 `ui-extension` exports; imports `fyx:ui/host` with `log` **and** `push-update`; `types` identical to the frozen WIT (sorted lines compared); one `fyx:ui-api-version` section |
| Tests on the host target | 10 pass: `cargo test -p fyx_demo_gallery` (with counter, todo and SDK: 32 pass) |
| clippy `--deny warnings` | clean on the host target (`--all-targets`) and on `wasm32-wasip2` |
| `script/fyx-build-demos` | builds and copies all three demos; prints three directories |

Outside `extensions/fyx_demo_gallery/`: its workspace member line in `Cargo.toml` (`# fyx`), the
new `fyx_demo_gallery` entry in `Cargo.lock` (no other change), the gallery in
`script/fyx-build-demos`. The crate has `ui_ext` as a dev-dependency only, so the tests apply every
update to a real `ui_ext::Tree` with the host's rules. No WIT or SDK change. `README.md` got the
two review lines that root `CLAUDE.md` requires.

Not checked: loading the gallery in the dev build (this VM does not build `zed`). That is W3's
check, and this contract is what it should see.

## Loading it

```
script/fyx-build-demos
```

then add `<repo>\extensions\fyx_demo_gallery` to `FYX_UI_DEV_EXTENSIONS`. The manifest has
`capabilities = ["ui:panel", "ui:status-bar"]`. `fyx_ui::UiManifest::check_surfaces` needs both
because of the status item. Extension id `fyx-demo-gallery`.

## Design decision: one section at a time

The host panel does not scroll (`SurfaceView` is a plain `v_flex`), and all sections together are
far taller than a dock panel, so the lower ones would be clipped. The panel therefore shows one
section, and nine navigation buttons switch between them. That also exercises `remove` + `insert`
patches, and dropping the rows of a removed list or table.

## Surface contract

Pinned by `behaves_as_documented` and the other tests in
`extensions/fyx_demo_gallery/src/fyx_demo_gallery.rs`. Each surface has its own id space from 1.
As in the counter, the SDK id allocator survives remounts, so a remount gets new ids. All ids
below are from the **first** mount.

### Surfaces

| id | kind | details |
|---|---|---|
| `gallery` | panel | right dock, title `Gallery`, icon `sparkle`, default size 420 |
| `gallery-status` | status item | right side, priority 0, title `Gallery selection` |

### `mount("gallery")`: the frame

68 nodes, parents first:

| id | parent | props |
|---|---|---|
| 1 | none | column flex, gap 8, padding 8, grow, align stretch |
| 2 | 1 | text `fyx:ui gallery`, bold |
| 3 | 1 | row flex, gap 4 |
| 4–8 | 3 | buttons `Layout`, `Text`, `Buttons`, `Input`, `List` |
| 9 | 1 | row flex, gap 4 |
| 10–13 | 9 | buttons `Table`, `Icons`, `Markdown`, `Diff` |
| 14 | 1 | section slot: column flex, gap 8, grow, align stretch |
| 15–68 | 14… | the Layout section (below) |

The shown section's navigation button is `filled`, the others `subtle`, none disabled and without
icons.

### Switching sections

`click(<nav button of another section>)` returns `patches`, in this order:

1. `set-props` on the old section's button (now `subtle`),
2. `set-props` on the new section's button (now `filled`),
3. `remove(<old section root>)`,
4. one `insert` per node of the new section, parents first: the root under 14, the rest under
   their parents. `before` is always `none`.

The new section always gets new ids. Clicking the shown section's button returns `unchanged`.
Example from a fresh mount, `click(8)` (List):
`set 4, set 8, remove 15, insert 69 under 14, insert 70–73 under 69`.

### Sections

Every section root is a column flex (gap 8, grow, align stretch). Its first child is a bold
heading text. Muted captions are text with `text-muted`. "From a fresh mount" means the ids you
get by clicking that section's button right after the first mount. They all start at 69.

| Section | Contents (in order) | Nodes, from a fresh mount |
|---|---|---|
| Layout | heading `Flex layouts`; row (gap 4, padding 4, bg `surface`) of 3 swatches `A` `B` `C`; column (gap 8, padding 8, bg `elevated-surface`) with `First`, `Second`; 4 columns (padding 4, bg `surface`) with align `start`/`center`/`end`/`stretch`, each holding a swatch `align <name>`; row (gap 4) `fixed` swatch + grow row (padding 4, bg `border`) `grow` + `fixed` swatch; one row (padding 4) per colour token as background, holding the token name. A swatch is a row flex (padding 4, bg `elevated-surface`) with one text. | 15–68 (first mount) |
| Text | heading `Text`; caption; `no colour (the default)`; one text per text token, coloured with it and naming it (`text`, `text-muted`, `text-accent`, `error`, `warning`, `success`, `border`); then `surface` and `elevated-surface`, which are invisible as text colour by design, each as a row (padding 4, bg the token) holding a default-colour text `<token> (background)`; caption; `bold`; `monospace`; `bold monospace` | 69–87 |
| Buttons | heading; three groups, each a caption plus a row (gap 4, align center) of 4 buttons: **Styles** `Default`/`Filled`/`Subtle`/`Danger` (73–76); **Disabled**, the same four disabled (79–82); **With icon** `Add` (`plus`, filled), `Search` (`magnifying_glass`, default), `Delete` (`trash`, danger), icon-only (empty label, `settings`, subtle) (85–88); then text `Last clicked: none` (muted) (89) | 69–89 |
| Input | heading; caption; input (value `""`, placeholder `Type and press Enter`) (72); text `Submitted: nothing yet` (muted) (73) | 69–73 |
| List | heading `Virtualized list`; caption; text `Selected: none` (muted) (72); list (`row-count` 1000, `row-height` 24) (73) | 69–73 |
| Table | heading; caption; table (columns `Name`, `Kind`, `Lines`; `row-count` 100) (72) | 69–72 |
| Icons | heading; caption; 3 rows (gap 12) of 4 cells; caption; 1 row with one cell. A cell is a column (gap 2, padding 4, align center) holding an icon and a muted text with its name. Known names: `check` (`success`), `close` (`error`), `warning` (`warning`), `info` (`text-accent`), `plus`, `dash`, `folder`, `file`, `settings`, `magnifying_glass`, `trash`, `pencil` (no colour). Unknown: `fyx_no_such_icon`, which should render as the host fallback (`Blocks`). `blocks` itself is deliberately not used. | 69–115 |
| Markdown | heading; one markdown node: H1, paragraph with bold/italic/inline code/link, bullet list, numbered list, block quote, fenced `rust` block | 69–71 |
| Diff | heading `Diff view`; caption `Rust`; diff-view (Rust before/after, `language: "rust"`) (72); caption `Plain text, no language`; diff-view (`language: none`) (74) | 69–74 |

### Events on `gallery`

Every event returns `patches` with only `set-props`, or `unchanged`. The only exception is section
switching (above). There is never a `replace`.

| Event | Result |
|---|---|
| `click(<Buttons section button>)` | `set-props` on `Last clicked` (89): `Last clicked: <name>`. Names are the labels; the icon-only one is `Icon only (settings)`. A click on a **disabled** button reads `Last clicked: <name> (disabled: the host should not have sent this)`, so a host bug shows on screen. Repeating the same click returns `unchanged`. |
| `input-changed(<input>, v)` | `unchanged`. The value is recorded with `set_props_from_host` (no echo), so it survives leaving and re-showing the section. |
| `input-submitted(<input>, v)` | one `set-props` on `Submitted` (73): `Submitted: v`, or `Submitted: (empty)` for `""`. The input itself is **not** patched: its text stays the user's. |
| `row-click(<list>, i)`, `i < 1000` | one `set-props` on `Selected` (72): `Selected: Row {i + 1}`, plus a **push** to `gallery-status` (below). The same row again returns `unchanged` and pushes nothing. |
| `row-click(<list>, i ≥ 1000)` | `unchanged` |
| `row-click(<table>, _)` | `unchanged`: the table is display only |
| anything else, or ids that are not shown (old sections, earlier mounts) | `unchanged` |
| unknown surface id | error |

State that survives section switches and remounts: the shown section, the last click, the typed
input text, the last submission, the selected row.

### `list-rows`

Only for the list or table of the **shown** section. Every other request, including the ids of a
removed list, is answered with an empty list and a `warn` log. Ranges are clamped to the row count.
Rows use row-local ids from `0x8000_0000`, as in the other demos.

| Node | Row `i` |
|---|---|
| list | row flex (gap 8, align center): icon `file`, text `Row {i + 1}`, text `index {i}` (muted, monospace). 4 nodes. |
| table | row flex with exactly 3 text children, one per column, as `view.rs` expects for cells: `file_{i + 1:03}.rs`, kind `source`/`test`/`doc` by `i % 3` (muted), lines `(i * 37) % 500 + 10` (monospace). Row 0 is `file_001.rs`, `source`, `10`; row 1 is `file_002.rs`, `test`, `47`. |

### `gallery-status`

`mount("gallery-status")` returns 3 nodes:

| id | parent | props |
|---|---|---|
| 1 | none | row flex, gap 4, align center |
| 2 | 1 | icon `list_tree`, `text-muted` |
| 3 | 1 | text `Gallery: none` (muted), or `Gallery: Row {n}` if a row is already selected |

A `row-click` on the gallery list that changes the selection calls
`push-update("gallery-status", patches [set-props(3, text "Gallery: Row {i + 1}")])` during that
`handle-event` call. Per integrator decision 3, the host should route it to the status item through
the registry. Nothing is pushed before the status item is mounted. A late mount shows the current
selection. `handle-event` on `gallery-status` returns `unchanged` (error before it is mounted).

## Notes for W3

- **What to look at, section by section:** Layout (gap/padding/align/grow/backgrounds), Text (7
  text colours, the 2 surface tokens as backgrounds, bold, monospace), Buttons (4 styles, disabled, start icon, icon-only
  `IconButton`), Input (submit echo, no per-keystroke echo), List (1,000 rows, scrolling, row
  click, status item update), Table (header plus equal-width cells), Icons (12 names plus fallback),
  Markdown, Diff (the placeholder, with and without a language).
- **Section switches remove a list or table.** That exercises "removed lists lose their cache"
  (integrator decision 5) and the `uniform_list` scroll handle of a node that goes away.
- **A push for another surface during `handle-event`.** This is the first demo that does it
  (decision 3). If the status item does not update on a row click, check the routing first.
- **The panel cannot scroll.** That is why the gallery shows one section at a time. A later
  `fyx:ui` version could add an `overflow`/scroll flag on flex, so long trees do not get clipped.

## Tests (`extensions/fyx_demo_gallery/src/fyx_demo_gallery.rs`)

Every update goes through a harness that applies it to a real `ui_ext::Tree` (the host's own rules
and caps). The harness also requires that the tree then equals the extension's mirror, and that
no update is a `replace`. For inputs it first writes the typed value into the host tree, as the host
does.

- `behaves_as_documented`: surfaces, frame ids 1–15, 68 nodes, nav labels and styles, the List
  switch patch sequence (69–73), rows 998–999, row click, unknown surface.
- `every_section_shows_and_patches_cleanly`: all 72 ordered section pairs. Checks the patch
  prefix and that one section stays under 14. Over all sections, all nine primitive kinds appear.
- `sections_cover_their_primitives`: the kinds per section; the 7 text tokens as text colour and
  `surface`/`elevated-surface` as backgrounds with default-colour labels in Text; all 9 tokens as
  background in Layout.
- `buttons_report_the_last_click`, `input_echoes_only_on_submit`,
  `row_click_updates_the_label_and_the_status_item`, `table_rows_fill_every_column`,
  `list_rows_only_for_the_shown_list`, `remount_keeps_the_shown_section_with_new_ids`.
- `icons_are_zed_icon_names_except_the_unknown_one`: every icon name the gallery uses, and the
  unknown one, is checked against the `IconName` variants in `crates/icons/src/icons.rs`. That
  file is read with `include_str!` instead of depending on the GPL crate.
