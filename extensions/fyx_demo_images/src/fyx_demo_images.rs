//! Fyx demo 8: images (`fyx:ui` 0.3 `image` nodes and `fyx:ui/images`,
//! `docs/fyx/wit-0.3/design.md`, section 12). Images need the staged 0.3 world: the default 0.1
//! build shows a panel that says so; build with `--features fyx-ui-0-3` for the real thing.
//!
//! - Panel `"images"` (left dock, title "Images", icon `image`, default size 340):
//!   - assets from the package's `assets/` folder: the Fyx logo as an SVG, plain and tinted
//!     with the accent colour, and a 256×160 PNG in a 96×96 box, `contain` beside `cover`;
//!   - a bar chart drawn at run time as an SVG and registered with `add-image`, with a
//!     `Shuffle` button that draws a new one, swaps the node to it and frees the old one;
//!   - an asset that does not exist, which the renderer draws as its alt text;
//!   - a status line, empty until a host call fails;
//!   - a list of file kinds with an SVG thumbnail per row, tinted like the host's own icons.

use ui_ext_sdk::*;

const PANEL: &str = "images";

#[cfg(feature = "fyx-ui-0-3")]
pub use images::*;

#[cfg(feature = "fyx-ui-0-3")]
mod images {
    use ui_ext_sdk::*;

    use super::PANEL;

    pub const LOGO: &str = "assets/logo.svg";
    pub const SAMPLE: &str = "assets/sample.png";
    pub const MISSING: &str = "assets/missing.png";
    pub const MISSING_ALT: &str = "A picture that is not in the package (this is its alt text)";
    const ROW_HEIGHT: u16 = 28;
    const CHART_WIDTH: u32 = 240;
    const CHART_HEIGHT: u32 = 96;
    pub const BARS: usize = 8;

    /// The list rows: a thumbnail from `assets/kinds/` and a label.
    pub const KINDS: &[(&str, &str, ColorToken)] = &[
        (
            "assets/kinds/source.svg",
            "Source files",
            ColorToken::TextAccent,
        ),
        ("assets/kinds/doc.svg", "Documents", ColorToken::Text),
        ("assets/kinds/test.svg", "Tests", ColorToken::Success),
    ];

    /// A small deterministic generator, so the tests know the charts.
    #[derive(Clone, Debug)]
    pub struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) as u32
        }
    }

    /// Bar heights from 10 to 100 percent.
    pub fn bars(random: &mut Lcg) -> [u32; BARS] {
        std::array::from_fn(|_| 10 + random.next() % 91)
    }

    /// The chart as a plain SVG, one colour so the host may tint it; without a tint it is drawn
    /// in its own colours.
    pub fn chart_svg(bars: &[u32; BARS]) -> String {
        let bar_width = CHART_WIDTH / BARS as u32;
        let mut svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{CHART_WIDTH}\" \
             height=\"{CHART_HEIGHT}\" viewBox=\"0 0 {CHART_WIDTH} {CHART_HEIGHT}\">"
        );
        for (index, percent) in bars.iter().enumerate() {
            let height = CHART_HEIGHT * percent / 100;
            let x = index as u32 * bar_width + 2;
            let y = CHART_HEIGHT - height;
            svg.push_str(&format!(
                "<rect x=\"{x}\" y=\"{y}\" width=\"{}\" height=\"{height}\" rx=\"2\" \
                 fill=\"#3b82f6\"/>",
                bar_width - 4
            ));
        }
        svg.push_str("</svg>");
        svg
    }

    fn host_error(error: HostError) -> String {
        format!("{}: {}", error.code.name(), error.message)
    }

    struct PanelNodes {
        chart: NodeId,
        shuffle: NodeId,
        status: NodeId,
        list: NodeId,
    }

    pub struct Images {
        random: Lcg,
        bars: [u32; BARS],
        chart: Option<GeneratedImage>,
        status: String,
        panel: SurfaceTree,
        nodes: Option<PanelNodes>,
    }

    impl Images {
        pub fn bars(&self) -> [u32; BARS] {
            self.bars
        }

        pub fn chart(&self) -> Option<GeneratedImage> {
            self.chart
        }

        pub fn shuffle_button(&self) -> Option<NodeId> {
            self.nodes.as_ref().map(|nodes| nodes.shuffle)
        }

        pub fn list_node(&self) -> Option<NodeId> {
            self.nodes.as_ref().map(|nodes| nodes.list)
        }

        /// The chart node's props: the generated image, or a text while `add-image` fails.
        fn chart_props(&self) -> Props {
            match self.chart {
                Some(chart) => ImageProps::generated(chart, "A bar chart of eight random values")
                    .size(CHART_WIDTH, CHART_HEIGHT)
                    .into(),
                None => TextProps::new("The chart could not be drawn")
                    .color(ColorToken::TextMuted)
                    .into(),
            }
        }

        fn status_label(&self) -> TextProps {
            TextProps::new(self.status.clone()).color(ColorToken::Error)
        }

        /// Registers the current bars as a new image and frees the old one afterwards, so the
        /// node never shows a freed handle once the reply applies.
        fn draw_chart(&mut self) {
            match add_image(ImageFormat::Svg, chart_svg(&self.bars).as_bytes()) {
                Ok(image) => {
                    if let Some(old) = self.chart.replace(image) {
                        remove_image(old);
                    }
                    self.status.clear();
                }
                Err(error) => {
                    if let Some(old) = self.chart.take() {
                        remove_image(old);
                    }
                    self.status = host_error(error);
                }
            }
        }

        fn shuffle(&mut self) -> Result<Update, String> {
            self.bars = bars(&mut self.random);
            self.draw_chart();
            let Some(nodes) = &self.nodes else {
                return Err(TreeError::NotMounted.into());
            };
            let (chart, status) = (nodes.chart, nodes.status);
            self.panel.set_props(chart, self.chart_props())?;
            self.panel.set_props(status, self.status_label())?;
            Ok(self.panel.take_update())
        }

        fn section(tree: &mut TreeBuilder, title: &str) {
            tree.text(
                TextProps::new(title.to_string())
                    .bold()
                    .color(ColorToken::TextMuted),
            );
        }

        fn mount_panel(&mut self) -> Result<Vec<Node>, String> {
            if self.chart.is_none() {
                self.draw_chart();
            }
            let mut tree = std::mem::take(&mut self.panel);
            let mut nodes = None;
            let chart_props = self.chart_props();
            let status = self.status_label();
            let count = KINDS.len() as u32;
            let mounted = tree.mount(|tree| {
                tree.flex(
                    FlexProps::column()
                        .gap(8)
                        .padding(8)
                        .grow()
                        .align(Align::Stretch),
                    |tree| {
                        Self::section(tree, "SVG asset, plain and tinted");
                        tree.flex(FlexProps::row().gap(12).align(Align::Center), |tree| {
                            tree.image(ImageProps::asset(LOGO, "The Fyx logo").size(48, 48));
                            tree.image(
                                ImageProps::asset(LOGO, "The Fyx logo in the accent colour")
                                    .size(48, 48)
                                    .tint(ColorToken::TextAccent),
                            );
                        });
                        Self::section(tree, "PNG asset in a 96×96 box: contain, cover");
                        tree.flex(FlexProps::row().gap(12), |tree| {
                            tree.image(
                                ImageProps::asset(SAMPLE, "A gradient with a circle and a square")
                                    .size(96, 96),
                            );
                            tree.image(
                                ImageProps::asset(SAMPLE, "The same picture, cropped to the box")
                                    .size(96, 96)
                                    .fit(ImageFit::Cover),
                            );
                        });
                        Self::section(tree, "Generated at run time");
                        let chart = tree.node(chart_props);
                        let shuffle = tree.button(
                            ButtonProps::new("Shuffle")
                                .icon(Icon::Refresh)
                                .style(ButtonStyle::Filled)
                                .tooltip("Draw the chart again with new values"),
                        );
                        Self::section(tree, "A missing asset shows its alt text");
                        tree.image(ImageProps::asset(MISSING, MISSING_ALT).size(96, 48));
                        let status = tree.text(status);
                        Self::section(tree, "Thumbnails in list rows");
                        let list = tree.list(ListProps::new(count, ROW_HEIGHT));
                        nodes = Some(PanelNodes {
                            chart,
                            shuffle,
                            status,
                            list,
                        });
                    },
                );
            });
            self.panel = tree;
            let mounted = mounted?;
            self.nodes = nodes;
            Ok(mounted)
        }
    }

    impl Extension for Images {
        fn new() -> Self {
            let mut random = Lcg(0x5eed);
            Self {
                bars: bars(&mut random),
                random,
                chart: None,
                status: String::new(),
                panel: SurfaceTree::default(),
                nodes: None,
            }
        }

        fn surfaces(&self) -> Vec<Surface> {
            vec![Surface::panel(
                PANEL,
                "Images",
                DockPosition::Left,
                Icon::Image,
                340,
            )]
        }

        fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
            if surface_id != PANEL {
                return Err(format!("unknown surface {surface_id:?}"));
            }
            self.mount_panel()
        }

        fn handle_event(&mut self, surface_id: &str, event: UiEvent) -> Result<Update, String> {
            if surface_id != PANEL {
                return Err(format!("unknown surface {surface_id:?}"));
            }
            self.panel.apply_event(&event);
            match event {
                UiEvent::Click(id) if Some(id) == self.shuffle_button() => self.shuffle(),
                _ => Ok(Update::Unchanged),
            }
        }

        fn list_rows(
            &mut self,
            surface_id: &str,
            list_node: NodeId,
            start: u32,
            count: u32,
        ) -> Vec<ListRow> {
            if surface_id != PANEL || self.list_node() != Some(list_node) {
                return Vec::new();
            }
            list_rows(start, count, KINDS.len() as u32, |index, tree| {
                let (path, label, color) = KINDS[index as usize];
                tree.flex(FlexProps::row().gap(8).align(Align::Center), |tree| {
                    tree.image(
                        ImageProps::asset(path, format!("{label} icon"))
                            .size(16, 16)
                            .tint(color),
                    );
                    tree.text(TextProps::new(label));
                });
            })
        }
    }
}

/// What a 0.1 or 0.2 build shows: images need the 0.3 world.
#[cfg(not(feature = "fyx-ui-0-3"))]
pub struct NeedsImages;

#[cfg(not(feature = "fyx-ui-0-3"))]
pub const NEEDS_0_3: &str =
    "Images need fyx:ui 0.3. Build this demo with script/fyx-build-demos --fyx-ui-0-3.";

#[cfg(not(feature = "fyx-ui-0-3"))]
impl Extension for NeedsImages {
    fn new() -> Self {
        Self
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            PANEL,
            "Images",
            DockPosition::Left,
            Icon::Image,
            340,
        )]
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        if surface_id != PANEL {
            return Err(format!("unknown surface {surface_id:?}"));
        }
        let mut ids = IdAllocator::new();
        let mut tree = TreeBuilder::new(&mut ids);
        tree.flex(FlexProps::column().gap(8).padding(8), |tree| {
            tree.text(TextProps::new(NEEDS_0_3).color(ColorToken::TextMuted));
        });
        Ok(tree.finish())
    }
}

// fyx:ui 0.1 by default, which is what every Fyx runs; `--features fyx-ui-0-2` builds the 0.2
// world (still without images) and `--features fyx-ui-0-3` the staged 0.3 world with them.
#[cfg(not(any(feature = "fyx-ui-0-2", feature = "fyx-ui-0-3")))]
register_extension!(NeedsImages, v0_1);
#[cfg(all(feature = "fyx-ui-0-2", not(feature = "fyx-ui-0-3")))]
register_extension!(NeedsImages, v0_2);
#[cfg(feature = "fyx-ui-0-3")]
register_extension!(Images);

#[cfg(all(test, feature = "fyx-ui-0-3"))]
mod tests {
    use super::*;

    fn mounted() -> Images {
        take_host_effects();
        let mut images = Images::new();
        images.mount(PANEL).expect("mount");
        images
    }

    #[test]
    fn the_chart_is_a_plain_svg() {
        let svg = chart_svg(&[10, 20, 30, 40, 50, 60, 70, 100]);
        assert!(svg.starts_with("<svg "), "{svg}");
        assert_eq!(svg.matches("<rect ").count(), BARS);
        assert!(!svg.contains("<image") && !svg.contains("<script"));
        assert!(svg.len() < 4096);
    }

    #[test]
    fn mounting_registers_the_chart_once() {
        let mut images = mounted();
        let effects = take_host_effects();
        assert!(
            matches!(
                &effects[..],
                [HostEffect::AddImage {
                    format: ImageFormat::Svg,
                    ..
                }]
            ),
            "{effects:?}"
        );
        let chart = images.chart().expect("chart");
        images.mount(PANEL).expect("remount");
        assert_eq!(images.chart(), Some(chart), "a remount keeps the image");
        assert!(take_host_effects().is_empty());
    }

    #[test]
    fn shuffle_swaps_the_image_and_frees_the_old_one() {
        let mut images = mounted();
        take_host_effects();
        let old = images.chart().expect("chart");
        let old_bars = images.bars();
        let shuffle = images.shuffle_button().expect("mounted");
        let update = images
            .handle_event(PANEL, UiEvent::Click(shuffle))
            .expect("shuffle");
        let new = images.chart().expect("chart");
        assert_ne!(new, old);
        assert_ne!(images.bars(), old_bars);
        let effects = take_host_effects();
        assert!(
            matches!(
                &effects[..],
                [HostEffect::AddImage { image, .. }, HostEffect::RemoveImage(removed)]
                    if *image == new && *removed == old
            ),
            "{effects:?}"
        );
        let Update::Patches(patches) = update else {
            panic!("{update:?}");
        };
        assert!(patches.iter().any(|patch| matches!(
            patch,
            Patch::SetProps((_, Props::Image(ImageProps { source: ImageSource::Generated(handle), .. })))
                if *handle == new.handle()
        )));
    }

    #[test]
    fn rows_have_tinted_svg_thumbnails() {
        let mut images = mounted();
        let list = images.list_node().expect("mounted");
        let rows = images.list_rows(PANEL, list, 0, 10);
        assert_eq!(rows.len(), KINDS.len());
        for row in rows {
            assert!(row.nodes.iter().any(|node| matches!(
                &node.props,
                Props::Image(ImageProps { tint: Some(_), source: ImageSource::Asset(path), .. })
                    if path.ends_with(".svg")
            )));
        }
    }
}
