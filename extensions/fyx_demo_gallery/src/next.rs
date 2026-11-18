//! The gallery's `fyx:ui` 0.2 part, built only with `--features fyx-ui-0-2`. It adds three
//! surfaces and leaves the 0.1 ones exactly as they are. (TODO(0.4): commands, hover, code lenses
//! and picker surfaces left the smaller 0.3 (A3); show them here when 0.4 brings them.)
//!
//! - Panel `"gallery-next"` (right dock, title "Gallery 0.2", icon `bolt`, default size 420):
//!   - a `Pick a fruit` button (icon `search`, tooltip) that shows the picker, and the text
//!     `Picked: …`;
//!   - three subtle buttons with icons and tooltips (`Copy`, `Pin`, `Delete`, the last `danger`),
//!     and the text `Last action: …`;
//!   - a `secret` input, and the text `Token: N characters`;
//!   - a `Refresh rows` button that answers `invalidate-rows` on the tree list, whose rows then
//!     read `(scan N)`;
//!   - a tree list of 10 rows whose `indent` is the depth;
//!   - a list of all 101 frozen icons, each row an icon and its WIT name;
//!   - notifications and progress (design.md §M.8):
//!     - `Notify` shows a sticky info notification with `Open` and `Later` buttons, and a warning
//!       that closes by itself;
//!     - `Run job` starts a cancellable progress in the status bar and one in this panel, and a
//!       tick every half second moves both by 10 %;
//!     - the text `Notifications: …` reports the ids, a button pressed or a close, and
//!       `Job: …` the progress, `done` at 100 % or `cancelled` after Cancel.
//! - Modal `"gallery-picker"` (medium, title "Pick a fruit"): a filter input, a list of the
//!   matching fruits whose `selected` row follows `row-selected` (arrow keys), and a hint. Enter
//!   or a row click picks the selected fruit: the modal hides itself and pushes `Picked: <fruit>`
//!   to the panel. Escape or a click outside dismisses it: the panel reads
//!   `Picked: nothing (escape)`.
//! - Status item `"gallery-clock"` (right side, priority 1): a clock icon and the UTC time,
//!   `HH:MM:SS UTC`, refreshed by a tick every second, requested at mount and again in each tick.

use ui_ext_sdk::*;

pub(crate) const PANEL: &str = "gallery-next";
pub(crate) const PICKER: &str = "gallery-picker";
pub(crate) const CLOCK: &str = "gallery-clock";
pub(crate) const CLOCK_INTERVAL_MS: u32 = 1000;
pub(crate) const JOB_TICK_MS: u32 = 500;

const ROW_HEIGHT: u16 = 24;

pub(crate) const FRUITS: [&str; 12] = [
    "Apple",
    "Apricot",
    "Banana",
    "Blueberry",
    "Cherry",
    "Fig",
    "Grape",
    "Lemon",
    "Mango",
    "Orange",
    "Peach",
    "Plum",
];

/// Depth, name and whether it is a folder.
pub(crate) const FILE_TREE: [(u8, &str, bool); 10] = [
    (0, "crates", true),
    (1, "ui_ext", true),
    (2, "src", true),
    (3, "tree.rs", false),
    (3, "v0_2.rs", false),
    (2, "Cargo.toml", false),
    (1, "ui_ext_sdk", true),
    (2, "src", true),
    (3, "ui_ext_sdk.rs", false),
    (0, "README.md", false),
];

/// Label, icon, style and tooltip of the tooltip buttons.
const ACTIONS: [(&str, Icon, ButtonStyle, &str); 3] = [
    (
        "Copy",
        Icon::Copy,
        ButtonStyle::Subtle,
        "Copy the selection",
    ),
    (
        "Pin",
        Icon::Pin,
        ButtonStyle::Subtle,
        "Keep this panel open",
    ),
    (
        "Delete",
        Icon::Trash,
        ButtonStyle::Danger,
        "Delete for good",
    ),
];

struct PanelNodes {
    pick: NodeId,
    picked: NodeId,
    actions: Vec<(NodeId, &'static str)>,
    last_action: NodeId,
    token: NodeId,
    token_length: NodeId,
    refresh: NodeId,
    tree_list: NodeId,
    icon_list: NodeId,
    notify: NodeId,
    run_job: NodeId,
    notifications: NodeId,
    job: NodeId,
}

/// A running job: its status-bar progress, its panel progress, and how far it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Job {
    pub(crate) status_bar: u64,
    pub(crate) panel: Option<u64>,
    pub(crate) percent: u8,
}

struct PickerNodes {
    filter: NodeId,
    list: NodeId,
}

#[derive(Default)]
pub(crate) struct Next {
    picked: Option<String>,
    last_action: Option<&'static str>,
    token_length: usize,
    scans: u32,
    filter: String,
    selected: Option<u32>,
    panel: SurfaceTree,
    panel_nodes: Option<PanelNodes>,
    picker: SurfaceTree,
    picker_nodes: Option<PickerNodes>,
    clock: SurfaceTree,
    clock_text: Option<NodeId>,
    notifications: String,
    job: Option<Job>,
    job_text: String,
}

pub(crate) fn surfaces() -> [Surface; 3] {
    [
        Surface::panel(PANEL, "Gallery 0.2", DockPosition::Right, Icon::Bolt, 420),
        Surface::modal(PICKER, "Pick a fruit", ModalWidth::Medium),
        Surface::status_item(CLOCK, "Clock", StatusBarSide::Right, 1),
    ]
}

pub(crate) fn owns(surface_id: &str) -> bool {
    matches!(surface_id, PANEL | PICKER | CLOCK)
}

fn caption(tree: &mut TreeBuilder, content: &str) {
    tree.text(TextProps::new(content).color(ColorToken::TextMuted));
}

/// `HH:MM:SS UTC` from the WASI wall clock.
pub(crate) fn clock_text() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
        % 86_400;
    format!(
        "{:02}:{:02}:{:02} UTC",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

impl Next {
    fn matches(&self) -> Vec<&'static str> {
        let filter = self.filter.to_lowercase();
        FRUITS
            .into_iter()
            .filter(|fruit| fruit.to_lowercase().contains(&filter))
            .collect()
    }

    fn picked_label(&self) -> TextProps {
        match &self.picked {
            Some(picked) => TextProps::new(format!("Picked: {picked}")),
            None => TextProps::new("Picked: nothing yet").color(ColorToken::TextMuted),
        }
    }

    fn last_action_label(&self) -> TextProps {
        TextProps::new(format!(
            "Last action: {}",
            self.last_action.unwrap_or("none")
        ))
    }

    fn notifications_label(&self) -> TextProps {
        let text = if self.notifications.is_empty() {
            "none yet"
        } else {
            &self.notifications
        };
        TextProps::new(format!("Notifications: {text}"))
    }

    fn job_label(&self) -> TextProps {
        let text = match (&self.job, self.job_text.as_str()) {
            (Some(job), _) => format!("{} %", job.percent),
            (None, "") => "not started".to_string(),
            (None, text) => text.to_string(),
        };
        TextProps::new(format!("Job: {text}"))
    }

    /// `Notify`: a sticky info notification with two buttons, and a warning that closes itself.
    fn notify(&mut self) {
        let info = Notification::new(PANEL, NotificationLevel::Info, "Gallery says hello")
            .body("A demo notification from the gallery's 0.2 build.")
            .action("open", "Open")
            .action("later", "Later")
            .sticky();
        let warning = Notification::new(
            PANEL,
            NotificationLevel::Warning,
            "This one closes by itself",
        );
        let ids = [notify(&info), notify(&warning)]
            .into_iter()
            .map(|result| match result {
                Ok(id) => format!("#{id}"),
                Err(error) => error.code.name().to_string(),
            })
            .collect::<Vec<_>>();
        self.notifications = format!("shown {}", ids.join(" and "));
    }

    /// `Run job`: a cancellable progress in the status bar and one in this panel, moved by ticks.
    fn run_job(&mut self) {
        if self.job.is_some() {
            return;
        }
        let status_bar =
            ProgressOptions::new(PANEL, "Gallery job", ProgressLocation::StatusBar).cancellable();
        let status_bar = match progress_start(&status_bar) {
            Ok(id) => id,
            Err(error) => {
                self.job_text = format!("could not start ({})", error.code.name());
                return;
            }
        };
        let panel = progress_start(&ProgressOptions::new(
            PANEL,
            "Gallery job",
            ProgressLocation::Panel,
        ))
        .ok();
        self.job = Some(Job {
            status_bar,
            panel,
            percent: 0,
        });
        if let Err(error) = request_tick(PANEL, JOB_TICK_MS) {
            log(
                LogLevel::Warn,
                &format!("the job cannot tick: {}", error.message),
            );
        }
    }

    fn end_job(&mut self, text: &str) {
        if let Some(job) = self.job.take() {
            progress_end(job.status_bar);
            if let Some(panel) = job.panel {
                progress_end(panel);
            }
        }
        cancel_tick(PANEL);
        self.job_text = text.to_string();
    }

    /// One tick of the job: 10 % more on both progresses, `done` at 100 %.
    fn job_tick(&mut self) -> Result<Update, String> {
        let Some(job) = self.job.as_mut() else {
            return Ok(Update::Unchanged);
        };
        job.percent = (job.percent + 10).min(100);
        let job = *job;
        if job.percent >= 100 {
            self.end_job("done");
        } else {
            let message = format!("Step {} of 10", job.percent / 10);
            for id in std::iter::once(job.status_bar).chain(job.panel) {
                progress_update(id, ProgressValue::Percent(job.percent), Some(&message))
                    .map_err(|error| error.message)?;
            }
            request_tick(PANEL, JOB_TICK_MS).map_err(|error| error.message)?;
        }
        self.refresh_notices()?;
        Ok(self.panel.take_update())
    }

    fn refresh_notices(&mut self) -> Result<(), String> {
        let Some(nodes) = &self.panel_nodes else {
            return Err(TreeError::NotMounted.into());
        };
        let (notifications, job) = (nodes.notifications, nodes.job);
        self.panel
            .set_props(notifications, self.notifications_label())?;
        self.panel.set_props(job, self.job_label())?;
        Ok(())
    }

    fn token_label(&self) -> TextProps {
        TextProps::new(format!("Token: {} characters", self.token_length))
    }

    fn picker_list(&self) -> ListProps {
        let count = u32::try_from(self.matches().len()).unwrap_or(u32::MAX);
        ListProps::new(count, ROW_HEIGHT).selected(self.selected)
    }

    pub(crate) fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        match surface_id {
            PANEL => self.mount_panel(),
            PICKER => self.mount_picker(),
            CLOCK => self.mount_clock(),
            _ => Err(format!("unknown surface {surface_id:?}")),
        }
    }

    fn mount_panel(&mut self) -> Result<Vec<Node>, String> {
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
                    tree.text(TextProps::new("fyx:ui 0.2").bold());
                    let mut pick = 0;
                    let mut picked = 0;
                    tree.flex(FlexProps::row().gap(8).align(Align::Center), |tree| {
                        pick = tree.button(
                            ButtonProps::new("Pick a fruit")
                                .icon(Icon::Search)
                                .style(ButtonStyle::Filled)
                                .tooltip("Opens a modal: arrow keys select, Enter picks"),
                        );
                        picked = tree.text(self.picked_label());
                    });
                    caption(tree, "Buttons with icons and tooltips");
                    let mut actions = Vec::new();
                    tree.flex(FlexProps::row().gap(4), |tree| {
                        for (label, icon, style, tooltip) in ACTIONS {
                            let id = tree.button(
                                ButtonProps::new(label)
                                    .icon(icon)
                                    .style(style)
                                    .tooltip(tooltip),
                            );
                            actions.push((id, label));
                        }
                    });
                    let last_action = tree.text(self.last_action_label());
                    caption(tree, "Secret input: the text is masked on screen");
                    let token =
                        tree.input(InputProps::new("").placeholder("Paste a token").secret());
                    let token_length = tree.text(self.token_label());
                    caption(tree, "Rows with indent; Refresh invalidates them in place");
                    let refresh = tree.button(
                        ButtonProps::new("Refresh rows")
                            .icon(Icon::Refresh)
                            .tooltip("invalidate-rows: the list keeps its scroll position"),
                    );
                    let tree_list = tree.list(ListProps::new(FILE_TREE.len() as u32, ROW_HEIGHT));
                    caption(tree, "All 101 frozen icons");
                    let icon_list = tree.list(ListProps::new(Icon::all().len() as u32, ROW_HEIGHT));
                    caption(tree, "Notifications and progress");
                    let mut notify = 0;
                    let mut run_job = 0;
                    tree.flex(FlexProps::row().gap(4), |tree| {
                        notify = tree.button(ButtonProps::new("Notify").icon(Icon::Bell).tooltip(
                            "An info notification with buttons, and a warning that closes itself",
                        ));
                        run_job =
                            tree.button(ButtonProps::new("Run job").icon(Icon::Play).tooltip(
                                "A cancellable progress in the status bar and in this panel",
                            ));
                    });
                    let notifications = tree.text(self.notifications_label());
                    let job = tree.text(self.job_label());
                    nodes = Some(PanelNodes {
                        pick,
                        picked,
                        actions,
                        last_action,
                        token,
                        token_length,
                        refresh,
                        tree_list,
                        icon_list,
                        notify,
                        run_job,
                        notifications,
                        job,
                    });
                },
            );
        });
        self.panel = tree;
        let mounted = mounted?;
        self.panel_nodes = nodes;
        Ok(mounted)
    }

    /// A fresh picker each time it shows: an empty filter and the first fruit selected.
    fn mount_picker(&mut self) -> Result<Vec<Node>, String> {
        self.filter.clear();
        self.selected = Some(0);
        let mut tree = std::mem::take(&mut self.picker);
        let mut nodes = None;
        let list = self.picker_list();
        let mounted = tree.mount(|tree| {
            tree.flex(
                FlexProps::column().gap(8).padding(8).align(Align::Stretch),
                |tree| {
                    let filter = tree.input(InputProps::new("").placeholder("Filter fruits"));
                    let list = tree.list(list);
                    caption(tree, "Up and Down select, Enter picks, Escape closes");
                    nodes = Some(PickerNodes { filter, list });
                },
            );
        });
        self.picker = tree;
        let mounted = mounted?;
        self.picker_nodes = nodes;
        Ok(mounted)
    }

    fn mount_clock(&mut self) -> Result<Vec<Node>, String> {
        let mut tree = std::mem::take(&mut self.clock);
        let mut text = None;
        let mounted = tree.mount(|tree| {
            tree.flex(FlexProps::row().gap(4).align(Align::Center), |tree| {
                tree.icon(IconProps::new(Icon::Clock).color(ColorToken::TextMuted));
                text = Some(tree.text(clock_text()));
            });
        });
        self.clock = tree;
        let mounted = mounted?;
        self.clock_text = text;
        if let Err(error) = request_tick(CLOCK, CLOCK_INTERVAL_MS) {
            log(
                LogLevel::Warn,
                &format!("the clock cannot tick: {}", error.message),
            );
        }
        Ok(mounted)
    }

    pub(crate) fn handle_event(
        &mut self,
        surface_id: &str,
        event: UiEvent,
    ) -> Result<Update, String> {
        match surface_id {
            PANEL => self.panel_event(event),
            PICKER => self.picker_event(event),
            CLOCK => Ok(Update::Unchanged),
            _ => Err(format!("unknown surface {surface_id:?}")),
        }
    }

    fn panel_event(&mut self, event: UiEvent) -> Result<Update, String> {
        self.panel.apply_event(&event);
        let Some(nodes) = &self.panel_nodes else {
            return Err(TreeError::NotMounted.into());
        };
        match event {
            UiEvent::Click(id) if id == nodes.pick => {
                if let Err(error) = show_surface(PICKER) {
                    self.last_action = Some("the picker could not open");
                    log(LogLevel::Warn, &format!("show-surface: {}", error.message));
                    let last_action = nodes.last_action;
                    self.panel
                        .set_props(last_action, self.last_action_label())?;
                }
            }
            UiEvent::Click(id) if id == nodes.refresh => {
                self.scans += 1;
                let list = nodes.tree_list;
                self.panel.invalidate_rows(list, None)?;
            }
            UiEvent::Click(id) if id == nodes.notify => {
                self.notify();
                self.refresh_notices()?;
            }
            UiEvent::Click(id) if id == nodes.run_job => {
                self.run_job();
                self.refresh_notices()?;
            }
            UiEvent::NotificationAction(action) => {
                self.notifications = format!("#{} pressed {}", action.notification, action.action);
                self.refresh_notices()?;
            }
            UiEvent::NotificationClosed((id, reason)) => {
                let reason = match reason {
                    NotificationCloseReason::User => "by you",
                    NotificationCloseReason::Expired => "by itself",
                    NotificationCloseReason::WindowClosed => "with the window",
                };
                self.notifications = format!("#{id} closed {reason}");
                self.refresh_notices()?;
            }
            UiEvent::ProgressCancelRequested(_) => {
                self.end_job("cancelled");
                self.refresh_notices()?;
            }
            UiEvent::Click(id) => {
                if let Some(&(_, label)) = nodes.actions.iter().find(|(button, _)| *button == id) {
                    self.last_action = Some(label);
                    let last_action = nodes.last_action;
                    self.panel
                        .set_props(last_action, self.last_action_label())?;
                }
            }
            UiEvent::InputChanged((id, value)) | UiEvent::InputSubmitted((id, value))
                if id == nodes.token =>
            {
                self.token_length = value.chars().count();
                let token_length = nodes.token_length;
                self.panel.set_props(token_length, self.token_label())?;
            }
            _ => {}
        }
        Ok(self.panel.take_update())
    }

    fn picker_event(&mut self, event: UiEvent) -> Result<Update, String> {
        self.picker.apply_event(&event);
        if let UiEvent::Dismissed(reason) = event {
            let reason = match reason {
                DismissReason::Escape => "escape",
                DismissReason::FocusLost => "focus lost",
                DismissReason::Replaced => "replaced",
                DismissReason::WindowClosed => "window closed",
            };
            self.picker_nodes = None;
            self.picked = None;
            self.push_picked(TextProps::new(format!("Picked: nothing ({reason})")))?;
            return Ok(Update::Unchanged);
        }
        let Some(nodes) = &self.picker_nodes else {
            return Err(TreeError::NotMounted.into());
        };
        let (filter, list) = (nodes.filter, nodes.list);
        match event {
            UiEvent::InputChanged((id, value)) if id == filter => {
                self.filter = value;
                self.selected = (!self.matches().is_empty()).then_some(0);
                self.picker.set_props(list, self.picker_list())?;
                self.picker.invalidate_rows(list, None)?;
            }
            UiEvent::RowSelected((id, row)) if id == list => self.selected = Some(row),
            UiEvent::RowClick((id, row)) if id == list => {
                self.selected = Some(row);
                return self.pick();
            }
            UiEvent::InputSubmitted((id, _)) if id == filter => return self.pick(),
            _ => {}
        }
        Ok(self.picker.take_update())
    }

    /// Picks the selected fruit: hides the picker and tells the panel.
    fn pick(&mut self) -> Result<Update, String> {
        let matches = self.matches();
        let Some(fruit) = self
            .selected
            .and_then(|row| matches.get(row as usize).copied())
        else {
            return Ok(Update::Unchanged);
        };
        hide_surface(PICKER).map_err(|error| error.message)?;
        self.picker_nodes = None;
        self.picked = Some(fruit.to_string());
        self.push_picked(self.picked_label())?;
        Ok(Update::Unchanged)
    }

    fn push_picked(&mut self, label: TextProps) -> Result<(), String> {
        let Some(picked) = self.panel_nodes.as_ref().map(|nodes| nodes.picked) else {
            return Ok(());
        };
        self.panel.set_props(picked, label)?;
        if let Err(error) = self.panel.push(PANEL) {
            log(
                LogLevel::Warn,
                &format!("panel update refused: {}", error.message),
            );
        }
        Ok(())
    }

    pub(crate) fn list_rows(
        &mut self,
        surface_id: &str,
        list_node: NodeId,
        start: u32,
        count: u32,
    ) -> Vec<ListRow> {
        match surface_id {
            PANEL => {
                let Some(nodes) = &self.panel_nodes else {
                    return Vec::new();
                };
                if list_node == nodes.tree_list {
                    let scans = self.scans;
                    list_rows(start, count, FILE_TREE.len() as u32, |index, tree| {
                        let (_, name, folder) = FILE_TREE[index as usize];
                        tree.flex(FlexProps::row().gap(6).align(Align::Center), |tree| {
                            tree.icon(if folder { Icon::Folder } else { Icon::File });
                            tree.text(name);
                            if scans > 0 {
                                caption(tree, &format!("(scan {scans})"));
                            }
                        });
                    })
                    .into_iter()
                    .map(|row| {
                        let depth = FILE_TREE[row.index as usize].0;
                        row.with_indent(depth)
                    })
                    .collect()
                } else if list_node == nodes.icon_list {
                    list_rows(start, count, Icon::all().len() as u32, |index, tree| {
                        let icon = Icon::all().nth(index as usize).unwrap_or(Icon::Blank);
                        tree.flex(FlexProps::row().gap(8).align(Align::Center), |tree| {
                            tree.icon(icon);
                            tree.text(TextProps::new(icon.name()).monospace());
                        });
                    })
                } else {
                    Vec::new()
                }
            }
            PICKER if self.picker_nodes.as_ref().map(|nodes| nodes.list) == Some(list_node) => {
                let matches = self.matches();
                list_rows(start, count, matches.len() as u32, |index, tree| {
                    tree.text(matches[index as usize]);
                })
            }
            _ => Vec::new(),
        }
    }

    pub(crate) fn tick(&mut self, surface_id: &str) -> Result<Update, String> {
        if surface_id == PANEL {
            return self.job_tick();
        }
        if surface_id != CLOCK {
            return Err(format!("no ticks for {surface_id:?}"));
        }
        let Some(text) = self.clock_text else {
            return Err(TreeError::NotMounted.into());
        };
        self.clock.set_props(text, TextProps::new(clock_text()))?;
        request_tick(CLOCK, CLOCK_INTERVAL_MS).map_err(|error| error.message)?;
        Ok(self.clock.take_update())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mounted() -> Next {
        take_host_effects();
        let mut next = Next::default();
        next.mount(PANEL).expect("mount");
        next
    }

    fn click(next: &mut Next, label: &str) {
        let id = next
            .panel
            .to_nodes()
            .into_iter()
            .find(|node| matches!(&node.props, Props::Button(button) if button.label == label))
            .map(|node| node.id)
            .unwrap_or_else(|| panic!("no button {label}"));
        next.handle_event(PANEL, UiEvent::Click(id)).expect("click");
    }

    #[test]
    fn notify_shows_two_notifications() {
        let mut next = mounted();
        click(&mut next, "Notify");
        let effects = take_host_effects();
        assert!(
            matches!(
                &effects[..],
                [
                    HostEffect::Notify { notification: info, .. },
                    HostEffect::Notify { notification: warning, .. },
                ] if info.actions.len() == 2 && !info.auto_dismiss
                    && warning.level == NotificationLevel::Warning && warning.auto_dismiss
            ),
            "{effects:?}"
        );
    }

    #[test]
    fn the_job_runs_on_ticks_and_ends() {
        let mut next = mounted();
        click(&mut next, "Run job");
        let effects = take_host_effects();
        assert!(
            matches!(
                &effects[..],
                [
                    HostEffect::ProgressStart { options: bar, .. },
                    HostEffect::ProgressStart { options: panel, .. },
                    HostEffect::RequestTick { .. },
                ] if bar.cancellable && bar.location == ProgressLocation::StatusBar
                    && panel.location == ProgressLocation::Panel
            ),
            "{effects:?}"
        );
        for _ in 0..10 {
            next.tick(PANEL).expect("tick");
        }
        assert!(next.job.is_none(), "done at 100 %");
        let ends = take_host_effects()
            .into_iter()
            .filter(|effect| matches!(effect, HostEffect::ProgressEnd(_)))
            .count();
        assert_eq!(ends, 2);
    }
}
