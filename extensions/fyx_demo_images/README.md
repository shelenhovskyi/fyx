# Images demo (`fyx-demo-images`)

A Fyx UI extension that shows `image` nodes and `fyx:ui/images` from the staged `fyx:ui` 0.3
([design, section 12](../../docs/fyx/wit-0.3/design.md)).

The left-dock panel **Images** has:

- the Fyx logo from `assets/logo.svg`, plain and tinted with the accent colour (tint is for SVG
  only);
- `assets/sample.png` (256×160) in a 96×96 box, scaled to fit (`contain`) and cropped to fill
  (`cover`);
- a bar chart the extension draws at run time as an SVG and registers with `add_image`.
  **Shuffle** draws a new one, points the node at it and frees the old image;
- an image of `assets/missing.png`, which is not in the package, so the renderer draws its alt
  text;
- a list with an SVG thumbnail from `assets/kinds/` in each row.

The extension declares `ui:images` in `extension.toml`; images are panel-only.

**Images need `fyx:ui` 0.3.** The default build exports 0.1, which Fyx runs today, and only says
that images need 0.3. Build the 0.3 component with `script/fyx-build-demos --fyx-ui-0-3`. It goes
to `<target>/fyx-demos-v0_3/fyx_demo_images/` with the manifest and the assets. A dev build runs
it there.

Test it headless with `fyx-ext test`, or with `Harness::load_package`, which also checks the assets
as the renderer's decoder would.

How to write your own: [docs/fyx/extension-authoring.md](../../docs/fyx/extension-authoring.md).
