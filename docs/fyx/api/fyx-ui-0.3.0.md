# `fyx:ui@0.3.0` WIT reference

Generated from `crates/ui_ext/wit/since_v0.3.0` by `script/fyx-api-docs`; do not edit by hand. A test fails when this page and its source disagree.

## World `ui-extension`

Imports:

- interface `fyx:ui/types@0.3.0`
- interface `fyx:ui/host@0.3.0`
- interface `fyx:ui/documents@0.3.0`
- interface `fyx:ui/decorations@0.3.0`
- interface `fyx:ui/notifications@0.3.0`
- interface `fyx:ui/language@0.3.0`
- interface `fyx:ui/virtual-documents@0.3.0`
- interface `fyx:ui/picker@0.3.0`
- interface `fyx:ui/images@0.3.0`
- interface `fyx:ui/permissions@0.3.0`
- interface `fyx:ui/network@0.3.0`
- interface `fyx:ui/process@0.3.0`
- interface `fyx:ui/workspace-edit@0.3.0`
- interface `fyx:ui/agent-observe@0.3.0`
- interface `fyx:ui/io@0.3.0`
- interface `fyx:ui/requests@0.3.0`
- type `surface`
- type `node`
- type `node-id`
- type `ui-event`
- type `update`
- type `list-row`
- type `io-event`
- type `decoration`
- type `decoration-request`
- type `hover-request`
- type `hover-reply`
- type `code-lens-request`
- type `code-lens`
- type `command-context`
- type `picker-source-event`
- type `virtual-document`

Exports:

- `surfaces: func() -> list<surface>`: ---- 0.2, unchanged ---- Called once after instantiation; declares the static surfaces. More can be added with `add-surface`.
- `mount: func(surface-id: string) -> result<list<node>, string>`: The whole tree of a surface (at most 16384 nodes): when a panel or status item is added, each time a modal is shown, and again whenever the host resyncs after an update that did not fit, which for a decoration replaces the tree it was placed with. Never for a picker.
- `handle-event: func(surface-id: string, event: ui-event) -> result<update, string>`: At most one call per input event; the host debounces scroll, hover and typing.
- `list-rows: func(surface-id: string, list-node: node-id, start: u32, count: u32) -> list<list-row>`
- `tick: func(surface-id: string) -> result<update, string>`: The tick `request-tick` asked for. Never called while the surface is mounting or failed, nor for a modal that is not shown. The update applies to `surface-id`, like an event's.
- `handle-io: func(event: io-event) -> result<_, string>`: ---- 0.3 ---- The outcome of a fetch, a process, an edit, an agent-review event or a permission change. Not tied to a surface: push to surfaces with `push-update`.
- `handle-command: func(command: string, argument: option<string>, context: command-context) -> result<_, string>`: One of the manifest's `[[fyx_ui.commands]]` ran (palette, keymap, code lens, gutter sign). The call has the click-or-Enter activation.
- `provide-decorations: func(request: decoration-request) -> result<list<decoration>, string>`: Decorations of a pull namespace for `request.lines`; they replace the namespace's decorations in those lines. At most 10000.
- `provide-hover: func(request: hover-request) -> result<hover-reply, string>`
- `provide-code-lenses: func(request: code-lens-request) -> result<list<code-lens>, string>`: At most 256 lenses, 8 per line.
- `provide-virtual-document: func(path: string) -> result<virtual-document, string>`: The text of one of this extension's virtual documents, when its tab opens, after `invalidate-virtual-document`, and after a reload of the extension.
- `picker-source-event: func(source: string, session: u64, event: picker-source-event) -> result<_, string>`: Events of a picker session for one source of this extension (`source` as in `picker.set-items`). The owner gets `dismissed` through `handle-event` as for any modal.

## Interface `types`

### type `node-id` = `u32`

Chosen by the extension. In a surface tree (`mount`, `update`) an id is unique within the surface and must not be reused for a different node while the surface lives. In a `list-row` subtree an id is row-local: unique within that row only, and every row may use the same ids. The host never routes events to row-local ids; a click anywhere in a row is `row-click`.

### enum `color-token`

Colours are semantic theme slots, never raw values, so every extension follows the active theme.

Cases: `text`, `text-muted`, `text-accent`, `error`, `warning`, `success`, `border`, `surface`, `elevated-surface`.

### enum `icon`

Icons the host promises to draw. Frozen per world version: the host maps each case to its own icon set, so upstream renames never break an extension. New icons need a new world version.

Cases: `arrow-up`, `arrow-down`, `arrow-left`, `arrow-right`, `chevron-up`, `chevron-down`, `chevron-left`, `chevron-right`, `expand`, `collapse`, `menu`, `ellipsis`, `list-tree`, `list-todo`, `table`, `file-tree`, `plus`, `dash`, `check`, `check-double`, `close`, `search`, `filter`, `refresh`, `undo`, `copy`, `pencil`, `trash`, `replace`, `download`, `send`, `share`, `link`, `pin`, `unpin`, `play`, `pause`, `stop`, `maximize`, `minimize`, `split`, `file`, `file-code`, `file-diff`, `file-text`, `folder`, `folder-open`, `code`, `terminal`, `diff`, `regex`, `case-sensitive`, `whole-word`, `hash`, `at-sign`, `git-branch`, `git-commit`, `pull-request`, `info`, `warning`, `error`, `help`, `bell`, `bell-dot`, `circle`, `dot`, `star`, `star-filled`, `bookmark`, `thumbs-up`, `thumbs-down`, `todo-pending`, `todo-progress`, `todo-complete`, `spinner`, `blank`, `settings`, `clock`, `debug`, `eye`, `eye-off`, `lock`, `unlock`, `person`, `people`, `chat`, `envelope`, `book`, `notepad`, `image`, `globe`, `server`, `database`, `keyboard`, `box`, `archive`, `sparkle`, `bolt`, `flame`, `power`, `blocks`.

### enum `flex-direction`

Cases: `row`, `column`.

### enum `align`

Cases: `start`, `center`, `end`, `stretch`.

### record `flex-props`

| Field | Type | Notes |
|---|---|---|
| `direction` | `flex-direction` |  |
| `gap` | `u16` |  |
| `padding` | `u16` |  |
| `grow` | `bool` |  |
| `align` | `align` |  |
| `background` | `option<color-token>` |  |

### record `text-props`

| Field | Type | Notes |
|---|---|---|
| `content` | `string` |  |
| `color` | `option<color-token>` |  |
| `bold` | `bool` |  |
| `monospace` | `bool` |  |

### enum `button-style`

Cases: `default`, `filled`, `subtle`, `danger`.

### record `button-props`

| Field | Type | Notes |
|---|---|---|
| `label` | `string` |  |
| `icon` | `option<icon>` |  |
| `style` | `button-style` |  |
| `disabled` | `bool` |  |
| `tooltip` | `option<string>` | Shown on hover. At most 1024 bytes; the host truncates longer text. |

### record `input-props`

| Field | Type | Notes |
|---|---|---|
| `value` | `string` |  |
| `placeholder` | `string` |  |
| `secret` | `bool` | Masks the text on screen (passwords, tokens). The value still reaches the extension in `input-changed` and `input-submitted`; the host never logs it. |

### record `list-props`

Virtualized: the host asks for rows with `list-rows` as they scroll into view. `row-height` is in pixels and fixed for every row. The host fetches rows again when `row-count` or `row-height` changes, or when an `invalidate-rows` patch names the list. Changing only `selected` fetches nothing.

| Field | Type | Notes |
|---|---|---|
| `row-count` | `u32` |  |
| `row-height` | `u16` |  |
| `selected` | `option<u32>` | The highlighted row, scrolled into view when the extension sets it. In a modal the arrow keys move it and the host reports each move with `row-selected`. |

### record `table-props`

Rows come from `list-rows`, like a list; the same refetch rules apply.

| Field | Type | Notes |
|---|---|---|
| `columns` | `list<string>` |  |
| `row-count` | `u32` |  |

### record `icon-props`

| Field | Type | Notes |
|---|---|---|
| `name` | `icon` |  |
| `color` | `option<color-token>` |  |

### record `diff-view-props`

Host component: rendered by Zed's own diff/editor machinery, the extension only supplies text.

| Field | Type | Notes |
|---|---|---|
| `old-text` | `string` |  |
| `new-text` | `string` |  |
| `language` | `option<string>` |  |

### variant `image-source`

0.3. Where an image comes from (design.md §12). `ui:images`.

| Case | Payload | Notes |
|---|---|---|
| `asset` | `string` | A PNG or SVG file in the extension package's `assets/` folder: a relative path with `/` separators and no `..`, such as `assets/logo.svg`. At most 2 MiB on disk. |
| `generated` | `u32` | An image the extension made at run time, from `images.add-image`. |

### enum `image-fit`

Cases: `contain`, `cover`.

### record `image-props`

0.3. A raster or vector image, drawn inside its own box and clipped to it. Panels only: an `image` in a status item, modal, picker or decoration tree rejects the update. Not interactive: a click on it is not an event.

| Field | Type | Notes |
|---|---|---|
| `source` | `image-source` |  |
| `alt` | `string` | Required, 1 to 256 bytes. Shown while the image loads, when it fails, and to screen readers. |
| `width` | `option<u32>` | The box in logical pixels, each 1 to 2048. With both `none` the image's own size is used (PNG pixels, or the SVG's `viewBox`); with one set the other follows the aspect ratio. The box never grows wider than its parent. |
| `height` | `option<u32>` |  |
| `fit` | `image-fit` |  |
| `tint` | `option<color-token>` | SVG only: draw the image as a one-colour mask in this theme colour, like the host's own icons. `none` keeps the SVG's colours. |

### enum `data-color`

0.3. Eight colours for charts and data, chosen by the host to stay distinct and readable on the active theme (and for common colour-vision deficiencies). Never raw values.

Cases: `series1`, `series2`, `series3`, `series4`, `series5`, `series6`, `series7`, `series8`.

### variant `canvas-color`

| Case | Payload | Notes |
|---|---|---|
| `token` | `color-token` |  |
| `data` | `data-color` |  |

### record `paint`

| Field | Type | Notes |
|---|---|---|
| `color` | `canvas-color` |  |
| `opacity` | `u8` | Percent, 0 to 100; 100 is opaque. |

### record `stroke`

| Field | Type | Notes |
|---|---|---|
| `paint` | `paint` |  |
| `width` | `f32` | Logical pixels, 0.5 to 16. |

### record `point`

Canvas coordinates: logical pixels from the canvas's top-left corner. Every number must be finite and within -32768 to 32768.

| Field | Type | Notes |
|---|---|---|
| `x` | `f32` |  |
| `y` | `f32` |  |

### record `rect`

| Field | Type | Notes |
|---|---|---|
| `x` | `f32` |  |
| `y` | `f32` |  |
| `width` | `f32` |  |
| `height` | `f32` |  |

### record `line-op`

| Field | Type | Notes |
|---|---|---|
| `from` | `point` |  |
| `to` | `point` |  |
| `stroke` | `stroke` |  |

### record `rect-op`

| Field | Type | Notes |
|---|---|---|
| `bounds` | `rect` |  |
| `fill` | `option<paint>` |  |
| `stroke` | `option<stroke>` |  |

### record `rounded-rect-op`

| Field | Type | Notes |
|---|---|---|
| `bounds` | `rect` |  |
| `radius` | `f32` | Logical pixels, clamped to half the shorter side. |
| `fill` | `option<paint>` |  |
| `stroke` | `option<stroke>` |  |

### record `circle-op`

| Field | Type | Notes |
|---|---|---|
| `center` | `point` |  |
| `radius` | `f32` |  |
| `fill` | `option<paint>` |  |
| `stroke` | `option<stroke>` |  |

### variant `path-segment`

| Case | Payload | Notes |
|---|---|---|
| `move-to` | `point` |  |
| `line-to` | `point` |  |
| `quad-to` | `tuple<point, point>` | Control point, end point. |
| `cubic-to` | `tuple<point, point, point>` | Two control points, end point. |
| `close` |  |  |

### record `path-op`

At least one of `fill` and `stroke`. A fill uses the non-zero rule.

| Field | Type | Notes |
|---|---|---|
| `segments` | `list<path-segment>` |  |
| `fill` | `option<paint>` |  |
| `stroke` | `option<stroke>` |  |

### enum `canvas-text-size`

The host's UI font sizes, not points.

Cases: `small`, `medium`, `large`.

### enum `text-anchor`

Cases: `start`, `middle`, `end`.

### record `text-op`

One line of text, at most 256 bytes; `at` is the baseline at the anchor.

| Field | Type | Notes |
|---|---|---|
| `at` | `point` |  |
| `text` | `string` |  |
| `paint` | `paint` |  |
| `size` | `canvas-text-size` |  |
| `anchor` | `text-anchor` |  |
| `monospace` | `bool` |  |

### variant `draw-op`

Drawn in list order, later ops over earlier ones.

| Case | Payload | Notes |
|---|---|---|
| `line` | `line-op` |  |
| `rect` | `rect-op` |  |
| `rounded-rect` | `rounded-rect-op` |  |
| `circle` | `circle-op` |  |
| `path` | `path-op` |  |
| `text` | `text-op` |  |

### enum `canvas-fit`

Cases: `clip`, `scale-down`.

### record `canvas-props`

0.3. A static vector drawing (design.md §13). `ui:canvas`. Panels only, like `image`, and not inside list rows. The whole draw list travels in every `set-props`: there are no per-op patches. Caps: 16384 ops, 65536 path segments, 256 KiB of text per canvas; 16 canvases per surface.

| Field | Type | Notes |
|---|---|---|
| `width` | `u32` | The drawing's own size in logical pixels, each 1 to 4096. Nothing is drawn outside it. |
| `height` | `u32` |  |
| `description` | `string` | Required, 1 to 1024 bytes: what the drawing shows, for screen readers and as the tooltip. |
| `ops` | `list<draw-op>` |  |
| `clickable` | `bool` | Sends `canvas-click` with canvas coordinates. No hover, pointer-move or drag in 0.3. |
| `fit` | `canvas-fit` |  |

### variant `props`

| Case | Payload | Notes |
|---|---|---|
| `flex` | `flex-props` |  |
| `text` | `text-props` |  |
| `button` | `button-props` |  |
| `input` | `input-props` |  |
| `list` | `list-props` |  |
| `table` | `table-props` |  |
| `icon` | `icon-props` |  |
| `markdown` | `string` |  |
| `diff-view` | `diff-view-props` |  |
| `image` | `image-props` | 0.3. |
| `canvas` | `canvas-props` | 0.3. |

### record `node`

WIT types cannot be recursive, so a tree travels as a flat `list<node>` in which every node names its parent. Parents precede children; siblings keep list order. Only `flex` nodes have children.

| Field | Type | Notes |
|---|---|---|
| `id` | `node-id` |  |
| `parent` | `option<node-id>` |  |
| `props` | `props` |  |

### record `insert-op`

| Field | Type | Notes |
|---|---|---|
| `node` | `node` |  |
| `before` | `option<node-id>` | Insert before this sibling; `none` appends. |

### record `row-range`

`count` rows from `start`.

| Field | Type | Notes |
|---|---|---|
| `start` | `u32` |  |
| `count` | `u32` |  |

### record `invalidate-rows-op`

| Field | Type | Notes |
|---|---|---|
| `target` | `node-id` | A `list` or `table` node of the surface tree. |
| `rows` | `option<row-range>` | `none` invalidates every row. Rows outside the list are ignored. |

### variant `patch`

| Case | Payload | Notes |
|---|---|---|
| `set-props` | `tuple<node-id, props>` |  |
| `insert` | `insert-op` |  |
| `remove` | `node-id` | Removes the node and its whole subtree. |
| `invalidate-rows` | `invalidate-rows-op` | The rows changed although the list's props did not. The host keeps drawing the old rows until their replacements arrive, and asks only for the visible ones. |

### variant `update`

Caps: at most 2048 patches in one `patches`; at most 16384 nodes in a surface tree, which is also the cap for a `replace` and for `mount`, so a whole surface can always be resent; at most 2048 nodes in one list row. An update that breaks a cap or does not fit the tree is rejected whole, and the host mounts the surface again.

| Case | Payload | Notes |
|---|---|---|
| `unchanged` |  |  |
| `replace` | `list<node>` |  |
| `patches` | `list<patch>` |  |

### record `list-row`

| Field | Type | Notes |
|---|---|---|
| `index` | `u32` |  |
| `indent` | `u8` | Nesting depth, drawn by the host as indentation (tree views). The host draws at most 32 levels and clamps deeper rows. |
| `nodes` | `list<node>` | A subtree whose single root has `parent: none`. Ids are row-local (see `node-id`). |

### enum `dismiss-reason`

Why the host closed a modal. Never sent when the extension closed it itself.

Cases: `escape`, `focus-lost`, `replaced`, `window-closed`.

### record `notification-action-event`

§M. Which action of which notification was clicked.

| Field | Type | Notes |
|---|---|---|
| `notification` | `u64` |  |
| `action` | `string` | The `id` the extension gave the action. |

### enum `notification-close-reason`

§M. Why a notification closed when the extension did not close it.

Cases: `user`, `expired`, `window-closed`.

### record `canvas-click`

| Field | Type | Notes |
|---|---|---|
| `canvas` | `node-id` |  |
| `x` | `f32` |  |
| `y` | `f32` |  |

### variant `ui-event`

| Case | Payload | Notes |
|---|---|---|
| `click` | `node-id` |  |
| `input-changed` | `tuple<node-id, string>` |  |
| `input-submitted` | `tuple<node-id, string>` |  |
| `row-click` | `tuple<node-id, u32>` | The list node and the row index within it. A click on anything inside a row, buttons included, is a `row-click`. |
| `row-selected` | `tuple<node-id, u32>` | The user moved a list's selection with the keyboard (modals only). The host has already set `selected` in its tree; the extension need not echo it. |
| `dismissed` | `dismiss-reason` | The host closed a modal surface; its tree is gone. The reply must be `unchanged`. |
| `orphaned` |  | 0.3, block decorations only: the line the decoration was anchored to was deleted. The block stays where the anchor collapsed to until the extension moves or removes it. |
| `canvas-click` | `canvas-click` | 0.3. A click on a `clickable` canvas, in canvas coordinates (after `scale-down`, so they match the draw list). Grants the click-or-Enter activation like `click`. |
| `notification-action` | `notification-action-event` | §M. The user clicked an action button of a notification this surface owns. Grants the click-or-Enter activation, like `click`. |
| `notification-closed` | `tuple<u64, notification-close-reason>` | §M. A notification this surface owns closed without the extension asking (an extension's own `dismiss-notification` sends nothing). |
| `progress-cancel-requested` | `u64` | §M. The user pressed Cancel on a cancellable progress this surface owns. The progress stays until the extension calls `progress-end`. |

### enum `dock-position`

Cases: `left`, `right`, `bottom`.

### record `panel-descriptor`

| Field | Type | Notes |
|---|---|---|
| `position` | `dock-position` |  |
| `icon` | `icon` |  |
| `default-size` | `u32` |  |

### enum `status-bar-side`

Cases: `left`, `right`.

### record `status-item-descriptor`

| Field | Type | Notes |
|---|---|---|
| `side` | `status-bar-side` |  |
| `priority` | `s32` | Higher sorts closer to the edge. |

### enum `modal-width`

Width classes of Zed's own modals, so extension pickers line up with the built-in ones.

Cases: `small`, `medium`, `large`.

### record `modal-descriptor`

Like Zed's own modals, an extension modal always closes on Escape, on a click outside it and when focus leaves it; it cannot veto that.

| Field | Type | Notes |
|---|---|---|
| `width` | `modal-width` |  |

### enum `picker-action-key`

Cases: `alt-enter`, `secondary-enter`.

### record `picker-action`

| Field | Type | Notes |
|---|---|---|
| `id` | `string` | 1 to 64 bytes of `a-z`, `0-9`, `-`, `_`, `.`. |
| `label` | `string` | At most 64 bytes; shown in the picker footer next to its key. |
| `key` | `picker-action-key` |  |

### record `picker-descriptor`

0.3. A picker is a modal whose body the host draws: a query input and a fuzzy-filtered list of items that sources supply (design.md, "Picker protocol"). It has the modal lifecycle: `show-surface` after a click or Enter, one modal per window, `dismissed` when the user closes it. The host never calls `mount` for a picker.

| Field | Type | Notes |
|---|---|---|
| `width` | `modal-width` |  |
| `placeholder` | `string` | Shown in the empty query input. |
| `multi-select` | `bool` | Tab marks or unmarks the highlighted item; Enter accepts the marked items, or the highlighted one when none is marked. |
| `dynamic` | `bool` | Sources get every query change (debounced 100 ms) and answer with new items, instead of the host filtering the items they gave when the picker opened. |
| `open-to-sources` | `bool` | Other extensions may contribute sources to this picker (manifest `[[fyx_ui.picker_sources]]`). Their items are shown by the host and never reach the owner. |
| `actions` | `list<picker-action>` | At most 2 extra ways to accept, one per key. |

### variant `surface-kind`

Panels and status items are mounted when they are added and live until removed. A modal is mounted each time it is shown and its tree is dropped when it closes. A picker is never mounted (0.3).

| Case | Payload | Notes |
|---|---|---|
| `panel` | `panel-descriptor` |  |
| `status-item` | `status-item-descriptor` |  |
| `modal` | `modal-descriptor` |  |
| `picker` | `picker-descriptor` |  |

### type `document-id` = `u64`

0.3. An open document (an editor buffer), as the host names it to this extension. Stable while the document is open; never reused for another document during the session.

### enum `decoration-placement`

Cases: `above`, `below`.

### variant `decoration-target`

0.3. What a decoration is anchored in.

| Case | Payload | Notes |
|---|---|---|
| `path` | `string` | Relative to a worktree root, `/`-separated, without `..` (`src/main.rs`), as in 0.2: matched against every worktree of every open workspace, anchored in buffers that are open now or opened later. |
| `document` | `document-id` | One open document. The decoration is removed when that document closes. |

### record `decoration-location`

Where a block decoration goes. Only checked for syntax when `place-decoration` is called.

| Field | Type | Notes |
|---|---|---|
| `target` | `decoration-target` | 0.3: replaces 0.2's `path: string`; `target: path(...)` behaves exactly like it. |
| `line` | `u32` | Zero-based row when the decoration is anchored. The host turns it into a CRDT anchor per buffer, so from then on the block moves with the line; rows past the end clamp to the last row. If the anchored line is deleted, the block stays where the anchor collapses to and the extension gets `orphaned` (0.3). |
| `placement` | `decoration-placement` |  |
| `height-lines` | `option<u16>` | Block height in editor lines; `none` lets the host estimate it from the tree. |

### record `surface`

| Field | Type | Notes |
|---|---|---|
| `id` | `string` | 1 to 64 bytes of `a-z`, `0-9`, `-`, `_`, `.`; unique among the extension's live surfaces. |
| `title` | `string` |  |
| `kind` | `surface-kind` |  |

## Interface `host`

Functions the extension may call on the host. Every function with an effect (an update, a surface change, a tick) is checked when it is called and returns the check's result at once, but the effect itself happens after the current export call returns, in the order the extension called them, and before that call's own return value is applied. 0.3: the same rule covers every effectful function of every interface below. Functions that start work outside the call (a fetch, a process, an edit) return an id at once; the outcome arrives later through the `handle-io` export.

### type `update` = `update`


### type `surface` = `surface`


### enum `host-error-code`

Cases: `unknown-surface`, `limit-exceeded`, `invalid-argument`, `not-permitted`, `unavailable`, `denied`.

### record `host-error`

| Field | Type | Notes |
|---|---|---|
| `code` | `host-error-code` |  |
| `message` | `string` |  |

### enum `log-level`

Cases: `trace`, `debug`, `info`, `warn`, `error`.

### `push-update: func(surface-id: string, update: update) -> result<_, host-error>`

Sends an update the extension initiated (a push to another surface, a tick's side effects). `ok` means the update passed the checks the host can make at once (the surface exists, the caps); it can still fail to fit the tree when it is applied, which remounts the surface. On `err` nothing was queued: the extension's mirror of that surface is now ahead of the host and must be resent whole. At most 64 pushes per export call.

### `request-tick: func(surface-id: string, delay-ms: u32) -> result<_, host-error>`

Asks for one `tick(surface-id)` call at least `delay-ms` after the current call returns. One pending tick per surface: a new request replaces the earlier one. The host clamps the delay up to 100 ms, and to 1000 ms while Fyx is in the background. A tick that falls due while its panel is hidden waits until the panel is shown and is then delivered once: a panel opened hours later gets one tick, not a backlog (design.md, "Ticks"). Decorations cannot request ticks.

### `cancel-tick: func(surface-id: string)`

Drops the pending tick of a surface, if any.

### `add-surface: func(surface: surface) -> result<_, host-error>`

Adds a surface at runtime. The manifest must declare the capability of its kind. Panels and status items are added to every workspace window and mounted; a modal waits for `show-surface`. At most 32 live surfaces per extension.

### `remove-surface: func(surface-id: string) -> result<_, host-error>`

Removes a live surface, static or runtime: its containers close in every window, its tree, pending tick and queued events are dropped, and no event is sent. The id may be added again afterwards, as a new surface.

### `show-surface: func(surface-id: string) -> result<_, host-error>`

Opens a modal in the active workspace window and focuses it, or reveals a panel there (opens its dock and activates it, without moving focus). Showing a modal needs a click or Enter: it is allowed during `handle-event` for `click`, `row-click` or `input-submitted`, or within 5 s after one, once. Typing (`input-changed`), `row-selected` and `dismissed` do not count. Showing a surface that is already shown does nothing. 0.3: a picker is a modal for this rule. `handle-command` and an accepted picker item also grant the click-or-Enter activation (design.md, "Gestures in 0.3").

### `hide-surface: func(surface-id: string) -> result<_, host-error>`

Closes a modal (no `dismissed` event) or hides a panel's dock if that panel is active.

### `log: func(level: log-level, message: string)`

## Interface `documents`

0.3. Positions, ranges and what the host tells an extension about a document.

### type `document-id` = `document-id`


### record `position`

Zero-based line, and a column in UTF-8 bytes from the start of the line. A column that is not on a character boundary is rejected with `invalid-argument`; one past the end of the line clamps to the end.

| Field | Type | Notes |
|---|---|---|
| `line` | `u32` |  |
| `column` | `u32` |  |

### record `text-range`

Half-open: `start` is included, `end` is not.

| Field | Type | Notes |
|---|---|---|
| `start` | `position` |  |
| `end` | `position` |  |

### record `line-range`

`count` lines from `start`.

| Field | Type | Notes |
|---|---|---|
| `start` | `u32` |  |
| `count` | `u32` |  |

### record `file-ref`

A file in a worktree of an open workspace.

| Field | Type | Notes |
|---|---|---|
| `worktree` | `option<u64>` | Opaque id of the worktree, stable for the session. `none` in a request means "the one worktree that has this path"; a path found in more than one is `invalid-argument`. |
| `path` | `string` | Relative to the worktree root, `/`-separated, without empty, `.` or `..` components. |

### variant `document-origin`

| Case | Payload | Notes |
|---|---|---|
| `file` | `file-ref` |  |
| `virtual` | `string` | One of this extension's own virtual documents (`fyx-ext://<id>/<path>`), by its path. |
| `other` |  | Neither: an untitled buffer, or a file outside every worktree. The host never names it. |

### record `document-info`

| Field | Type | Notes |
|---|---|---|
| `id` | `document-id` |  |
| `origin` | `document-origin` |  |
| `language` | `option<string>` | The language name as the host knows it (`Rust`, `Markdown`), if any. |
| `version` | `u64` | Grows with every change to the buffer; an edit can name the version it was computed against. |
| `line-count` | `u32` |  |

### record `text-slice`

Lines of a document, joined with `\n`, starting at `start-line`.

| Field | Type | Notes |
|---|---|---|
| `start-line` | `u32` |  |
| `text` | `string` |  |

### enum `diagnostic-severity`

Cases: `error`, `warning`, `information`, `hint`.

### record `diagnostic`

| Field | Type | Notes |
|---|---|---|
| `range` | `text-range` |  |
| `severity` | `diagnostic-severity` |  |
| `message` | `string` | At most 4 KiB; longer messages are truncated. |
| `source` | `option<string>` | The language server or tool that reported it. |

### record `document-context`

What a request tells the extension about a document. `text` and `selections` are filled only for an extension granted `workspace:read-buffers`, `diagnostics` only for one granted `workspace:diagnostics` (T1, design.md "N"); otherwise they are `none`.

| Field | Type | Notes |
|---|---|---|
| `info` | `document-info` |  |
| `text` | `option<text-slice>` | The lines the request is about (the visible range, the hovered line), plus up to 200 lines around them; at most 1 MiB. |
| `selections` | `option<list<text-range>>` |  |
| `diagnostics` | `option<list<diagnostic>>` | Diagnostics inside the lines of `text`, or of the requested range when `text` is `none`; at most 1000. |

### record `document-selector`

Which documents a provider or a decoration namespace wants requests for. A document matches when its language is in `languages` or its worktree path matches one of `globs` (globset syntax). Both empty matches every document of origin `file`.

| Field | Type | Notes |
|---|---|---|
| `languages` | `list<string>` |  |
| `globs` | `list<string>` |  |

## Interface `decorations`

Editor decorations: retained `fyx:ui` trees drawn as blocks above or below a buffer line (0.2, design.md "L"), and 0.3's styled decorations in namespaces. Needs the `ui:editor-decorations` capability; without it every call returns `not-permitted`. Every function follows the effect rule of `host`. A block decoration id lives in the same namespace as the extension's surface ids and follows the same id rule, so it must not equal a live surface id. Once placed, the id works like a surface id for `push-update`, `handle-event` (clicks, input, 0.3 `orphaned`), `list-rows` and `mount` (the host calls `mount(id)` to resync a decoration whose update did not fit), but not for `request-tick`, `show-surface` or `remove-surface`. At most 512 live block decorations per extension, not counted against the 32 surfaces.

### type `node` = `node`


### type `decoration-location` = `decoration-location`


### type `color-token` = `color-token`


### type `icon` = `icon`


### type `document-id` = `document-id`


### type `host-error` = `host-error`


### type `position` = `position`


### type `text-range` = `text-range`


### type `line-range` = `line-range`


### type `document-selector` = `document-selector`


### variant `decoration-color`

---- 0.3: styled decorations ---- A colour for decorations: a theme token, or one of the theme's accent colours (the ones bracket colouring and indent guides use), taken modulo however many the theme has.

| Case | Payload | Notes |
|---|---|---|
| `token` | `color-token` |  |
| `accent` | `u8` |  |

### enum `underline-style`

Cases: `straight`, `wavy`.

### record `underline`

| Field | Type | Notes |
|---|---|---|
| `style` | `underline-style` |  |
| `color` | `option<decoration-color>` |  |

### record `inline-style`

Styles a range of text. Several styles on the same text merge; the extension that defined its namespace later wins a conflict.

| Field | Type | Notes |
|---|---|---|
| `range` | `text-range` |  |
| `foreground` | `option<decoration-color>` |  |
| `background` | `option<decoration-color>` |  |
| `underline` | `option<underline>` |  |
| `strikethrough` | `bool` |  |
| `bold` | `bool` |  |
| `italic` | `bool` |  |

### record `eol-text`

Muted text after the end of a line (inline blame, inline diagnostics). At most 256 bytes.

| Field | Type | Notes |
|---|---|---|
| `line` | `u32` |  |
| `text` | `string` |  |
| `color` | `option<decoration-color>` |  |

### enum `inline-side`

Cases: `before`, `after`.

### record `inline-text`

Text drawn between characters, which the cursor skips (an inlay). At most 64 bytes.

| Field | Type | Notes |
|---|---|---|
| `position` | `position` |  |
| `side` | `inline-side` |  |
| `text` | `string` |  |
| `color` | `option<decoration-color>` |  |

### record `command-invocation`

What clicking a gutter sign or a code lens runs: one of the extension's own manifest commands, called as `handle-command(command, argument, ...)`.

| Field | Type | Notes |
|---|---|---|
| `command` | `string` |  |
| `argument` | `option<string>` | Opaque to the host. At most 4 KiB. |

### record `gutter-sign`

An icon in the gutter. One sign per line per extension: the last one given wins.

| Field | Type | Notes |
|---|---|---|
| `line` | `u32` |  |
| `icon` | `icon` |  |
| `color` | `option<decoration-color>` |  |
| `tooltip` | `option<string>` | Shown on hover. At most 1024 bytes. |
| `on-click` | `option<command-invocation>` |  |

### record `line-background`

Paints whole lines.

| Field | Type | Notes |
|---|---|---|
| `lines` | `line-range` |  |
| `color` | `decoration-color` |  |

### record `color-swatch`

A small square of a raw colour, drawn before or after `position` with a border, so it is legible on any theme. The only place where a raw colour is accepted (`0xRRGGBBAA`).

| Field | Type | Notes |
|---|---|---|
| `position` | `position` |  |
| `side` | `inline-side` |  |
| `rgba` | `u32` |  |

### variant `decoration`

| Case | Payload | Notes |
|---|---|---|
| `inline-style` | `inline-style` |  |
| `eol-text` | `eol-text` |  |
| `inline-text` | `inline-text` |  |
| `gutter-sign` | `gutter-sign` |  |
| `line-background` | `line-background` |  |
| `color-swatch` | `color-swatch` |  |

### record `namespace-options`

| Field | Type | Notes |
|---|---|---|
| `pull` | `option<document-selector>` | When set, the host calls `provide-decorations` for matching documents as their visible range changes (debounced 50 ms). When `none`, the namespace is filled with `set-decorations` only. |

### `place-decoration: func(id: string, location: decoration-location, nodes: list<node>) -> result<_, host-error>`

Places a decoration with the initial tree `nodes` (at most 16384 nodes, parents first). Placing an id that is already a live decoration moves it and replaces its tree, like a fresh mount.

### `remove-decoration: func(id: string) -> result<_, host-error>`

Removes a live decoration in every editor; no event is sent. The id may be placed again.

### `define-namespace: func(namespace: string, options: namespace-options) -> result<_, host-error>`

Declares a namespace: 1 to 64 bytes of `a-z`, `0-9`, `-`, `_`, `.`, owned by this extension. Defining it again replaces its options and keeps its decorations. At most 16 namespaces per extension.

### `set-decorations: func(namespace: string, document: document-id, lines: option<line-range>, decorations: list<decoration>) -> result<_, host-error>`

Replaces the namespace's decorations in one open document: within `lines` only, or in the whole document when `lines` is `none`. The host anchors every decoration, so it follows edits until it is replaced. At most 10000 decorations per call, per namespace and document, and 50000 per extension.

### `clear-namespace: func(namespace: string) -> result<_, host-error>`

Removes the namespace's decorations from every document. The namespace stays defined.

## Interface `language`

0.3. Hover and code lens contributions, drawn by the editor's own hover popover and code lens lines. `ui:hover` and `ui:code-lens` respectively.

### type `host-error` = `host-error`


### type `document-selector` = `document-selector`


### enum `provider-kind`

Cases: `hover`, `code-lens`.

### `register-provider: func(kind: provider-kind, selector: document-selector) -> result<u32, host-error>`

The host calls `provide-hover` or `provide-code-lenses` with the returned id for documents that match. At most 16 providers per extension.

### `unregister-provider: func(provider: u32)`

### `resolve-hover: func(request: u64, markdown: option<string>) -> result<_, host-error>`

Answers a hover that `provide-hover` left `pending`. Ignored after 3 s, or once the hover popover has closed. `none` shows nothing.

## Interface `virtual-documents`

0.3. Read-only editor tabs whose text the extension supplies, at `fyx-ext://<id>/<path>`. `ui:virtual-documents`. The host pulls the text with `provide-virtual-document`.

### type `host-error` = `host-error`


### `open-virtual-document: func(path: string) -> result<_, host-error>`

Opens the document in a tab of the active workspace window, or focuses its tab. Needs the click-or-Enter activation, like `show-surface` on a modal. `path` follows the worktree-path rule, at most 256 bytes.

### `invalidate-virtual-document: func(path: string) -> result<_, host-error>`

The text changed: the host calls `provide-virtual-document` again if the document is open.

### `close-virtual-document: func(path: string) -> result<_, host-error>`

Closes every tab of the document.

## Interface `permissions`

0.3. The state of the T2 capabilities this extension declares, and a way to ask for one ahead of its first use (from a "Connect" button, say).

### type `host-error` = `host-error`


### enum `t2-capability`

Cases: `network-fetch`, `process-spawn`, `workspace-edit`, `agent-observe`.

### enum `permission-state`

Cases: `granted`, `denied`, `ask`, `unavailable`.

### `query: func(capability: t2-capability) -> permission-state`

The state as of the start of the current call.

### `request: func(capability: t2-capability) -> result<_, host-error>`

Shows the consent prompt now, if the state is `ask`. Needs the click-or-Enter activation. The answer arrives as `io-event::permission-changed`.

## Interface `network`

0.3. `network:fetch`: HTTPS requests to the domains the manifest lists (spec decision 23).

### type `host-error` = `host-error`


### enum `http-method`

Cases: `get`, `head`, `post`, `put`, `patch`, `delete`, `options`.

### record `http-request`

| Field | Type | Notes |
|---|---|---|
| `method` | `http-method` |  |
| `url` | `string` | `https://` and a host the manifest allows; `http://` only for a declared `localhost` or `127.0.0.1` with a port. No user info. At most 8 KiB. |
| `headers` | `list<tuple<string, string>>` | At most 64 headers, 16 KiB in total. `host`, `content-length`, `transfer-encoding`, `connection`, `upgrade`, `te`, `trailer`, `expect`, `keep-alive` and every `proxy-*` header are set by the host and rejected here. |
| `body` | `option<list<u8>>` | At most 4 MiB. |
| `timeout-ms` | `option<u32>` | At most 30000, the default. |

### record `http-response`

| Field | Type | Notes |
|---|---|---|
| `status` | `u16` |  |
| `headers` | `list<tuple<string, string>>` |  |
| `body` | `list<u8>` | Decompressed, at most 16 MiB. |
| `url` | `string` | The URL that answered, after the redirects the host followed. |

### variant `fetch-error`

| Case | Payload | Notes |
|---|---|---|
| `denied` |  | The capability is denied or was revoked while the request waited or ran. |
| `host-not-allowed` | `string` | The host is not in the manifest's domain list, or resolved to an address the host does not connect to (a private address for a public domain). |
| `redirect-not-allowed` | `string` | A redirect pointed off the list, to `http://`, or past the fifth hop. Names the host. |
| `dns` | `string` |  |
| `connect` | `string` |  |
| `tls` | `string` |  |
| `timeout` |  |  |
| `too-large` |  | The response passed 16 MiB, or the headers 64 KiB. |
| `cancelled` |  |  |
| `other` | `string` |  |

### `fetch: func(request: http-request) -> result<u64, host-error>`

Starts a request after the current call returns and returns its id. The outcome arrives as `io-event::fetch-done`. The first fetch shows the consent prompt if the state is `ask`; the request waits for the answer. At most 8 requests in flight and 120 started per minute.

### `cancel-fetch: func(request: u64)`

Cancels a request; `fetch-done` follows with `cancelled` unless it already completed.

## Interface `process`

0.3. `process:spawn`: runs programs the manifest names (spec decision 24).

### type `host-error` = `host-error`


### type `file-ref` = `file-ref`


### record `spawn-request`

| Field | Type | Notes |
|---|---|---|
| `program` | `string` | A name from the manifest's `[[fyx_ui.processes]]`, never a path. |
| `args` | `list<string>` | At most 256 arguments, 128 KiB in total. |
| `cwd` | `option<file-ref>` | A directory inside a worktree; `none` is the root of the first worktree. |
| `env` | `list<tuple<string, string>>` | Only names the manifest lists in `[fyx_ui.process].env`; at most 32. |
| `stdin` | `bool` | Keep stdin open for `write-stdin`; otherwise it is closed at start. |

### enum `output-stream`

Cases: `stdout`, `stderr`.

### record `process-output`

| Field | Type | Notes |
|---|---|---|
| `process` | `u64` |  |
| `stream` | `output-stream` |  |
| `data` | `list<u8>` | At most 64 KiB per event. |

### record `process-exit`

| Field | Type | Notes |
|---|---|---|
| `process` | `u64` |  |
| `code` | `option<s32>` | `none` when it ended by a signal or was killed. |
| `killed` | `bool` | The host killed it (`kill`, disable, reload, revoke, uninstall, Fyx quitting). |

### variant `spawn-error`

| Case | Payload | Notes |
|---|---|---|
| `denied` |  |  |
| `not-found` | `string` | The program is not on `PATH` outside the worktrees, or is a script the host does not run (`.bat`, `.cmd`). |
| `limit-exceeded` |  |  |
| `io` | `string` |  |

### `spawn: func(request: spawn-request) -> result<u64, host-error>`

Starts the program after the current call returns and returns its id. Output arrives as `io-event::process-output`, then exactly one `process-exited`, or one `spawn-failed` instead of both. At most 4 processes per extension.

### `write-stdin: func(process: u64, data: list<u8>) -> result<_, host-error>`

At most 1 MiB per call and 4 MiB queued per process.

### `close-stdin: func(process: u64) -> result<_, host-error>`

### `kill: func(process: u64) -> result<_, host-error>`

Kills the process and everything it started.

## Interface `workspace-edit`

0.3. `workspace:edit`: text edits to files in the worktrees, as one undoable step each.

### type `host-error` = `host-error`


### type `document-id` = `document-id`


### type `text-range` = `text-range`


### type `file-ref` = `file-ref`


### record `text-edit`

| Field | Type | Notes |
|---|---|---|
| `range` | `text-range` |  |
| `new-text` | `string` |  |

### variant `edit-target`

| Case | Payload | Notes |
|---|---|---|
| `document` | `document-id` |  |
| `file` | `file-ref` |  |

### record `file-edit`

| Field | Type | Notes |
|---|---|---|
| `target` | `edit-target` |  |
| `expected-version` | `option<u64>` | Fails the whole edit with `conflict` if the document has changed since this version. |
| `create` | `bool` | Create the file if it does not exist (its parent directory must exist). |
| `edits` | `list<text-edit>` | Non-overlapping, at most 4096 per edit in total. |

### record `workspace-edit`

| Field | Type | Notes |
|---|---|---|
| `label` | `string` | Names the step in the undo history and in the title of the multi-file view. At most 128 bytes. |
| `files` | `list<file-edit>` | At most 64 files. |

### variant `edit-error`

| Case | Payload | Notes |
|---|---|---|
| `denied` |  |  |
| `conflict` | `string` | A file changed since `expected-version`. |
| `not-allowed` | `string` | The path is outside every worktree, leaves it through a symlink, or names a protected path (`.git/`, `.fyx/`, `.zed/`, `.vscode/`, the `private_files` setting). |
| `not-found` | `string` |  |
| `invalid` | `string` |  |
| `cancelled` |  |  |

### `apply-edit: func(edit: workspace-edit) -> result<u64, host-error>`

Applies the edit after the current call returns, all or nothing, and returns its id. The outcome arrives as `io-event::edit-done`. Nothing is saved: edited buffers stay dirty. At most 4 edits in flight and 4 MiB of new text per edit.

## Interface `agent-observe`

0.3. `agent:observe`: the host's review of agent edits, read-only. Nothing in this world can accept, reject, comment on or submit a review.

### type `host-error` = `host-error`


### type `file-ref` = `file-ref`


### enum `hunk-decision`

Cases: `pending`, `accepted`, `rejected`.

### record `hunk-info`

| Field | Type | Notes |
|---|---|---|
| `old-start` | `u32` | Zero-based rows of the old text the hunk replaces, and of the new text it inserts. |
| `old-count` | `u32` |  |
| `new-start` | `u32` |  |
| `new-count` | `u32` |  |
| `old-text` | `string` |  |
| `new-text` | `string` |  |
| `decision` | `hunk-decision` |  |
| `comment` | `option<string>` | The user's comment to the agent, if any. |

### variant `resolution`

| Case | Payload | Notes |
|---|---|---|
| `unchanged` |  |  |
| `pending` | `u32` |  |
| `accepted-all` |  |  |
| `rejected-all` |  |  |
| `partial` | `tuple<u32, u32>` | Accepted and rejected hunk counts. |

### enum `review-source`

Cases: `permission`, `write`.

### enum `rule-kind`

Cases: `auto-accept`, `always-ask`, `deny`.

### record `review-info`

| Field | Type | Notes |
|---|---|---|
| `review` | `u64` | Opaque ids, stable for the session, different for every extension. |
| `session` | `u64` |  |
| `turn` | `u64` |  |
| `file` | `option<file-ref>` | `none` when the file is outside every worktree; its path is never shown. |
| `source` | `review-source` |  |
| `rule` | `option<rule-kind>` | The project rule that applies, from `.fyx/review.toml`. |
| `resolution` | `resolution` |  |
| `submitted` | `bool` |  |
| `hunks` | `list<hunk-info>` | Empty when `file` is `none`. Cut after 256 KiB of hunk text, with `truncated` set. |
| `truncated` | `bool` |  |

### record `turn-file`

| Field | Type | Notes |
|---|---|---|
| `file` | `option<file-ref>` |  |
| `resolution` | `resolution` |  |
| `rule` | `option<rule-kind>` |  |

### variant `agent-event`

| Case | Payload | Notes |
|---|---|---|
| `snapshot` | `list<review-info>` | First event after `start`: every open review. |
| `opened` | `review-info` |  |
| `updated` | `review-info` |  |
| `resolved` | `review-info` |  |
| `closed` | `u64` |  |
| `turn-ended` | `tuple<u64, list<turn-file>>` | The turn stopped; the files decided in it. |
| `unreviewed-changes` | `tuple<u64, list<file-ref>>` | Files that changed on disk during the turn without review. |
| `review-mode` | `bool` | Review mode was switched on or off. |

### `start: func() -> result<_, host-error>`

Starts delivering `io-event::agent` events, beginning with a `snapshot`. The first call shows the consent prompt if the state is `ask`.

### `stop: func()`

## Interface `io`

0.3. Outcomes of work that runs outside a call, delivered through `handle-io`.

### type `http-response` = `http-response`


### type `fetch-error` = `fetch-error`


### type `process-output` = `process-output`


### type `process-exit` = `process-exit`


### type `spawn-error` = `spawn-error`


### type `edit-error` = `edit-error`


### type `agent-event` = `agent-event`


### type `t2-capability` = `t2-capability`


### type `permission-state` = `permission-state`


### variant `io-event`

| Case | Payload | Notes |
|---|---|---|
| `fetch-done` | `tuple<u64, result<http-response, fetch-error>>` |  |
| `process-output` | `process-output` |  |
| `process-exited` | `process-exit` |  |
| `spawn-failed` | `tuple<u64, spawn-error>` |  |
| `edit-done` | `tuple<u64, result<_, edit-error>>` |  |
| `agent` | `agent-event` |  |
| `permission-changed` | `tuple<t2-capability, permission-state>` | The user answered a prompt or changed the extension's permissions; in-flight work of a revoked capability has already been cancelled. |

## Interface `requests`

0.3. What the host passes along with a request to an extension's provider exports.

### type `document-context` = `document-context`


### type `position` = `position`


### type `line-range` = `line-range`


### type `command-invocation` = `command-invocation`


### enum `decoration-reason`

Cases: `opened`, `scrolled`, `edited`, `refresh`.

### record `decoration-request`

| Field | Type | Notes |
|---|---|---|
| `namespace` | `string` |  |
| `document` | `document-context` |  |
| `lines` | `line-range` | The lines in view, plus one screen above and below. |
| `reason` | `decoration-reason` |  |

### record `hover-request`

| Field | Type | Notes |
|---|---|---|
| `provider` | `u32` |  |
| `request` | `u64` | For `resolve-hover`. |
| `document` | `document-context` |  |
| `position` | `position` |  |

### variant `hover-reply`

| Case | Payload | Notes |
|---|---|---|
| `none` |  |  |
| `ready` | `string` | Markdown, at most 64 KiB. Links open only when they are `https:`. |
| `pending` |  | The answer comes later through `resolve-hover`. |

### record `code-lens-request`

| Field | Type | Notes |
|---|---|---|
| `provider` | `u32` |  |
| `document` | `document-context` |  |
| `lines` | `line-range` |  |

### enum `command-source`

Cases: `palette`, `keymap`, `code-lens`, `gutter-sign`.

### record `command-context`

| Field | Type | Notes |
|---|---|---|
| `source` | `command-source` |  |
| `document` | `option<document-context>` | The focused editor's document, if any. |

### enum `picker-accept`

Cases: `enter`, `alt-enter`, `secondary-enter`.

### variant `picker-source-event`

| Case | Payload | Notes |
|---|---|---|
| `opened` | `string` | The picker opened; send items with `picker.set-items`. Carries the initial query. |
| `query-changed` | `string` | Dynamic pickers only: the query changed (debounced 100 ms). |
| `highlighted` | `string` | The highlighted item of this source changed, for a live preview. Coalesced; at most every 50 ms. |
| `accepted` | `tuple<list<string>, picker-accept>` | The user accepted these items of this source, with that key. |
| `closed` |  | The picker closed, accepted or dismissed. No more events for this session. |

### record `code-lens`

One code lens line item, drawn like a language server's lens above `line`.

| Field | Type | Notes |
|---|---|---|
| `line` | `u32` |  |
| `title` | `string` | At most 256 bytes. |
| `tooltip` | `option<string>` |  |
| `on-click` | `option<command-invocation>` |  |

### record `virtual-document`

| Field | Type | Notes |
|---|---|---|
| `text` | `string` | At most 8 MiB. |
| `language` | `option<string>` | A language name the host knows (`JSON`, `Markdown`); plain text otherwise. |
| `title` | `option<string>` | The tab title after the extension's name; the last path component when `none`. At most 128 bytes. |

## Interface `images`

0.3. Images the extension makes at run time, for `image-source::generated` (design.md §12). `ui:images`. Package assets need no call.

### type `host-error` = `host-error`


### enum `image-format`

Cases: `png`, `svg`.

### `add-image: func(format: image-format, bytes: list<u8>) -> result<u32, host-error>`

Registers an image and returns its handle. The bytes are checked and decoded off the main thread after the call returns; until then, and if decoding fails, nodes show the alt text. At most 1 MiB of bytes, the same size limits as assets, at most 32 live generated images, and all images of the extension count against its 64 MiB of decoded pixels.

### `remove-image: func(image: u32)`

Frees a generated image. Nodes that still show it show their alt text. Unknown handles are ignored. A reload or a disable frees all of them.

## Interface `picker`

0.3. Items for pickers: the owner's own picker surfaces and the sources this extension contributes to other pickers (manifest `[[fyx_ui.picker_sources]]`). `ui:picker`.

### type `icon` = `icon`


### type `host-error` = `host-error`


### record `picker-item`

| Field | Type | Notes |
|---|---|---|
| `key` | `string` | Unique within the source; 1 to 128 bytes. |
| `label` | `string` | What the host matches the query against, unless `filter-text` is set. At most 512 bytes. |
| `detail` | `option<string>` | Muted text right of the label (a path, a branch). At most 512 bytes. |
| `icon` | `option<icon>` |  |
| `filter-text` | `option<string>` |  |

### `set-items: func(source: string, session: u64, items: list<picker-item>, append: bool) -> result<_, host-error>`

Sets the items of `source` for one showing of a picker (`session` from `picker-source-event`). `source` is the picker's surface id for the owner, or the source id from the manifest for a contributor. Replaces the items given before for this session unless `append` is set. Items for a session that has closed are dropped without error. At most 10000 items per source and session, and 2000 per call.

## Interface `notifications`

Notifications and progress (design.md, "M"). Needs the `ui:notifications` capability (tier T0); without it every call returns `not-permitted`. Every function follows the effect rule of `host`: checked at the call, applied after the export returns, dropped by a trap. Every notification and progress has an **owner**: a live surface or decoration id of this extension. Its events arrive as `handle-event(owner, …)`, and removing the owner closes them without an event. The host always draws the extension's name and id above a notification and next to a progress; the extension cannot style, hide or replace that line.

### type `host-error` = `host-error`


### enum `notification-level`

Cases: `info`, `success`, `warning`, `error`.

### record `notification-action`

| Field | Type | Notes |
|---|---|---|
| `id` | `string` | 1 to 64 bytes of `[a-z0-9._-]`, unique within the notification. Sent back in `notification-action-event`. |
| `label` | `string` | The button text: 1 to 24 bytes of plain text on one line. |

### record `notification`

| Field | Type | Notes |
|---|---|---|
| `owner` | `string` | The surface or decoration whose `handle-event` gets this notification's events. |
| `level` | `notification-level` |  |
| `title` | `string` | 1 to 80 bytes of plain text on one line. |
| `body` | `option<string>` | At most 500 bytes of plain text, at most 4 lines. No markdown, links or icons. |
| `actions` | `list<notification-action>` | At most 2. Drawn as buttons in list order; the first is the primary one. |
| `auto-dismiss` | `bool` | `true`: the host closes it after a time it chooses by level (6 s for `info` and `success`, 10 s for `warning` and `error`), paused while the pointer is over it or the window is in the background. `false`: it stays until the user or the extension closes it. |

### enum `progress-location`

Cases: `status-bar`, `panel`.

### record `progress-options`

| Field | Type | Notes |
|---|---|---|
| `owner` | `string` | The surface or decoration whose `handle-event` gets `progress-cancel-requested`; a panel for `location: panel`. |
| `title` | `string` | 1 to 80 bytes of plain text on one line. |
| `location` | `progress-location` |  |
| `cancellable` | `bool` | Draws a Cancel button that sends `progress-cancel-requested`. |

### variant `progress-value`

| Case | Payload | Notes |
|---|---|---|
| `indeterminate` |  |  |
| `percent` | `u8` | 0 to 100. |

### `notify: func(notification: notification) -> result<u64, host-error>`

Shows a notification in the active workspace window, without moving focus, and returns its id. Caps: 5 accepted calls per rolling 60 s and 3 open notifications per extension; past either, `limit-exceeded`, and the title is written to the log instead.

### `dismiss-notification: func(id: u64)`

Closes a notification of this extension; no event is sent. Unknown or closed ids are ignored.

### `progress-start: func(options: progress-options) -> result<u64, host-error>`

Starts a progress, indeterminate until the first `progress-update`, and returns its id. At most 4 running progresses per extension and 30 starts per rolling 60 s.

### `progress-update: func(id: u64, value: progress-value, message: option<string>) -> result<_, host-error>`

Sets the value and, when given, a one-line message (at most 120 bytes). The host redraws at most 10 times a second and keeps the last value. At most 64 updates per export call.

### `progress-end: func(id: u64)`

Ends a progress. Unknown or ended ids are ignored. Disabling, reloading or unloading the extension, or removing the owner, ends its progresses too.
