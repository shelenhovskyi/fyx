# `fyx:ui@0.2.0` WIT reference

Generated from `crates/ui_ext/wit/since_v0.2.0` by `script/fyx-api-docs`; do not edit by hand. A test fails when this page and its source disagree.

## World `ui-extension`

Imports:

- interface `fyx:ui/types@0.2.0`
- interface `fyx:ui/host@0.2.0`
- interface `fyx:ui/decorations@0.2.0`
- interface `fyx:ui/notifications@0.2.0`
- type `surface`
- type `node`
- type `node-id`
- type `ui-event`
- type `update`
- type `list-row`

Exports:

- `surfaces: func() -> list<surface>`: Called once after instantiation; declares the static surfaces. More can be added with `add-surface`.
- `mount: func(surface-id: string) -> result<list<node>, string>`: The whole tree of a surface (at most 16384 nodes): when a panel or status item is added, each time a modal is shown, and again whenever the host resyncs after an update that did not fit, which for a decoration replaces the tree it was placed with.
- `handle-event: func(surface-id: string, event: ui-event) -> result<update, string>`: At most one call per input event; the host debounces scroll, hover and typing.
- `list-rows: func(surface-id: string, list-node: node-id, start: u32, count: u32) -> list<list-row>`
- `tick: func(surface-id: string) -> result<update, string>`: The tick `request-tick` asked for. Never called while the surface is mounting or failed, nor for a modal that is not shown. The update applies to `surface-id`, like an event's.

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
| `accessible-label` | `option<string>` | The accessible name, read by screen readers; without it, `label` when it is not empty, then the tooltip, then the icon's name. Set it for an icon-only button. At most 256 bytes; the host truncates longer text. |

### record `input-props`

| Field | Type | Notes |
|---|---|---|
| `value` | `string` |  |
| `placeholder` | `string` |  |
| `secret` | `bool` | Masks the text on screen (passwords, tokens). The value still reaches the extension in `input-changed` and `input-submitted`; the host never logs it. |
| `label` | `option<string>` | The accessible name, read by screen readers; without it, the placeholder is used. At most 256 bytes; the host truncates longer text. |

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

Caps: at most 2048 patches in one `patches`; at most 16384 nodes in a surface tree, which is also the cap for a `replace` and for `mount`, so a whole surface can always be resent; at most 2048 nodes in one list row; at most 64 levels from a root to its deepest node; at most 8 MiB of text in a surface tree's props. An update that breaks a cap or does not fit the tree is rejected whole, and the host mounts the surface again.

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

### variant `ui-event`

| Case | Payload | Notes |
|---|---|---|
| `click` | `node-id` |  |
| `input-changed` | `tuple<node-id, string>` |  |
| `input-submitted` | `tuple<node-id, string>` |  |
| `row-click` | `tuple<node-id, u32>` | The list node and the row index within it. A click on anything inside a row, buttons included, is a `row-click`. |
| `row-selected` | `tuple<node-id, u32>` | The user moved a list's selection with the keyboard (modals only). The host has already set `selected` in its tree; the extension need not echo it. |
| `dismissed` | `dismiss-reason` | The host closed a modal surface; its tree is gone. The reply must be `unchanged`. |
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

### variant `surface-kind`

Panels and status items are mounted when they are added and live until removed. A modal is mounted each time it is shown and its tree is dropped when it closes.

| Case | Payload | Notes |
|---|---|---|
| `panel` | `panel-descriptor` |  |
| `status-item` | `status-item-descriptor` |  |
| `modal` | `modal-descriptor` |  |

### enum `decoration-placement`

Cases: `above`, `below`.

### record `decoration-location`

Where an editor decoration goes. Only checked for syntax when `place-decoration` is called; the host anchors it in every open buffer of that file, and in buffers opened later.

| Field | Type | Notes |
|---|---|---|
| `path` | `string` | Relative to a worktree root, `/`-separated, without `..` (`src/main.rs`). Matched against every worktree of every open workspace. |
| `line` | `u32` | Zero-based row when the decoration is anchored. The host turns it into a CRDT anchor per buffer, so from then on the block moves with the line; rows past the end clamp to the last row. If the anchored line is deleted, the block stays where the anchor collapses to. |
| `placement` | `decoration-placement` |  |
| `height-lines` | `option<u16>` | Block height in editor lines; `none` lets the host estimate it from the tree. |

### record `surface`

| Field | Type | Notes |
|---|---|---|
| `id` | `string` | 1 to 64 bytes of `a-z`, `0-9`, `-`, `_`, `.`; unique among the extension's live surfaces. |
| `title` | `string` |  |
| `kind` | `surface-kind` |  |

## Interface `host`

Functions the extension may call on the host. Every function with an effect (an update, a surface change, a tick) is checked when it is called and returns the check's result at once, but the effect itself happens after the current export call returns, in the order the extension called them, and before that call's own return value is applied.

### type `update` = `update`


### type `surface` = `surface`


### enum `host-error-code`

Cases: `unknown-surface`, `limit-exceeded`, `invalid-argument`, `not-permitted`, `unavailable`.

### record `host-error`

| Field | Type | Notes |
|---|---|---|
| `code` | `host-error-code` |  |
| `message` | `string` |  |

### enum `log-level`

Cases: `trace`, `debug`, `info`, `warn`, `error`.

### `push-update: func(surface-id: string, update: update) -> result<_, host-error>`

Sends an update the extension initiated (a push to another surface, a tick's side effects). `ok` means the update passed the checks the host can make at once (the surface exists, the caps); it can still fail to fit the tree when it is applied, which remounts the surface. On `err` nothing was queued: the extension's mirror of that surface is now ahead of the host and must be resent whole. At most 64 pushes per export call, carrying at most 32768 patches plus nodes together (two whole trees); past that, `limit-exceeded`.

### `request-tick: func(surface-id: string, delay-ms: u32) -> result<_, host-error>`

Asks for one `tick(surface-id)` call at least `delay-ms` after the current call returns. One pending tick per surface: a new request replaces the earlier one. The host clamps the delay up to 100 ms, and to 1000 ms while Fyx is in the background. A tick that falls due while its panel is hidden waits until the panel is shown and is then delivered once: a panel opened hours later gets one tick, not a backlog (design.md, "Ticks"). A delay over 24 hours is `invalid-argument`. Decorations cannot request ticks.

### `cancel-tick: func(surface-id: string)`

Drops the pending tick of a surface, if any.

### `add-surface: func(surface: surface) -> result<_, host-error>`

Adds a surface at runtime. The manifest must declare the capability of its kind. Panels and status items are added to every workspace window and mounted; a modal waits for `show-surface`. At most 32 live surfaces per extension.

### `remove-surface: func(surface-id: string) -> result<_, host-error>`

Removes a live surface, static or runtime: its containers close in every window, its tree, pending tick and queued events are dropped, and no event is sent. The id may be added again afterwards, as a new surface.

### `show-surface: func(surface-id: string) -> result<_, host-error>`

Opens a modal in the active workspace window and focuses it, or reveals a panel there (opens its dock and activates it, without moving focus). Showing a modal needs a click or Enter: it is allowed during `handle-event` for `click`, `row-click`, `input-submitted` or a clicked `notification-action`, or within 5 s after one, once. Typing (`input-changed`), `row-selected` and `dismissed` do not count. Showing a surface that is already shown does nothing.

### `hide-surface: func(surface-id: string) -> result<_, host-error>`

Closes a modal (no `dismissed` event) or hides a panel's dock if that panel is active.

### `log: func(level: log-level, message: string)`

## Interface `decorations`

Editor decorations: retained `fyx:ui` trees drawn as blocks above or below a buffer line (design.md, "L"). Needs the `ui:editor-decorations` capability; without it every call returns `not-permitted`. Both functions follow the effect rule of `host`. A decoration id lives in the same namespace as the extension's surface ids and follows the same id rule, so it must not equal a live surface id. Once placed, the id works like a surface id for `push-update`, `handle-event` (clicks, input), `list-rows` and `mount` (the host calls `mount(id)` to resync a decoration whose update did not fit), but not for `request-tick`, `show-surface` or `remove-surface`. At most 512 live decorations per extension, not counted against the 32 surfaces.

### type `node` = `node`


### type `decoration-location` = `decoration-location`


### type `host-error` = `host-error`


### `place-decoration: func(id: string, location: decoration-location, nodes: list<node>) -> result<_, host-error>`

Places a decoration with the initial tree `nodes` (at most 16384 nodes, parents first). Placing an id that is already a live decoration moves it and replaces its tree, like a fresh mount. At most 512 `place-decoration` and `remove-decoration` calls per export call; past that, `limit-exceeded`.

### `remove-decoration: func(id: string) -> result<_, host-error>`

Removes a live decoration in every editor; no event is sent. The id may be placed again.

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

Shows a notification in the active workspace window, without moving focus, and returns its id. Caps: 5 accepted calls per rolling 60 s and 3 open notifications per extension, and 4 `notify` and `dismiss-notification` calls together per export call; past any, `limit-exceeded`, and the title is written to the log instead.

### `dismiss-notification: func(id: u64)`

Closes a notification of this extension; no event is sent. Unknown or closed ids are ignored.

### `progress-start: func(options: progress-options) -> result<u64, host-error>`

Starts a progress, indeterminate until the first `progress-update`, and returns its id. At most 4 running progresses per extension and 30 starts per rolling 60 s.

### `progress-update: func(id: u64, value: progress-value, message: option<string>) -> result<_, host-error>`

Sets the value and, when given, a one-line message (at most 120 bytes). The host redraws at most 10 times a second and keeps the last value. At most 64 updates per export call.

### `progress-end: func(id: u64)`

Ends a progress. Unknown or ended ids are ignored. Disabling, reloading or unloading the extension, or removing the owner, ends its progresses too.
