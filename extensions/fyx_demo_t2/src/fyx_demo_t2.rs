//! Fyx demo 10: the T2 capabilities of the staged `fyx:ui` 0.3, for checking their consent flows
//! by hand (`docs/fyx/wit-0.3/design.md` §7 to §11). It builds only for 0.3: the default build
//! shows a panel that says so.
//!
//! One panel `"t2"` (right dock, title "T2 capabilities", icon `lock`), with four sections. Each
//! shows its permission state (`permissions.query`), a `Request …` button that shows Fyx's
//! consent prompt (`permissions.request`, enabled while the state is `ask`), and an action, whose
//! first use shows the prompt too:
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
//! The state comes from `query_permission` when the panel mounts and with every event and
//! outcome, from the `permission-changed` the host sends when the user answers, and from outcomes:
//! - `waiting for an answer…` lasts until the next event or outcome. A prompt closed without an
//!   answer sends nothing, so the next query finds `ask` again and `Request …` is enabled again.
//! - A refusal with a reason the demo knows says what to do: an untrusted worktree reads
//!   `Trust this project to …`, no open worktree `Open a folder …`. Any other refusal shows the
//!   host's message with the queried permission state next to it.

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
        /// Refused because the worktree is not trusted, whatever the permission says.
        Untrusted,
        /// Refused because no worktree is open.
        NoWorktree,
    }

    impl Known {
        fn label(self, capability: T2Capability) -> &'static str {
            match self {
                Known::NotAsked => "Permission: not asked yet",
                Known::Waiting => "Permission: waiting for an answer…",
                Known::Allowed => "Permission: allowed",
                Known::Denied => "Permission: denied (change it on the extensions page)",
                Known::Unavailable => "Permission: not available here",
                Known::NotDeclared => "Permission: not declared in extension.toml",
                Known::Untrusted => match capability {
                    T2Capability::ProcessSpawn => "Trust this project to use git",
                    T2Capability::WorkspaceEdit => "Trust this project to edit its files",
                    T2Capability::NetworkFetch | T2Capability::AgentObserve => {
                        "Trust this project to use this"
                    }
                },
                Known::NoWorktree => "Open a folder to use this",
            }
        }

        fn color(self) -> ColorToken {
            match self {
                Known::Allowed => ColorToken::Success,
                Known::Denied | Known::NotDeclared => ColorToken::Error,
                Known::Unavailable | Known::Untrusted | Known::NoWorktree => ColorToken::Warning,
                Known::NotAsked | Known::Waiting => ColorToken::TextMuted,
            }
        }

        /// Whether this is a condition outside the permission, which a query cannot see.
        fn is_condition(self) -> bool {
            matches!(self, Known::Untrusted | Known::NoWorktree)
        }

        /// From an import's refusal. The host's reasons the demo knows (an untrusted worktree,
        /// none open) say what to do; any other `denied` or `unavailable` is the queried state.
        /// The rest is the call's own problem.
        fn from_error(error: &HostError, queried: PermissionState, before: Known) -> Known {
            match error.code {
                HostErrorCode::UntrustedWorktree => Known::Untrusted,
                HostErrorCode::NoWorktree => Known::NoWorktree,
                HostErrorCode::Denied => match queried {
                    PermissionState::Denied | PermissionState::Unavailable => {
                        Known::from_state(queried)
                    }
                    // The broker allows it, so the refusal has a reason the demo does not know:
                    // the result line shows it.
                    PermissionState::Granted | PermissionState::Ask => Known::Denied,
                },
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

    /// The capability of each section, in order.
    pub const SECTIONS: [(T2Capability, &str); 4] = [
        (T2Capability::NetworkFetch, "Request fetch"),
        (T2Capability::ProcessSpawn, "Request process"),
        (T2Capability::WorkspaceEdit, "Request edit"),
        (T2Capability::AgentObserve, "Request agent"),
    ];

    #[derive(Default)]
    struct Nodes {
        /// The `Request …` buttons, in [`SECTIONS`] order.
        requests: [NodeId; 4],
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
        /// The states the last query answered, in [`SECTIONS`] order.
        queried: [PermissionState; 4],
        panel: SurfaceTree,
        nodes: Option<Nodes>,
    }

    fn section_title(tree: &mut TreeBuilder, title: &str) {
        tree.text(TextProps::new(title).bold());
    }

    fn state_text(known: Known, capability: T2Capability) -> TextProps {
        TextProps::new(known.label(capability)).color(known.color())
    }

    fn state_name(state: PermissionState) -> &'static str {
        match state {
            PermissionState::Granted => "granted",
            PermissionState::Denied => "denied",
            PermissionState::Ask => "ask",
            PermissionState::Unavailable => "unavailable",
        }
    }

    /// An import's refusal for the result line, with the queried permission state next to it.
    fn refusal(error: &HostError, queried: PermissionState) -> String {
        format!(
            "{}: {} (permission: {})",
            error.code.name(),
            error.message,
            state_name(queried)
        )
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
                .chain(nodes.requests)
                .find(|id| {
                    matches!(self.panel.props(*id), Some(Props::Button(button)) if button.label == label)
                })
        }

        fn state(&self, capability: T2Capability) -> Known {
            match capability {
                T2Capability::NetworkFetch => self.fetch_state,
                T2Capability::ProcessSpawn => self.process_state,
                T2Capability::WorkspaceEdit => self.edit_state,
                T2Capability::AgentObserve => self.agent_state,
            }
        }

        fn set_state(&mut self, capability: T2Capability, known: Known) {
            match capability {
                T2Capability::NetworkFetch => self.fetch_state = known,
                T2Capability::ProcessSpawn => self.process_state = known,
                T2Capability::WorkspaceEdit => self.edit_state = known,
                T2Capability::AgentObserve => self.agent_state = known,
            }
        }

        fn queried(&self, capability: T2Capability) -> PermissionState {
            let index = SECTIONS.iter().position(|(known, _)| *known == capability);
            index.map_or(PermissionState::Unavailable, |index| self.queried[index])
        }

        /// The host's states as of this call, at the start of every call.
        /// - `waiting` from an earlier call ends here: a prompt closed without an answer sends
        ///   nothing, and the query finds `ask` again.
        /// - An untrusted or missing worktree stays while the permission would allow the action:
        ///   the query cannot see it.
        fn query_states(&mut self) {
            for (index, (capability, _)) in SECTIONS.into_iter().enumerate() {
                let state = query_permission(capability);
                self.queried[index] = state;
                let allows = matches!(state, PermissionState::Granted | PermissionState::Ask);
                if !(self.state(capability).is_condition() && allows) {
                    self.set_state(capability, Known::from_state(state));
                }
            }
        }

        /// Enabled while the queried state is `ask`, also while a prompt may be open.
        fn request_button(label: &str, queried: PermissionState) -> ButtonProps {
            ButtonProps::new(label)
                .icon(Icon::Lock)
                .style(ButtonStyle::Subtle)
                .disabled(queried != PermissionState::Ask)
                .tooltip("Shows Fyx's permission prompt now")
        }

        fn request(&mut self, capability: T2Capability) {
            let queried = self.queried(capability);
            match request_permission(capability) {
                Ok(()) => {
                    if queried == PermissionState::Ask {
                        self.set_state(capability, Known::Waiting);
                    }
                }
                Err(error) => {
                    let known = Known::from_error(&error, queried, self.state(capability));
                    self.set_state(capability, known);
                }
            }
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
                (
                    nodes.fetch_state,
                    Props::from(state_text(self.fetch_state, T2Capability::NetworkFetch)),
                ),
                (
                    nodes.fetch_result,
                    TextProps::new(self.fetch_result.clone()).into(),
                ),
                (
                    nodes.process_state,
                    state_text(self.process_state, T2Capability::ProcessSpawn).into(),
                ),
                (
                    nodes.process_result,
                    Props::Markdown(self.process_markdown()),
                ),
                (
                    nodes.edit_state,
                    state_text(self.edit_state, T2Capability::WorkspaceEdit).into(),
                ),
                (
                    nodes.edit_result,
                    TextProps::new(self.edit_result.clone()).into(),
                ),
                (
                    nodes.agent_state,
                    state_text(self.agent_state, T2Capability::AgentObserve).into(),
                ),
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
            let requests = nodes.requests;
            for (id, props) in ids {
                self.panel.set_props(id, props)?;
            }
            for ((capability, label), id) in SECTIONS.into_iter().zip(requests) {
                let props = Self::request_button(label, self.queried(capability));
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
                    let queried = self.queried(T2Capability::NetworkFetch);
                    self.fetch_state = Known::from_error(&error, queried, self.fetch_state);
                    self.fetch_result = refusal(&error, queried);
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
                    let queried = self.queried(T2Capability::ProcessSpawn);
                    self.process_state = Known::from_error(&error, queried, self.process_state);
                    self.process_result = refusal(&error, queried);
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
                    let queried = self.queried(T2Capability::WorkspaceEdit);
                    self.edit_state = Known::from_error(&error, queried, self.edit_state);
                    self.edit_result = refusal(&error, queried);
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
                Err(error) => {
                    let queried = self.queried(T2Capability::AgentObserve);
                    self.agent_state = Known::from_error(&error, queried, self.agent_state);
                    log(LogLevel::Info, &refusal(&error, queried));
                }
            }
        }

        fn agent(&mut self, event: AgentEvent) {
            self.agent_state = Known::Allowed;
            match event {
                AgentEvent::Snapshot(snapshot) => {
                    self.reviews = snapshot.reviews.iter().map(review_row).collect();
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
            self.query_states();
            let request_props = SECTIONS
                .map(|(capability, label)| Self::request_button(label, self.queried(capability)));
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
                        let [fetch_request, process_request, edit_request, agent_request] =
                            request_props;
                        section_title(tree, "Fetch: api.github.com/zen");
                        nodes.fetch_state = tree.text(state_text(fetch_state, T2Capability::NetworkFetch));
                        nodes.requests[0] = tree.button(fetch_request);
                        nodes.fetch = tree.button(
                            ButtonProps::new("Fetch zen")
                                .icon(Icon::Globe)
                                .tooltip("GET api.github.com/zen; asks for permission the first time"),
                        );
                        nodes.fetch_result = tree.text(TextProps::new(fetch_result));

                        section_title(tree, "Process: git status --short");
                        nodes.process_state = tree.text(state_text(process_state, T2Capability::ProcessSpawn));
                        nodes.requests[1] = tree.button(process_request);
                        nodes.run = tree.button(
                            ButtonProps::new("Run git status")
                                .icon(Icon::Terminal)
                                .tooltip("Runs git in the worktree; asks for permission the first time"),
                        );
                        nodes.process_result = tree.markdown(process_markdown);

                        section_title(tree, "Edit: insert a header comment");
                        nodes.edit_state = tree.text(state_text(edit_state, T2Capability::WorkspaceEdit));
                        nodes.requests[2] = tree.button(edit_request);
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
                        nodes.agent_state = tree.text(state_text(agent_state, T2Capability::AgentObserve));
                        nodes.requests[3] = tree.button(agent_request);
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
                queried: [PermissionState::Ask; 4],
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
            self.query_states();
            let nodes = self.nodes()?;
            let requests = nodes.requests;
            let (fetch, run, insert, path, toggle) = (
                nodes.fetch,
                nodes.run,
                nodes.insert,
                nodes.edit_path,
                nodes.agent_toggle,
            );
            match event {
                UiEvent::Click(id) if requests.contains(&id) => {
                    let index = requests.iter().position(|request| *request == id);
                    if let Some((capability, _)) = index.map(|index| SECTIONS[index]) {
                        self.request(capability);
                    }
                }
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
            self.query_states();
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
                        let queried = self.queried(T2Capability::NetworkFetch);
                        self.fetch_state = Known::Denied;
                        self.fetch_result =
                            format!("Denied (permission: {}).", state_name(queried));
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
                    let queried = self.queried(T2Capability::ProcessSpawn);
                    if error == SpawnError::Denied {
                        self.process_state = Known::Denied;
                    }
                    self.process_result = format!(
                        "Could not start: {error} (permission: {})",
                        state_name(queried)
                    );
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
                        let queried = self.queried(T2Capability::WorkspaceEdit);
                        self.edit_state = Known::Denied;
                        self.edit_result = format!("Denied (permission: {}).", state_name(queried));
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

    #[cfg(test)]
    mod tests {
        use super::*;

        fn click(t2: &mut T2, label: &str) {
            let id = t2
                .button(label)
                .unwrap_or_else(|| panic!("no button {label}"));
            t2.handle_event(PANEL, UiEvent::Click(id)).expect("click");
        }

        #[test]
        fn a_prompt_closed_without_an_answer_asks_again() {
            take_host_effects();
            let mut t2 = T2::new();
            t2.mount(PANEL).expect("mount");
            click(&mut t2, "Request fetch");
            assert_eq!(t2.fetch_state, Known::Waiting);
            // Closed without an answer: no event, and the next one finds `ask` again.
            click(&mut t2, "Request edit");
            assert_eq!(t2.fetch_state, Known::NotAsked);
            let request = t2.button("Request fetch").expect("button");
            assert!(matches!(
                t2.panel.props(request),
                Some(Props::Button(button)) if !button.disabled
            ));
        }

        #[test]
        fn refusals_say_why() {
            let untrusted = HostError {
                code: HostErrorCode::UntrustedWorktree,
                message: "the worktree is restricted: it is not trusted".into(),
            };
            let known = Known::from_error(&untrusted, PermissionState::Granted, Known::Allowed);
            assert_eq!(known, Known::Untrusted);
            assert_eq!(
                known.label(T2Capability::ProcessSpawn),
                "Trust this project to use git"
            );
            let none_open = HostError {
                code: HostErrorCode::NoWorktree,
                message: "no worktree is open".into(),
            };
            assert_eq!(
                Known::from_error(&none_open, PermissionState::Granted, Known::Allowed),
                Known::NoWorktree
            );
            let broker = HostError {
                code: HostErrorCode::Denied,
                message: "network:fetch is Unavailable".into(),
            };
            assert_eq!(
                Known::from_error(&broker, PermissionState::Unavailable, Known::NotAsked),
                Known::Unavailable
            );
            assert_eq!(
                refusal(&broker, PermissionState::Unavailable),
                "denied: network:fetch is Unavailable (permission: unavailable)"
            );
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
        // Each click reads the states again, so only the last one still waits.
        assert_eq!(t2.fetch_state, Known::NotAsked);
        assert_eq!(t2.agent_state, Known::Waiting);
    }

    #[test]
    fn the_state_shows_up_front_and_request_asks() {
        set_permission_state(T2Capability::NetworkFetch, PermissionState::Granted);
        set_permission_state(T2Capability::AgentObserve, PermissionState::Denied);
        let mut t2 = mounted();
        assert_eq!(t2.fetch_state, Known::Allowed);
        assert_eq!(t2.agent_state, Known::Denied);
        assert_eq!(t2.process_state, Known::NotAsked);
        click(&mut t2, "Request process");
        assert_eq!(
            take_host_effects(),
            [HostEffect::RequestPermission(T2Capability::ProcessSpawn)]
        );
        assert_eq!(t2.process_state, Known::Waiting);
        t2.handle_io(IoEvent::PermissionChanged((
            T2Capability::ProcessSpawn,
            PermissionState::Granted,
        )))
        .expect("io");
        assert_eq!(t2.process_state, Known::Allowed);
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
