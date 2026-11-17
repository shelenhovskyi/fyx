# `fyx:ui` 0.2 candidates (integrator backlog)

`fyx:ui@0.1.0` stays frozen for the sprint. Every item below has a working workaround today; none
blocks Phase 1–4. Decide them together after Phase 4, as one 0.2 world.

| # | Change | Why | Today's workaround | Source |
|---|---|---|---|---|
| 1 | `invalidate-rows(node-id)` patch | rows change while the list's props do not | re-key the list node (remove + insert); host re-requests rows of touched lists | judge integration-1 #5, W4, W8 |
| 2 | `request-tick(ms)` import + `tick` export | `push-update` cannot drive timers: a component runs only inside a host call | none (no clocks/progress status items) | judge integration-1 #9 |
| 3 | Document row-local ids in the WIT | ids inside `list-row` subtrees are row-local, not surface-unique; clicks in rows are `row-click` | host never maps row ids; SDK `ROW_NODE_ID_BASE` | judge integration-1 #4 |
| 4 | Frozen icon list instead of upstream `IconName` strings | extension ABI must not follow upstream's icon enum | host parses `IconName::from_str`, fallback icon | judge integration-1 #12 |
| 5 | `surface-kind::modal` | QuickPick / InputBox | panel at the bottom dock | W8 |
| 6 | Runtime add/remove of surfaces | VS Code creates pickers/output channels after `surfaces()` | sidecar notification `fyx/surfacesChanged` | W8 |
| 7 | `ui-event::dismissed` | user closed a surface | sidecar request `fyx/surfaceClosed` | W8 |
| 8 | `indent` prop on rows, `tooltip` on button | tree views, icon buttons | flattened labels, no tooltip | W8 |

Details for 5–8: `docs/vscode-coverage.md` ("Schema gaps").
