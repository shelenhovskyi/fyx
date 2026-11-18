//! Fyx demo 9: charts on `canvas` nodes (`fyx:ui` 0.3, `docs/fyx/wit-0.3/design.md` §13, plan
//! 4.15). Canvases need the staged 0.3 world: the default 0.1 build shows a panel that says so;
//! build with `--features fyx-ui-0-3` for the real thing.
//!
//! - Panel `"charts"` (left dock, title "Charts", icon `table`, default size 320):
//!   - a bar chart of a mock week of commits, one data colour per bar. It is `clickable` and
//!     `scale-down`: a click on a bar selects it (drawn with an accent outline), a click
//!     elsewhere clears the selection, and the text below reads `Wed: 42 commits`;
//!   - a sparkline of the last 60 samples of a mock load value. A tick every 500 ms adds a
//!     sample while it runs; `Pause` and `Resume` stop and restart it.

use ui_ext_sdk::*;

const PANEL: &str = "charts";

#[cfg(feature = "fyx-ui-0-3")]
pub use charts::*;

#[cfg(feature = "fyx-ui-0-3")]
mod charts {
    use ui_ext_sdk::*;

    use super::PANEL;

    pub const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    pub const COMMITS: [u32; 7] = [12, 30, 42, 18, 25, 8, 15];
    pub const CHART_WIDTH: u32 = 280;
    pub const CHART_HEIGHT: u32 = 140;
    const BAR_TOP: f32 = 12.0;
    const BAR_BOTTOM: f32 = 116.0;
    const LABEL_BASELINE: f32 = 132.0;
    const SLOT: f32 = CHART_WIDTH as f32 / DAYS.len() as f32;
    const BAR_WIDTH: f32 = 24.0;

    pub const SPARK_WIDTH: u32 = 280;
    pub const SPARK_HEIGHT: u32 = 48;
    pub const SAMPLES: usize = 60;
    pub const TICK_MS: u32 = 500;

    /// The box of bar `day`: its height follows the value, against the week's maximum.
    pub fn bar(day: usize) -> Rect {
        let max = COMMITS.iter().copied().max().unwrap_or(1) as f32;
        let height = (BAR_BOTTOM - BAR_TOP) * COMMITS[day] as f32 / max;
        let x = day as f32 * SLOT + (SLOT - BAR_WIDTH) / 2.0;
        Rect::new(x, BAR_BOTTOM - height, BAR_WIDTH, height)
    }

    /// The bar chart and its hit regions: a click anywhere in a bar's column selects that day.
    pub fn bar_chart(selected: Option<usize>) -> (CanvasProps, CanvasHits<usize>) {
        let mut draw = DrawList::new();
        let mut hits = CanvasHits::new();
        for day in 0..DAYS.len() {
            let stroke = (selected == Some(day))
                .then(|| Stroke::new(Paint::token(ColorToken::TextAccent), 2.0));
            draw.rect(bar(day), Some(Paint::data(DataColor::cycle(day))), stroke);
            draw.text_with(
                pt(day as f32 * SLOT + SLOT / 2.0, LABEL_BASELINE),
                DAYS[day],
                Paint::token(ColorToken::TextMuted),
                TextStyle::default().anchor(TextAnchor::Middle),
            );
            hits.add(Rect::new(day as f32 * SLOT, 0.0, SLOT, BAR_BOTTOM), day);
        }
        draw.line(
            pt(0.0, BAR_BOTTOM),
            pt(CHART_WIDTH as f32, BAR_BOTTOM),
            Stroke::new(Paint::token(ColorToken::Border), 1.0),
        );
        let canvas = CanvasProps::new(
            CHART_WIDTH,
            CHART_HEIGHT,
            "Commits per day this week, Monday to Sunday",
        )
        .draw(draw)
        .clickable()
        .fit(CanvasFit::ScaleDown);
        (canvas, hits)
    }

    /// The last samples as a line across the box, values 0 to 100.
    pub fn sparkline(samples: &[u8]) -> CanvasProps {
        let mut draw = DrawList::new();
        let step = SPARK_WIDTH as f32 / (SAMPLES - 1) as f32;
        let height = SPARK_HEIGHT as f32 - 4.0;
        draw.line(
            pt(0.0, SPARK_HEIGHT as f32 - 1.0),
            pt(SPARK_WIDTH as f32, SPARK_HEIGHT as f32 - 1.0),
            Stroke::new(Paint::token(ColorToken::Border), 1.0),
        );
        if samples.len() >= 2 {
            let start = SAMPLES - samples.len();
            draw.path(
                Path::polyline(samples.iter().enumerate().map(|(index, value)| {
                    pt(
                        (start + index) as f32 * step,
                        2.0 + height * (100 - *value) as f32 / 100.0,
                    )
                }))
                .stroke(Stroke::new(Paint::data(DataColor::Series1), 1.5)),
            );
        }
        CanvasProps::new(SPARK_WIDTH, SPARK_HEIGHT, "Load over the last 30 seconds")
            .draw(draw)
            .fit(CanvasFit::ScaleDown)
    }

    /// A small deterministic random walk, so the tests know the samples.
    #[derive(Clone, Debug)]
    pub struct Walk {
        state: u64,
        value: u8,
    }

    impl Walk {
        pub fn new() -> Self {
            Self {
                state: 0x5eed,
                value: 50,
            }
        }

        pub fn next(&mut self) -> u8 {
            self.state = self
                .state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let delta = (self.state >> 60) as i16 - 8;
            self.value = (i16::from(self.value) + delta).clamp(0, 100) as u8;
            self.value
        }
    }

    impl Default for Walk {
        fn default() -> Self {
            Self::new()
        }
    }

    struct PanelNodes {
        chart: NodeId,
        selection: NodeId,
        spark: NodeId,
        toggle: NodeId,
    }

    pub struct Charts {
        selected: Option<usize>,
        hits: CanvasHits<usize>,
        samples: Vec<u8>,
        walk: Walk,
        running: bool,
        panel: SurfaceTree,
        nodes: Option<PanelNodes>,
    }

    fn selection_text(selected: Option<usize>) -> TextProps {
        match selected {
            Some(day) => TextProps::new(format!("{}: {} commits", DAYS[day], COMMITS[day])),
            None => TextProps::new("Click a bar").color(ColorToken::TextMuted),
        }
    }

    fn toggle_button(running: bool) -> ButtonProps {
        if running {
            ButtonProps::new("Pause")
                .icon(Icon::Pause)
                .tooltip("Stop adding samples")
        } else {
            ButtonProps::new("Resume")
                .icon(Icon::Play)
                .tooltip("Add a sample every half second")
        }
    }

    impl Charts {
        pub fn selected(&self) -> Option<usize> {
            self.selected
        }

        pub fn samples(&self) -> &[u8] {
            &self.samples
        }

        pub fn is_running(&self) -> bool {
            self.running
        }

        pub fn chart_node(&self) -> Option<NodeId> {
            self.nodes.as_ref().map(|nodes| nodes.chart)
        }

        pub fn toggle_node(&self) -> Option<NodeId> {
            self.nodes.as_ref().map(|nodes| nodes.toggle)
        }

        fn select(&mut self, x: f32, y: f32) -> Result<Update, String> {
            let selected = self.hits.at(x, y).copied();
            if selected == self.selected {
                return Ok(Update::Unchanged);
            }
            self.selected = selected;
            let Some(nodes) = &self.nodes else {
                return Err(TreeError::NotMounted.into());
            };
            let (chart, selection) = (nodes.chart, nodes.selection);
            let (canvas, hits) = bar_chart(self.selected);
            self.hits = hits;
            self.panel.set_props(chart, canvas)?;
            self.panel
                .set_props(selection, selection_text(self.selected))?;
            Ok(self.panel.take_update())
        }

        fn toggle(&mut self) -> Result<Update, String> {
            self.running = !self.running;
            if self.running {
                request_tick(PANEL, TICK_MS).map_err(|error| error.message)?;
            } else {
                cancel_tick(PANEL);
            }
            let Some(nodes) = &self.nodes else {
                return Err(TreeError::NotMounted.into());
            };
            let toggle = nodes.toggle;
            self.panel.set_props(toggle, toggle_button(self.running))?;
            Ok(self.panel.take_update())
        }

        fn sample(&mut self) {
            if self.samples.len() == SAMPLES {
                self.samples.remove(0);
            }
            let value = self.walk.next();
            self.samples.push(value);
        }

        fn mount_panel(&mut self) -> Result<Vec<Node>, String> {
            let (chart_props, hits) = bar_chart(self.selected);
            self.hits = hits;
            let spark_props = sparkline(&self.samples);
            let selection = selection_text(self.selected);
            let toggle_props = toggle_button(self.running);
            let mut tree = std::mem::take(&mut self.panel);
            let mut nodes = None;
            let mounted = tree.mount(|tree| {
                tree.flex(
                    FlexProps::column()
                        .gap(8)
                        .padding(8)
                        .grow()
                        .align(Align::Stretch),
                    |tree| {
                        tree.text(TextProps::new("Commits this week").bold());
                        let chart = tree.canvas(chart_props);
                        let selection = tree.text(selection);
                        tree.text(TextProps::new("Load").bold());
                        let spark = tree.canvas(spark_props);
                        let toggle = tree.button(toggle_props);
                        nodes = Some(PanelNodes {
                            chart,
                            selection,
                            spark,
                            toggle,
                        });
                    },
                );
            });
            self.panel = tree;
            let mounted = mounted?;
            self.nodes = nodes;
            if self.running
                && let Err(error) = request_tick(PANEL, TICK_MS)
            {
                log(
                    LogLevel::Warn,
                    &format!("the sparkline cannot tick: {}", error.message),
                );
            }
            Ok(mounted)
        }
    }

    impl Extension for Charts {
        fn new() -> Self {
            Self {
                selected: None,
                hits: CanvasHits::new(),
                samples: Vec::new(),
                walk: Walk::new(),
                running: true,
                panel: SurfaceTree::default(),
                nodes: None,
            }
        }

        fn surfaces(&self) -> Vec<Surface> {
            vec![Surface::panel(
                PANEL,
                "Charts",
                DockPosition::Left,
                Icon::Table,
                320,
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
                UiEvent::CanvasClick(click) if Some(click.canvas) == self.chart_node() => {
                    self.select(click.x, click.y)
                }
                UiEvent::Click(id) if Some(id) == self.toggle_node() => self.toggle(),
                _ => Ok(Update::Unchanged),
            }
        }

        /// A sample, then the next tick while running. The host pauses ticks while the panel is
        /// hidden.
        fn tick(&mut self, surface_id: &str) -> Result<Update, String> {
            if surface_id != PANEL || !self.running {
                return Ok(Update::Unchanged);
            }
            self.sample();
            let Some(nodes) = &self.nodes else {
                return Err(TreeError::NotMounted.into());
            };
            let spark = nodes.spark;
            self.panel.set_props(spark, sparkline(&self.samples))?;
            request_tick(PANEL, TICK_MS).map_err(|error| error.message)?;
            Ok(self.panel.take_update())
        }
    }
}

/// What a 0.1 or 0.2 build shows: canvases need the 0.3 world.
#[cfg(not(feature = "fyx-ui-0-3"))]
pub struct NeedsCanvas;

#[cfg(not(feature = "fyx-ui-0-3"))]
pub const NEEDS_0_3: &str =
    "Charts need fyx:ui 0.3. Build this demo with script/fyx-build-demos --fyx-ui-0-3.";

#[cfg(not(feature = "fyx-ui-0-3"))]
impl Extension for NeedsCanvas {
    fn new() -> Self {
        Self
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            PANEL,
            "Charts",
            DockPosition::Left,
            Icon::Table,
            320,
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
// world (still without canvases) and `--features fyx-ui-0-3` the staged 0.3 world with them.
#[cfg(not(any(feature = "fyx-ui-0-2", feature = "fyx-ui-0-3")))]
register_extension!(NeedsCanvas, v0_1);
#[cfg(all(feature = "fyx-ui-0-2", not(feature = "fyx-ui-0-3")))]
register_extension!(NeedsCanvas, v0_2);
#[cfg(feature = "fyx-ui-0-3")]
register_extension!(Charts);

#[cfg(all(test, feature = "fyx-ui-0-3"))]
mod tests {
    use super::*;

    fn mounted() -> Charts {
        take_host_effects();
        let mut charts = Charts::new();
        charts.mount(PANEL).expect("mount");
        charts
    }

    fn click(charts: &mut Charts, x: f32, y: f32) -> Update {
        let canvas = charts.chart_node().expect("mounted");
        charts
            .handle_event(PANEL, UiEvent::CanvasClick(CanvasClick { canvas, x, y }))
            .expect("click")
    }

    #[test]
    fn the_chart_passes_the_host_rules() {
        let (canvas, hits) = bar_chart(Some(2));
        assert_eq!(canvas.ops.len(), 15, "7 bars, 7 labels and the axis");
        assert_eq!(hits.len(), 7);
        let tallest = bar(2);
        assert!((tallest.y - 12.0).abs() < 0.01 && tallest.height > 100.0);
        let mut surface = SurfaceTree::default();
        surface
            .mount(|tree| {
                tree.canvas(canvas);
                tree.canvas(sparkline(&[10, 90, 40]));
            })
            .expect("the host's canvas checks pass");
    }

    #[test]
    fn a_click_on_a_bar_selects_it() {
        let mut charts = mounted();
        let wed = bar(2);
        let update = click(&mut charts, wed.x + 5.0, wed.y + 5.0);
        assert_eq!(charts.selected(), Some(2));
        let Update::Patches(patches) = update else {
            panic!("{update:?}");
        };
        assert!(patches.iter().any(|patch| matches!(
            patch,
            Patch::SetProps((_, Props::Text(text))) if text.content == "Wed: 42 commits"
        )));
        assert_eq!(
            click(&mut charts, wed.x + 5.0, wed.y + 5.0),
            Update::Unchanged
        );
        click(&mut charts, 10.0, 130.0);
        assert_eq!(charts.selected(), None, "below the bars selects nothing");
    }

    #[test]
    fn ticks_add_samples_until_paused() {
        let mut charts = mounted();
        assert!(matches!(
            &take_host_effects()[..],
            [HostEffect::RequestTick {
                delay_ms: TICK_MS,
                ..
            }]
        ));
        for _ in 0..(SAMPLES + 5) {
            charts.tick(PANEL).expect("tick");
        }
        assert_eq!(charts.samples().len(), SAMPLES);
        take_host_effects();

        let toggle = charts.toggle_node().expect("mounted");
        charts
            .handle_event(PANEL, UiEvent::Click(toggle))
            .expect("pause");
        assert!(!charts.is_running());
        assert!(matches!(
            &take_host_effects()[..],
            [HostEffect::CancelTick { .. }]
        ));
        assert_eq!(charts.tick(PANEL).expect("tick"), Update::Unchanged);
    }
}
