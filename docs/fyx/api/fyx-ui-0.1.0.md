# `fyx:ui@0.1.0` WIT reference

Generated from `crates/ui_ext/wit` by `script/fyx-api-docs`; do not edit by hand. A test fails when this page and its source disagree.

## World `ui-extension`

Imports:

- interface `fyx:ui/types@0.1.0`
- interface `fyx:ui/host@0.1.0`
- type `surface`
- type `node`
- type `node-id`
- type `ui-event`
- type `update`
- type `list-row`

Exports:

- `surfaces: func() -> list<surface>`: Called once after instantiation; declares every surface the extension contributes.
- `mount: func(surface-id: string) -> result<list<node>, string>`: Initial tree when the surface is first shown. Not called per frame: the host retains the tree.
- `handle-event: func(surface-id: string, event: ui-event) -> result<update, string>`: At most one call per input event; the host debounces scroll, hover and typing.
- `list-rows: func(surface-id: string, list-node: node-id, start: u32, count: u32) -> list<list-row>`

## Interface `types`

### type `node-id` = `u32`

Chosen by the extension, unique within one surface. The host keys event routing and patches on it, so an id must not be reused for a different node while the surface lives.

### enum `color-token`

Colours are semantic theme slots, never raw values, so every extension follows the active theme (the Zellij approach).

Cases: `text`, `text-muted`, `text-accent`, `error`, `warning`, `success`, `border`, `surface`, `elevated-surface`.

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
| `icon` | `option<string>` |  |
| `style` | `button-style` |  |
| `disabled` | `bool` |  |

### record `input-props`

| Field | Type | Notes |
|---|---|---|
| `value` | `string` |  |
| `placeholder` | `string` |  |

### record `list-props`

Virtualized: the host asks for rows with `list-rows` as they scroll into view, mirroring GPUI's `uniform_list`. `row-height` is in pixels and fixed for every row.

| Field | Type | Notes |
|---|---|---|
| `row-count` | `u32` |  |
| `row-height` | `u16` |  |

### record `table-props`

| Field | Type | Notes |
|---|---|---|
| `columns` | `list<string>` |  |
| `row-count` | `u32` |  |

### record `icon-props`

| Field | Type | Notes |
|---|---|---|
| `name` | `string` |  |
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

WIT types cannot be recursive, so a tree travels as a flat `list<node>` in which every node names its parent. Parents precede children; siblings keep list order.

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

### variant `patch`

| Case | Payload | Notes |
|---|---|---|
| `set-props` | `tuple<node-id, props>` |  |
| `insert` | `insert-op` |  |
| `remove` | `node-id` | Removes the node and its whole subtree. |

### variant `update`

| Case | Payload | Notes |
|---|---|---|
| `unchanged` |  |  |
| `replace` | `list<node>` |  |
| `patches` | `list<patch>` |  |

### record `list-row`

| Field | Type | Notes |
|---|---|---|
| `index` | `u32` |  |
| `nodes` | `list<node>` | A subtree whose single root has `parent: none`. |

### variant `ui-event`

| Case | Payload | Notes |
|---|---|---|
| `click` | `node-id` |  |
| `input-changed` | `tuple<node-id, string>` |  |
| `input-submitted` | `tuple<node-id, string>` |  |
| `row-click` | `tuple<node-id, u32>` | The list node and the row index within it. |

### enum `dock-position`

Cases: `left`, `right`, `bottom`.

### record `panel-descriptor`

| Field | Type | Notes |
|---|---|---|
| `position` | `dock-position` |  |
| `icon` | `string` |  |
| `default-size` | `u32` |  |

### enum `status-bar-side`

Cases: `left`, `right`.

### record `status-item-descriptor`

| Field | Type | Notes |
|---|---|---|
| `side` | `status-bar-side` |  |
| `priority` | `s32` | Higher sorts closer to the edge. |

### variant `surface-kind`

| Case | Payload | Notes |
|---|---|---|
| `panel` | `panel-descriptor` |  |
| `status-item` | `status-item-descriptor` |  |

### record `surface`

| Field | Type | Notes |
|---|---|---|
| `id` | `string` |  |
| `title` | `string` |  |
| `kind` | `surface-kind` |  |

## Interface `host`

Functions the extension may call on the host.

### type `update` = `update`


### enum `log-level`

Cases: `trace`, `debug`, `info`, `warn`, `error`.

### `push-update: func(surface-id: string, update: update)`

Extension-initiated update, during a host call only (for example to update another surface); timers need 0.2 `request-tick`, see tasks/wit-0.2.md item 2. The host coalesces pushes and applies them on the next frame; it never calls back into the extension while rendering.

### `log: func(level: log-level, message: string)`
