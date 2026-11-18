//! Fyx demo 10: the T2 capabilities of the staged `fyx:ui` 0.3, for checking their consent flows
//! by hand (`docs/fyx/wit-0.3/design.md` §7 to §11). It builds only for 0.3: the default build
//! shows a panel that says so.
//!
//! One panel `"t2"` (right dock, title "T2 capabilities", icon `lock`), with four sections. Each
//! shows its permission state and has a button whose first use shows Fyx's consent prompt:
//!
//! - **Fetch** (`network:fetch`, domain `api.github.com`): `GET https://api.github.com/zen`, and
//!   the text it returns, or the error;
//! - **Process** (`process:spawn`, program `git` with `status --short`): runs it in the first
//!   worktree, and shows its output and exit code;
//! - **Edit** (`workspace:edit`): inserts a header comment at the top of the file named in the
//!   input (`README.md` by default) as one undoable edit; nothing is saved;
//! - **Agent reviews** (`agent:observe`): `Start` shows the open agent reviews live, as list
//!   rows of file and state; `Stop` ends the feed.
//!
//! The 0.3 `permissions` interface (`query`, `request`) is not built in the host yet, and calling
//! it traps. So the demo shows the state it has learned: `not asked yet` at first, then
//! `waiting for an answer…` while a request waits for the prompt, `allowed` or `denied` from the
//! outcome, and any `permission-changed` the host sends.

use ui_ext_sdk::*;

const PANEL: &str = "t2";

#[cfg(feature = "fyx-ui-0-3")]
pub use t2::*;

#[cfg(feature = "fyx-ui-0-3")]
mod t2 {
    use ui_ext_sdk::*;

    use super::PANEL;

    pub const ZEN_URL: &str = "https://api.github.com/zen";
    pub const HEADER: &str = "Fyx T2 demo: this line was inserted by workspace:edit\n";
    const ROW_HEIGHT: u16 = 24;

    /// What the demo knows about one capability's state.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Known {
        NotAsked,
        Waiting,
        Allowed,
        Denied,
        Unavailable,
        NotDeclared,
    }

    impl Known {
        fn label(self) -> &'static str {
            match self {
                Known::NotAsked => "Permission: not asked yet",
                Known::Waiting => "Permission: waiting for an answer…",
                Known::Allowed => "Permission: allowed",
                Known::Denied => "Permission: denied (change it on the extensions page)",
                Known::Unavailable => "Permission: not available here",
                Known::NotDeclared => "Permission: not declared in extension.toml",
            }
        }

        fn color(self) -> ColorToken {
            match self {
                Known::Allowed => ColorToken::Success,
                Known::Denied | Known::NotDeclared => ColorToken::Error,
                Known::Unavailable => ColorToken::Warning,
                Known::NotAsked | Known::Waiting => ColorToken::TextMuted,
            }
        }

        /// From an import's refusal: `denied` and `unavailable` are the broker's, the rest is
        /// the call's own problem.
        fn from_error(error: &HostError, before: Known) -> Known {
            match error.code {
                HostErrorCode::Denied => Known::Denied,
                HostErrorCode::Unavailable => Known::Unavailable,
                HostErrorCode::NotPermitted => Known::NotDeclared,
                _ => before,
            }
        }

        fn from_state(state: PermissionState) -> Known {
            match state {
                PermissionState::Granted => Known::Allowed,
                PermissionState::Denied => Known::Denied,
                PermissionState::Ask => Known::NotAsked,
                PermissionState::Unavailable => Known::Unavailable,
            }
        }
    }

    /// One review as the list shows it.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct ReviewRow {
        pub review: u64,
        pub file: String,
        pub state: String,
    }

    fn review_row(info: &ReviewInfo) -> ReviewRow {
        let state = match info.resolution {
            Resolution::Unchanged => "unchanged".to_string(),
            Resolution::Pending(count) => format!("{count} pending"),
            Resolution::AcceptedAll => "accepted".to_string(),
            Resolution::RejectedAll => "rejected".to_string(),
            Resolution::Partial((accepted, rejected)) => {
                format!("{accepted} accepted, {rejected} rejected")
            }
        };
        ReviewRow {
            review: info.review,
            file: info.file.as_ref().map_or_else(
                || "(outside the worktrees)".to_string(),
                |file| file.path.clone(),
            ),
            state,
        }
    }

    #[derive(Default)]
    struct Nodes {
        fetch_state: NodeId,
        fetch_result: NodeId,
        fetch: NodeId,
        process_state: NodeId,
        process_result: NodeId,
        run: NodeId,
        edit_state: NodeId,
        edit_path: NodeId,
        edit_result: NodeId,
        insert: NodeId,
        agent_state: NodeId,
        agent_toggle: NodeId,
        agent_summary: NodeId,
        reviews: NodeId,
    }

    pub struct T2 {
        pub fetch_state: Known,
        pub process_state: Known,
        pub edit_state: Known,
        pub agent_state: Known,
        fetch_result: String,
        process_output: String,
        process_result: String,
        edit_path: String,
        edit_result: String,
        observing: bool,
        reviews: Vec<ReviewRow>,
        panel: SurfaceTree,
        nodes: Option<Nodes>,
    }

    fn section_title(tree: &mut TreeBuilder, title: &str) {
        tree.text(TextProps::new(title).bold());
    }

    fn state_text(known: Known) -> TextProps {
        TextProps::new(known.label()).color(known.color())
    }

    fn agent_button(observing: bool) -> ButtonProps {
        if observing {
            ButtonProps::new("Stop")
                .icon(Icon::Stop)
                .tooltip("End the agent review feed")
        } else {
            ButtonProps::new("Start")
                .icon(Icon::Play)
                .tooltip("Show agent reviews; asks for permission the first time")
        }
    }

    impl T2 {
        pub fn reviews(&self) -> &[ReviewRow] {
            &self.reviews
        }

        /// The panel's button with this label.
        pub fn button(&self, label: &str) -> Option<NodeId> {
            let nodes = self.nodes.as_ref()?;
            [nodes.fetch, nodes.run, nodes.insert, nodes.agent_toggle]
                .into_iter()
                .find(|id| {
                    matches!(self.panel.props(*id), Some(Props::Button(button)) if button.label == label)
                })
        }

        fn nodes(&self) -> Result<&Nodes, String> {
            self.nodes
                .as_ref()
                .ok_or_else(|| TreeError::NotMounted.into())
        }

        fn process_markdown(&self) -> String {
            if self.process_output.is_empty() {
                return self.process_result.clone();
            }
            format!(
                "```\n{}\n```\n{}",
                self.process_output.trim_end(),
                self.process_result
            )
        }

        /// Queues every section's props; unchanged ones queue nothing.
        fn refresh(&mut self, rows: bool) -> Result<(), String> {
            let nodes = self.nodes()?;
            let ids = [
                (nodes.fetch_state, Props::from(state_text(self.fetch_state))),
                (
                    nodes.fetch_result,
                    TextProps::new(self.fetch_result.clone()).into(),
                ),
                (nodes.process_state, state_text(self.process_state).into()),
                (
                    nodes.process_result,
                    Props::Markdown(self.process_markdown()),
                ),
                (nodes.edit_state, state_text(self.edit_state).into()),
                (
                    nodes.edit_result,
                    TextProps::new(self.edit_result.clone()).into(),
                ),
                (nodes.agent_state, state_text(self.agent_state).into()),
                (nodes.agent_toggle, agent_button(self.observing).into()),
                (
                    nodes.agent_summary,
                    TextProps::new(format!("{} open reviews", self.reviews.len()))
                        .color(ColorToken::TextMuted)
                        .into(),
                ),
                (
                    nodes.reviews,
                    ListProps::new(self.reviews.len() as u32, ROW_HEIGHT).into(),
                ),
            ];
            let reviews = nodes.reviews;
            for (id, props) in ids {
                self.panel.set_props(id, props)?;
            }
            if rows {
                self.panel.invalidate_rows(reviews, None)?;
            }
            Ok(())
        }

        fn fetch_zen(&mut self) {
            let request = HttpRequest::get(ZEN_URL)
                .header("user-agent", "fyx-demo-t2")
                .header("accept", "text/plain");
            match fetch(&request) {
                Ok(_) => {
                    if self.fetch_state != Known::Allowed {
                        self.fetch_state = Known::Waiting;
                    }
                    self.fetch_result = "Fetching…".into();
                }
                Err(error) => {
                    self.fetch_state = Known::from_error(&error, self.fetch_state);
                    self.fetch_result = format!("{}: {}", error.code.name(), error.message);
                }
            }
        }

        fn run_git(&mut self) {
            self.process_output.clear();
            match spawn(&SpawnRequest::new("git").args(["status", "--short"])) {
                Ok(_) => {
                    if self.process_state != Known::Allowed {
                        self.process_state = Known::Waiting;
                    }
                    self.process_result = "Running `git status --short`…".into();
                }
                Err(error) => {
                    self.process_state = Known::from_error(&error, self.process_state);
                    self.process_result = format!("{}: {}", error.code.name(), error.message);
                }
            }
        }

        fn insert_header(&mut self) {
            let edit = WorkspaceEdit::new("Insert the T2 demo header")
                .file(FileEdit::path(self.edit_path.clone()).insert(Position::new(0, 0), HEADER));
            match apply_edit(&edit) {
                Ok(_) => {
                    if self.edit_state != Known::Allowed {
                        self.edit_state = Known::Waiting;
                    }
                    self.edit_result = format!("Editing {}…", self.edit_path);
                }
                Err(error) => {
                    self.edit_state = Known::from_error(&error, self.edit_state);
                    self.edit_result = format!("{}: {}", error.code.name(), error.message);
                }
            }
        }

        fn toggle_agent(&mut self) {
            if self.observing {
                stop_observing();
                self.observing = false;
                return;
            }
            match start_observing() {
                Ok(()) => {
                    self.observing = true;
                    if self.agent_state != Known::Allowed {
                        self.agent_state = Known::Waiting;
                    }
                }
                Err(error) => self.agent_state = Known::from_error(&error, self.agent_state),
            }
        }

        fn agent(&mut self, event: AgentEvent) {
            self.agent_state = Known::Allowed;
            match event {
                AgentEvent::Snapshot(reviews) => {
                    self.reviews = reviews.iter().map(review_row).collect();
                }
                AgentEvent::Opened(info)
                | AgentEvent::Updated(info)
                | AgentEvent::Resolved(info) => {
                    let row = review_row(&info);
                    match self.reviews.iter_mut().find(|r| r.review == row.review) {
                        Some(existing) => *existing = row,
                        None => self.reviews.push(row),
                    }
                }
                AgentEvent::Closed(review) => self.reviews.retain(|row| row.review != review),
                AgentEvent::TurnEnded(_)
                | AgentEvent::UnreviewedChanges(_)
                | AgentEvent::ReviewMode(_) => {}
            }
        }

        fn mount_panel(&mut self) -> Result<Vec<Node>, String> {
            let mut tree = std::mem::take(&mut self.panel);
            let mut nodes = Nodes::default();
            let path = self.edit_path.clone();
            let (fetch_state, process_state, edit_state, agent_state) = (
                self.fetch_state,
                self.process_state,
                self.edit_state,
                self.agent_state,
            );
            let (fetch_result, process_markdown, edit_result) = (
                self.fetch_result.clone(),
                self.process_markdown(),
                self.edit_result.clone(),
            );
            let observing = self.observing;
            let review_count = self.reviews.len() as u32;
            let mounted = tree.mount(|tree| {
                tree.flex(
                    FlexProps::column().gap(6).padding(8).grow().align(Align::Stretch),
                    |tree| {
                        section_title(tree, "Fetch: api.github.com/zen");
                        nodes.fetch_state = tree.text(state_text(fetch_state));
                        nodes.fetch = tree.button(
                            ButtonProps::new("Fetch zen")
                                .icon(Icon::Globe)
                                .tooltip("GET api.github.com/zen; asks for permission the first time"),
                        );
                        nodes.fetch_result = tree.text(TextProps::new(fetch_result));

                        section_title(tree, "Process: git status --short");
                        nodes.process_state = tree.text(state_text(process_state));
                        nodes.run = tree.button(
                            ButtonProps::new("Run git status")
                                .icon(Icon::Terminal)
                                .tooltip("Runs git in the worktree; asks for permission the first time"),
                        );
                        nodes.process_result = tree.markdown(process_markdown);

                        section_title(tree, "Edit: insert a header comment");
                        nodes.edit_state = tree.text(state_text(edit_state));
                        nodes.edit_path = tree.input(
                            InputProps::new(path)
                                .placeholder("A file in the worktree")
                                .label("File to edit"),
                        );
                        nodes.insert = tree.button(
                            ButtonProps::new("Insert header")
                                .icon(Icon::Pencil)
                                .tooltip("One undoable edit; nothing is saved; asks for permission the first time"),
                        );
                        nodes.edit_result = tree.text(TextProps::new(edit_result));

                        section_title(tree, "Agent reviews");
                        nodes.agent_state = tree.text(state_text(agent_state));
                        nodes.agent_toggle = tree.button(agent_button(observing));
                        nodes.agent_summary = tree.text(
                            TextProps::new(format!("{review_count} open reviews"))
                                .color(ColorToken::TextMuted),
                        );
                        nodes.reviews = tree.list(ListProps::new(review_count, ROW_HEIGHT));
                    },
                );
            });
            self.panel = tree;
            let mounted = mounted?;
            self.nodes = Some(nodes);
            Ok(mounted)
        }
    }

    impl Extension for T2 {
        fn new() -> Self {
            Self {
                fetch_state: Known::NotAsked,
                process_state: Known::NotAsked,
                edit_state: Known::NotAsked,
                agent_state: Known::NotAsked,
                fetch_result: String::new(),
                process_output: String::new(),
                process_result: String::new(),
                edit_path: "README.md".into(),
                edit_result: String::new(),
                observing: false,
                reviews: Vec::new(),
                panel: SurfaceTree::default(),
                nodes: None,
            }
        }

        fn surfaces(&self) -> Vec<Surface> {
            vec![Surface::panel(
                PANEL,
                "T2 capabilities",
                DockPosition::Right,
                Icon::Lock,
                360,
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
            let nodes = self.nodes()?;
            let (fetch, run, insert, path, toggle) = (
                nodes.fetch,
                nodes.run,
                nodes.insert,
                nodes.edit_path,
                nodes.agent_toggle,
            );
            match event {
                UiEvent::Click(id) if id == fetch => self.fetch_zen(),
                UiEvent::Click(id) if id == run => self.run_git(),
                UiEvent::Click(id) if id == insert => self.insert_header(),
                UiEvent::Click(id) if id == toggle => self.toggle_agent(),
                UiEvent::InputChanged((id, text)) if id == path => {
                    self.panel.set_props_from_host(
                        id,
                        InputProps::new(text.clone())
                            .placeholder("A file in the worktree")
                            .label("File to edit"),
                    )?;
                    self.edit_path = text;
                    return Ok(Update::Unchanged);
                }
                _ => return Ok(Update::Unchanged),
            }
            self.refresh(false)?;
            Ok(self.panel.take_update())
        }

        fn list_rows(
            &mut self,
            surface_id: &str,
            list_node: NodeId,
            start: u32,
            count: u32,
        ) -> Vec<ListRow> {
            let list = self.nodes.as_ref().map(|nodes| nodes.reviews);
            if surface_id != PANEL || list != Some(list_node) {
                return Vec::new();
            }
            list_rows(start, count, self.reviews.len() as u32, |index, tree| {
                let row = &self.reviews[index as usize];
                tree.flex(FlexProps::row().gap(8).align(Align::Center), |tree| {
                    tree.icon(IconProps::new(Icon::FileDiff).color(ColorToken::TextMuted));
                    tree.text(TextProps::new(row.file.clone()).monospace());
                    tree.text(TextProps::new(row.state.clone()).color(ColorToken::TextMuted));
                });
            })
        }

        /// Outcomes update their section and are pushed to the panel.
        fn handle_io(&mut self, event: IoEvent) -> Result<(), String> {
            let mut rows = false;
            match event {
                IoEvent::FetchDone((_, result)) => match result {
                    Ok(response) => {
                        self.fetch_state = Known::Allowed;
                        self.fetch_result = if response.is_success() {
                            response.text().unwrap_or("(not UTF-8)").trim().to_string()
                        } else {
                            format!("HTTP {}", response.status)
                        };
                    }
                    Err(FetchError::Denied) => {
                        self.fetch_state = Known::Denied;
                        self.fetch_result = "Denied.".into();
                    }
                    Err(error) => {
                        self.fetch_state = Known::Allowed;
                        self.fetch_result = format!("Failed: {error}");
                    }
                },
                IoEvent::ProcessOutput(output) => {
                    self.process_state = Known::Allowed;
                    self.process_output.push_str(&output.text());
                }
                IoEvent::ProcessExited(exit) => {
                    self.process_state = Known::Allowed;
                    self.process_result = match (exit.code, exit.killed) {
                        (_, true) => "Killed.".into(),
                        (Some(0), _) if self.process_output.is_empty() => {
                            "Exited with 0: the worktree is clean.".into()
                        }
                        (Some(code), _) => format!("Exited with {code}."),
                        (None, _) => "Ended by a signal.".into(),
                    };
                }
                IoEvent::SpawnFailed((_, error)) => {
                    if error == SpawnError::Denied {
                        self.process_state = Known::Denied;
                    }
                    self.process_result = format!("Could not start: {error}");
                }
                IoEvent::EditDone((_, result)) => match result {
                    Ok(()) => {
                        self.edit_state = Known::Allowed;
                        self.edit_result = format!(
                            "Inserted at the top of {}; undo with Ctrl+Z.",
                            self.edit_path
                        );
                    }
                    Err(EditError::Denied) => {
                        self.edit_state = Known::Denied;
                        self.edit_result = "Denied.".into();
                    }
                    Err(error) => {
                        self.edit_state = Known::Allowed;
                        self.edit_result = format!("Failed: {error}");
                    }
                },
                IoEvent::Agent(event) => {
                    self.agent(event);
                    rows = true;
                }
                IoEvent::PermissionChanged((capability, state)) => {
                    let known = Known::from_state(state);
                    match capability {
                        T2Capability::NetworkFetch => self.fetch_state = known,
                        T2Capability::ProcessSpawn => self.process_state = known,
                        T2Capability::WorkspaceEdit => self.edit_state = known,
                        T2Capability::AgentObserve => {
                            self.agent_state = known;
                            if state != PermissionState::Granted {
                                self.observing = false;
                            }
                        }
                    }
                }
            }
            if self.nodes.is_none() {
                return Ok(());
            }
            self.refresh(rows)?;
            self.panel.push(PANEL).map_err(|error| error.message)
        }
    }
}

/// What a 0.1 or 0.2 build shows: the T2 capabilities need the 0.3 world.
#[cfg(not(feature = "fyx-ui-0-3"))]
pub struct NeedsT2;

#[cfg(not(feature = "fyx-ui-0-3"))]
pub const NEEDS_0_3: &str =
    "This demo needs fyx:ui 0.3. Build it with script/fyx-build-demos --fyx-ui-0-3.";

#[cfg(not(feature = "fyx-ui-0-3"))]
impl Extension for NeedsT2 {
    fn new() -> Self {
        Self
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            PANEL,
            "T2 capabilities",
            DockPosition::Right,
            Icon::Lock,
            360,
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

#[cfg(not(any(feature = "fyx-ui-0-2", feature = "fyx-ui-0-3")))]
register_extension!(NeedsT2, v0_1);
#[cfg(all(feature = "fyx-ui-0-2", not(feature = "fyx-ui-0-3")))]
register_extension!(NeedsT2, v0_2);
#[cfg(feature = "fyx-ui-0-3")]
register_extension!(T2);

#[cfg(all(test, feature = "fyx-ui-0-3"))]
mod tests {
    use super::*;

    fn mounted() -> T2 {
        take_host_effects();
        let mut t2 = T2::new();
        t2.mount(PANEL).expect("mount");
        t2
    }

    fn click(t2: &mut T2, label: &str) {
        let id = t2
            .button(label)
            .unwrap_or_else(|| panic!("no button {label}"));
        t2.handle_event(PANEL, UiEvent::Click(id)).expect("click");
    }

    #[test]
    fn each_button_makes_its_request() {
        let mut t2 = mounted();
        for label in ["Fetch zen", "Run git status", "Insert header", "Start"] {
            click(&mut t2, label);
        }
        let effects = take_host_effects();
        assert!(
            matches!(
                &effects[..],
                [
                    HostEffect::Fetch { request, .. },
                    HostEffect::Spawn { request: spawn_request, .. },
                    HostEffect::ApplyEdit { edit, .. },
                    HostEffect::StartObserving,
                ] if request.url == ZEN_URL
                    && spawn_request.args == ["status", "--short"]
                    && edit.files[0].edits[0].new_text == HEADER
            ),
            "{effects:?}"
        );
        assert_eq!(t2.fetch_state, Known::Waiting);
        assert_eq!(t2.agent_state, Known::Waiting);
    }

    #[test]
    fn outcomes_set_the_state() {
        let mut t2 = mounted();
        t2.handle_io(IoEvent::FetchDone((1, Err(FetchError::Denied))))
            .expect("io");
        assert_eq!(t2.fetch_state, Known::Denied);
        t2.handle_io(IoEvent::PermissionChanged((
            T2Capability::NetworkFetch,
            PermissionState::Granted,
        )))
        .expect("io");
        assert_eq!(t2.fetch_state, Known::Allowed);
        t2.handle_io(IoEvent::EditDone((2, Ok(())))).expect("io");
        assert_eq!(t2.edit_state, Known::Allowed);
    }
}
