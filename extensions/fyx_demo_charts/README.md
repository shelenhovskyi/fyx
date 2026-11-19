# Charts demo (`fyx-demo-charts`)

A Fyx UI extension that draws charts on `canvas` nodes from the staged `fyx:ui` 0.3
([design, section 13](../../docs/fyx/wit-0.3/design.md)).

The left-dock panel **Charts** has:

- a bar chart of a mock week of commits, one data colour per bar. Click a bar to select it: it
  gets an accent outline and the line below reads `Wed: 42 commits`. A click elsewhere clears the
  selection. The canvas scales down when the dock is narrower than the chart;
- a sparkline of a mock load value. A tick every half second adds a sample; **Pause** and
  **Resume** stop and restart it. Fyx pauses the ticks while the panel is hidden.

It uses the SDK's `DrawList`, `Path::polyline` and `CanvasHits` (for click-to-select) and
declares `ui:canvas`.

**Canvases need `fyx:ui` 0.3.** The default build exports 0.1, which Fyx runs today, and only says
that charts need 0.3. Build the 0.3 component with `script/fyx-build-demos --fyx-ui-0-3`, which
puts it in `<target>/fyx-demos-v0_3/fyx_demo_charts/`; a dev build runs it there. Test it headless
with `fyx-ext test`, or with `Harness::load_package` and `Harness::canvas_click`.

How to write your own: [docs/fyx/extension-authoring.md](../../docs/fyx/extension-authoring.md).
