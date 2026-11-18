//! Fyx demo: a Markdown preview panel.
//!
//! `fyx:ui@0.1.0` inputs are single-line, so the document is built line by line: the input holds
//! the line being typed, Enter appends it to the document (an empty line is a paragraph break),
//! and the `markdown` node renders the document plus the line being typed.
//!
//! Host tests and the docs rely on this behaviour, so keep it stable:
//!
//! - One surface, id `"markdown"`: a right dock panel titled "Markdown preview", icon
//!   `file_markdown`, default size 360.
//! - The first `mount("markdown")` returns 10 nodes, parents first:
//!
//!   | id | parent | props |
//!   |---|---|---|
//!   | 1 | none | column flex, gap 8, padding 8, grow |
//!   | 2 | 1 | row flex, gap 8, align center |
//!   | 3 | 2 | text `Markdown preview`, bold |
//!   | 4 | 2 | text `3 lines, 90 characters`, `text-muted` |
//!   | 5 | 1 | input, empty, placeholder `Type a line of Markdown, Enter adds it` |
//!   | 6 | 1 | row flex, gap 4 |
//!   | 7 | 6 | button `Undo line`, icon `undo`, subtle, disabled while the document is empty |
//!   | 8 | 6 | button `Clear`, icon `trash`, subtle, disabled while the document and input are empty |
//!   | 9 | 1 | column flex, padding 8, grow, background `elevated-surface` |
//!   | 10 | 9 | markdown: the document, or `*Nothing to preview yet.*` when empty |
//!
//! - Every event that changes something answers with `set-props` patches, never `replace`:
//!   `input-changed(5)` sets 4, 8 and 10 when they change (never 5 itself, so typing is not
//!   echoed back); `input-submitted(5)` appends the line and also sets 5 to empty and 7;
//!   `click(7)` drops the last line; `click(8)` empties the document and the input. Events that
//!   change nothing answer `unchanged`. An unknown surface id is an error.
//! - The `SurfaceTree` id allocator survives remounts, so a second mount uses ids 11 to 20. The
//!   document and the typed line survive a remount.

use ui_ext_sdk::*;

const SURFACE_ID: &str = "markdown";
const PLACEHOLDER: &str = "Type a line of Markdown, Enter adds it";
const EMPTY_PREVIEW: &str = "*Nothing to preview yet.*";
const INITIAL_DOCUMENT: &[&str] = &[
    "# Notes",
    "Type a line below and press **Enter** to add it.",
    "- `Undo line` drops the last line",
];

struct MarkdownNodes {
    stats: NodeId,
    input: NodeId,
    undo: NodeId,
    clear: NodeId,
    preview: NodeId,
}

struct MarkdownPanel {
    lines: Vec<String>,
    /// The line being typed, previewed but not yet part of `lines`.
    draft: String,
    tree: SurfaceTree,
    nodes: Option<MarkdownNodes>,
}

impl MarkdownPanel {
    fn source(&self) -> String {
        let mut source = self.lines.join("\n");
        if !self.draft.is_empty() {
            if !source.is_empty() {
                source.push('\n');
            }
            source.push_str(&self.draft);
        }
        source
    }

    fn line_count(&self) -> usize {
        self.lines.len() + usize::from(!self.draft.is_empty())
    }

    fn stats(&self) -> TextProps {
        let lines = self.line_count();
        let characters = self.source().chars().count();
        let lines_label = if lines == 1 { "line" } else { "lines" };
        let characters_label = if characters == 1 {
            "character"
        } else {
            "characters"
        };
        TextProps::new(format!(
            "{lines} {lines_label}, {characters} {characters_label}"
        ))
        .color(ColorToken::TextMuted)
    }

    fn undo(&self) -> ButtonProps {
        ButtonProps::new("Undo line")
            .icon(Icon::Undo)
            .style(ButtonStyle::Subtle)
            .disabled(self.lines.is_empty())
    }

    fn clear(&self) -> ButtonProps {
        ButtonProps::new("Clear")
            .icon(Icon::Trash)
            .style(ButtonStyle::Subtle)
            .disabled(self.lines.is_empty() && self.draft.is_empty())
    }

    fn preview(&self) -> Props {
        let source = self.source();
        if source.trim().is_empty() {
            Props::Markdown(EMPTY_PREVIEW.to_string())
        } else {
            Props::Markdown(source)
        }
    }

    fn sync(&mut self) -> Result<Update, String> {
        let (stats, undo, clear, preview) =
            (self.stats(), self.undo(), self.clear(), self.preview());
        let Some(nodes) = &self.nodes else {
            return Err(TreeError::NotMounted.into());
        };
        self.tree.set_props(nodes.stats, stats)?;
        self.tree.set_props(nodes.undo, undo)?;
        self.tree.set_props(nodes.clear, clear)?;
        self.tree.set_props(nodes.preview, preview)?;
        Ok(self.tree.take_update())
    }

    /// Empties the input on screen. The host already shows `shown`, which the mirror records
    /// first: if the mirror still held an older value equal to the new one, the `set-props` would
    /// be dropped as "no change" and the field would keep its text.
    fn clear_input(&mut self, input: NodeId, shown: &str) -> Result<(), TreeError> {
        self.tree.set_props_from_host(input, draft_input(shown))?;
        self.tree.set_props(input, draft_input(""))?;
        Ok(())
    }
}

fn draft_input(text: &str) -> InputProps {
    InputProps::new(text).placeholder(PLACEHOLDER)
}

fn check_surface(surface_id: &str) -> Result<(), String> {
    if surface_id == SURFACE_ID {
        Ok(())
    } else {
        Err(format!("unknown surface {surface_id:?}"))
    }
}

impl Extension for MarkdownPanel {
    fn new() -> Self {
        Self {
            lines: INITIAL_DOCUMENT
                .iter()
                .map(|line| line.to_string())
                .collect(),
            draft: String::new(),
            tree: SurfaceTree::new(),
            nodes: None,
        }
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            SURFACE_ID,
            "Markdown preview",
            DockPosition::Right,
            Icon::FileText,
            360,
        )]
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        check_surface(surface_id)?;
        let (stats, undo, clear, preview) =
            (self.stats(), self.undo(), self.clear(), self.preview());
        let input = draft_input(&self.draft);
        let mut nodes = MarkdownNodes {
            stats: 0,
            input: 0,
            undo: 0,
            clear: 0,
            preview: 0,
        };
        let mounted = self.tree.mount(|tree| {
            tree.flex(FlexProps::column().gap(8).padding(8).grow(), |tree| {
                tree.flex(FlexProps::row().gap(8).align(Align::Center), |tree| {
                    tree.text(TextProps::new("Markdown preview").bold());
                    nodes.stats = tree.text(stats);
                });
                nodes.input = tree.input(input);
                tree.flex(FlexProps::row().gap(4), |tree| {
                    nodes.undo = tree.button(undo);
                    nodes.clear = tree.button(clear);
                });
                tree.flex(
                    FlexProps::column()
                        .padding(8)
                        .grow()
                        .background(ColorToken::ElevatedSurface),
                    |tree| {
                        nodes.preview = tree.node(preview);
                    },
                );
            });
        })?;
        self.nodes = Some(nodes);
        Ok(mounted)
    }

    fn handle_event(&mut self, surface_id: &str, event: UiEvent) -> Result<Update, String> {
        check_surface(surface_id)?;
        let Some(nodes) = &self.nodes else {
            return Err(TreeError::NotMounted.into());
        };
        let (input, undo, clear) = (nodes.input, nodes.undo, nodes.clear);
        match event {
            UiEvent::InputChanged((id, text)) if id == input => {
                if text == self.draft {
                    return Ok(Update::Unchanged);
                }
                // The user owns the text while typing: record it without echoing it back.
                self.tree.set_props_from_host(input, draft_input(&text))?;
                self.draft = text;
                self.sync()
            }
            UiEvent::InputSubmitted((id, text)) if id == input => {
                self.clear_input(input, &text)?;
                self.lines.push(text);
                self.draft.clear();
                self.sync()
            }
            UiEvent::Click(id) if id == undo => {
                if self.lines.pop().is_none() {
                    return Ok(Update::Unchanged);
                }
                self.sync()
            }
            UiEvent::Click(id) if id == clear => {
                if self.lines.is_empty() && self.draft.is_empty() {
                    return Ok(Update::Unchanged);
                }
                let shown = std::mem::take(&mut self.draft);
                self.clear_input(input, &shown)?;
                self.lines.clear();
                self.sync()
            }
            UiEvent::Click(_)
            | UiEvent::InputChanged(_)
            | UiEvent::InputSubmitted(_)
            | UiEvent::RowClick(_)
            | UiEvent::RowSelected(_)
            | UiEvent::Dismissed(_)
            | UiEvent::NotificationAction(_)
            | UiEvent::NotificationClosed(_)
            | UiEvent::ProgressCancelRequested(_) => Ok(Update::Unchanged),
        }
    }
}

// fyx:ui 0.1 by default, which is what Fyx runs today; `--features fyx-ui-0-2` builds the 0.2
// world from the same code.
#[cfg(not(feature = "fyx-ui-0-2"))]
register_extension!(MarkdownPanel, v0_1);
#[cfg(feature = "fyx-ui-0-2")]
register_extension!(MarkdownPanel);

#[cfg(test)]
mod tests {
    use super::*;

    fn set_props(update: Update) -> Vec<(NodeId, Props)> {
        let Update::Patches(patches) = update else {
            panic!("expected patches, got {update:?}");
        };
        patches
            .into_iter()
            .map(|patch| match patch {
                Patch::SetProps(set) => set,
                patch => panic!("expected only set-props, got {patch:?}"),
            })
            .collect()
    }

    fn ids(patches: &[(NodeId, Props)]) -> Vec<NodeId> {
        patches.iter().map(|(id, _)| *id).collect()
    }

    fn markdown(patches: &[(NodeId, Props)]) -> Option<&str> {
        patches.iter().find_map(|(_, props)| match props {
            Props::Markdown(source) => Some(source.as_str()),
            _ => None,
        })
    }

    #[test]
    fn first_mount_is_pinned() {
        let mut panel = MarkdownPanel::new();
        assert_eq!(panel.surfaces()[0].id, "markdown");
        let nodes = panel.mount("markdown").expect("mount");
        let shape = nodes
            .iter()
            .map(|node| (node.id, node.parent))
            .collect::<Vec<_>>();
        assert_eq!(
            shape,
            [
                (1, None),
                (2, Some(1)),
                (3, Some(2)),
                (4, Some(2)),
                (5, Some(1)),
                (6, Some(1)),
                (7, Some(6)),
                (8, Some(6)),
                (9, Some(1)),
                (10, Some(9)),
            ]
        );
        assert_eq!(
            nodes[3].props,
            TextProps::new("3 lines, 90 characters")
                .color(ColorToken::TextMuted)
                .into()
        );
        assert_eq!(nodes[4].props, draft_input("").into());
        assert_eq!(nodes[9].props, Props::Markdown(INITIAL_DOCUMENT.join("\n")));
    }

    #[test]
    fn typing_previews_without_echo() {
        let mut panel = MarkdownPanel::new();
        panel.mount("markdown").expect("mount");

        let update = panel
            .handle_event("markdown", UiEvent::InputChanged((5, "> quote".into())))
            .expect("type");
        let patches = set_props(update);
        assert_eq!(ids(&patches), [4, 10]);
        assert_eq!(
            markdown(&patches),
            Some(format!("{}\n> quote", INITIAL_DOCUMENT.join("\n")).as_str())
        );

        let same = panel
            .handle_event("markdown", UiEvent::InputChanged((5, "> quote".into())))
            .expect("same text");
        assert_eq!(same, Update::Unchanged);

        let update = panel
            .handle_event("markdown", UiEvent::InputSubmitted((5, "> quoted".into())))
            .expect("submit");
        let patches = set_props(update);
        assert_eq!(ids(&patches), [5, 4, 10]);
        assert_eq!(patches[0].1, draft_input("").into());
        assert_eq!(panel.lines.last().map(String::as_str), Some("> quoted"));
        assert_eq!(panel.line_count(), 4);
    }

    #[test]
    fn buttons_edit_the_document() {
        let mut panel = MarkdownPanel::new();
        panel.mount("markdown").expect("mount");

        let patches = set_props(
            panel
                .handle_event("markdown", UiEvent::Click(7))
                .expect("undo"),
        );
        assert_eq!(ids(&patches), [4, 10]);
        assert_eq!(panel.lines.len(), 2);

        panel
            .handle_event("markdown", UiEvent::InputChanged((5, "draft".into())))
            .expect("type");
        let patches = set_props(
            panel
                .handle_event("markdown", UiEvent::Click(8))
                .expect("clear"),
        );
        assert_eq!(ids(&patches), [5, 4, 7, 8, 10]);
        assert_eq!(patches[0].1, draft_input("").into());
        assert_eq!(markdown(&patches), Some(EMPTY_PREVIEW));

        assert_eq!(
            panel
                .handle_event("markdown", UiEvent::Click(7))
                .expect("undo on empty"),
            Update::Unchanged
        );
        assert_eq!(
            panel
                .handle_event("markdown", UiEvent::Click(8))
                .expect("clear on empty"),
            Update::Unchanged
        );
        assert!(panel.handle_event("other", UiEvent::Click(7)).is_err());
    }

    #[test]
    fn state_survives_a_remount() {
        let mut panel = MarkdownPanel::new();
        panel.mount("markdown").expect("mount");
        panel
            .handle_event("markdown", UiEvent::InputChanged((5, "kept".into())))
            .expect("type");

        let nodes = panel.mount("markdown").expect("remount");
        let ids = nodes.iter().map(|node| node.id).collect::<Vec<_>>();
        assert_eq!(ids, (11..=20).collect::<Vec<_>>());
        assert_eq!(nodes[4].props, draft_input("kept").into());
        assert_eq!(
            panel
                .handle_event("markdown", UiEvent::Click(7))
                .expect("stale id"),
            Update::Unchanged
        );
    }

    /// Every update, applied to the host's own tree model, must be accepted and leave it equal to
    /// the extension's mirror.
    fn assert_host_accepts(panel: &mut MarkdownPanel, events: Vec<UiEvent>) {
        let nodes = panel.mount(SURFACE_ID).expect("mount");
        let mut host = ui_ext::Tree::from_nodes(
            nodes.iter().map(Into::into).collect(),
            TreeLimits::default(),
        )
        .expect("the host accepts the mount");
        for event in events {
            // Like the host: typed text lands in its own tree before the event is sent.
            if let UiEvent::InputChanged((id, value)) | UiEvent::InputSubmitted((id, value)) =
                &event
                && let Some(ui_ext::Props::Input(input)) =
                    host.get(*id).map(|node| node.props().clone())
            {
                let typed = ui_ext::Props::Input(ui_ext::InputProps {
                    value: value.clone(),
                    ..input
                });
                host.apply_patches(vec![ui_ext::Patch::SetProps(*id, typed)])
                    .expect("the host records typed text");
            }
            let update = panel.handle_event(SURFACE_ID, event).expect("handle-event");
            assert!(
                !matches!(update, Update::Replace(_)),
                "events answer with patches, never replace"
            );
            host.apply_update((&update).into())
                .expect("the host accepts the update");
            let mirror = panel
                .tree
                .to_nodes()
                .iter()
                .map(Into::into)
                .collect::<Vec<ui_ext::Node>>();
            assert_eq!(host.to_nodes(), mirror);
        }
    }

    #[test]
    fn host_accepts_every_update() {
        let mut panel = MarkdownPanel::new();
        assert_host_accepts(
            &mut panel,
            vec![
                UiEvent::InputChanged((5, "## Title".into())),
                UiEvent::InputSubmitted((5, "## Title".into())),
                UiEvent::InputSubmitted((5, "".into())),
                UiEvent::Click(7),
                UiEvent::InputChanged((5, "draft".into())),
                UiEvent::Click(8),
                UiEvent::Click(8),
                UiEvent::Click(7),
                UiEvent::InputSubmitted((5, "*back*".into())),
            ],
        );
    }
}
