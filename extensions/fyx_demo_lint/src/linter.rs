//! The panel and the publishing, on the 0.4 `context` imports.

use ui_ext_sdk::context::{self, Diagnostic, EditorContext, Severity};
use ui_ext_sdk::*;

use crate::{Finding, scan};

const PANEL: &str = "lint";
const ROW_HEIGHT: u16 = 24;

fn diagnostics(findings: &[Finding]) -> Vec<Diagnostic> {
    findings
        .iter()
        .map(|finding| {
            context::line_diagnostic(
                finding.line,
                finding.columns.clone(),
                Severity::Warning,
                finding.message.clone(),
            )
        })
        .collect()
}

struct Nodes {
    status: NodeId,
    list: NodeId,
    clear: NodeId,
}

pub struct LintDemo {
    /// What the host holds for this extension, file by file in the order first published.
    published: Vec<(FileRef, Vec<Finding>)>,
    status: String,
    tree: SurfaceTree,
    nodes: Option<Nodes>,
}

impl LintDemo {
    fn rows(&self) -> Vec<(&FileRef, &Finding)> {
        self.published
            .iter()
            .flat_map(|(file, findings)| findings.iter().map(move |finding| (file, finding)))
            .collect()
    }

    fn row_count(&self) -> u32 {
        u32::try_from(self.rows().len()).unwrap_or(u32::MAX)
    }

    fn status_props(&self) -> TextProps {
        TextProps::new(self.status.as_str()).color(ColorToken::TextMuted)
    }

    /// Lints the selection of the active file, if it holds text.
    fn editor_changed(&mut self, editor: EditorContext) -> Result<(), String> {
        let (Some(file), Some((range, text))) = (editor.active, editor.selection) else {
            return Ok(());
        };
        let Some(text) = text else {
            self.status = format!("{} is private; it is not linted.", file.path);
            return self.push();
        };
        if text.is_empty() {
            return Ok(());
        }
        let findings = scan(range.start.line, range.start.column, &text);
        match context::publish_diagnostics(&file, &diagnostics(&findings)) {
            Ok(()) => {
                self.status = format!(
                    "{} TODO{} in the selection of {}.",
                    findings.len(),
                    if findings.len() == 1 { "" } else { "s" },
                    file.path
                );
                self.published
                    .retain(|(published, _)| published.path != file.path);
                if !findings.is_empty() {
                    self.published.push((file, findings));
                }
            }
            Err(error) => {
                self.status = format!("Could not publish: {}", error.message);
                log(LogLevel::Warn, &self.status);
            }
        }
        self.push()
    }

    fn clear(&mut self) -> Result<Update, String> {
        match context::clear_diagnostics(None) {
            Ok(()) => {
                self.published.clear();
                self.status = "Cleared. Select text in a file to lint it.".to_string();
            }
            Err(error) => self.status = format!("Could not clear: {}", error.message),
        }
        self.sync()
    }

    fn sync(&mut self) -> Result<Update, String> {
        let status = self.status_props();
        let list = ListProps::new(self.row_count(), ROW_HEIGHT);
        let Some(nodes) = &self.nodes else {
            return Ok(Update::Unchanged);
        };
        let (status_node, list_node) = (nodes.status, nodes.list);
        self.tree.set_props(status_node, status)?;
        self.tree.set_props(list_node, list)?;
        self.tree.invalidate_rows(list_node, None)?;
        Ok(self.tree.take_update())
    }

    /// Sends the panel's changes outside an event (after an io event).
    fn push(&mut self) -> Result<(), String> {
        let update = self.sync()?;
        if update != Update::Unchanged
            && let Err(error) = push_update(PANEL, &update)
        {
            log(
                LogLevel::Warn,
                &format!("updating the panel failed: {}", error.message),
            );
        }
        Ok(())
    }
}

impl Extension for LintDemo {
    fn new() -> Self {
        let status = match context::watch(true) {
            Ok(()) => "Select text in a file: every TODO in it is marked.".to_string(),
            Err(error) => format!("The editor context is off: {}", error.message),
        };
        Self {
            published: Vec::new(),
            status,
            tree: SurfaceTree::new(),
            nodes: None,
        }
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            PANEL,
            "TODO lint",
            DockPosition::Right,
            Icon::Warning,
            320,
        )]
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        if surface_id != PANEL {
            return Err(format!("unknown surface {surface_id:?}"));
        }
        let status = self.status_props();
        let list = ListProps::new(self.row_count(), ROW_HEIGHT);
        let mut nodes = None;
        let mut tree = SurfaceTree::new();
        let mounted = tree.mount(|tree| {
            tree.flex(FlexProps::column().gap(8).padding(8).grow(), |tree| {
                tree.text(TextProps::new("TODO lint").bold());
                let status = tree.text(status);
                let list = tree.list(list);
                let clear = tree.button(ButtonProps::new("Clear").icon(Icon::Trash));
                nodes = Some(Nodes {
                    status,
                    list,
                    clear,
                });
            });
        })?;
        self.tree = tree;
        self.nodes = nodes;
        Ok(mounted)
    }

    fn handle_event(&mut self, surface_id: &str, event: UiEvent) -> Result<Update, String> {
        if surface_id != PANEL {
            return Err(format!("unknown surface {surface_id:?}"));
        }
        self.tree.apply_event(&event);
        let Some(nodes) = &self.nodes else {
            return Ok(Update::Unchanged);
        };
        match event {
            UiEvent::Click(id) if id == nodes.clear => self.clear(),
            UiEvent::RowClick((id, index)) if id == nodes.list => {
                let target = usize::try_from(index).ok().and_then(|index| {
                    self.rows().get(index).map(|(file, finding)| {
                        ((*file).clone(), finding.line, finding.columns.clone())
                    })
                });
                if let Some((file, line, columns)) = target {
                    let range = TextRange::new(
                        Position::new(line, columns.start),
                        Position::new(line, columns.end),
                    );
                    if let Err(error) = context::open(&file, Some(range), true) {
                        self.status = format!("Could not open {}: {}", file.path, error.message);
                        return self.sync();
                    }
                }
                Ok(Update::Unchanged)
            }
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
        if surface_id != PANEL
            || self
                .nodes
                .as_ref()
                .is_none_or(|nodes| nodes.list != list_node)
        {
            return Vec::new();
        }
        let rows = self.rows();
        list_rows(start, count, self.row_count(), |index, tree| {
            let Some((file, finding)) = usize::try_from(index)
                .ok()
                .and_then(|index| rows.get(index))
            else {
                return;
            };
            tree.flex(
                FlexProps::row().gap(8).padding(4).align(Align::Center),
                |tree| {
                    tree.text(
                        TextProps::new(format!("{}:{}", file.path, finding.line + 1))
                            .monospace()
                            .color(ColorToken::TextMuted),
                    );
                    tree.text(finding.message.as_str());
                },
            );
        })
    }

    fn handle_io(&mut self, event: IoEvent) -> Result<(), String> {
        match event {
            IoEvent::ContextChanged(editor) => self.editor_changed(editor),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ui_ext_sdk::context::ContextEffect;

    fn selection(path: &str, text: Option<&str>) -> EditorContext {
        let file = FileRef {
            worktree: Some(1),
            path: path.into(),
        };
        EditorContext {
            active: Some(file.clone()),
            open: vec![file],
            selection: Some((
                TextRange::new(Position::new(0, 0), Position::new(1, 0)),
                text.map(Into::into),
            )),
            trusted: Some(true),
        }
    }

    #[test]
    fn a_selection_publishes_its_todos_for_the_file() {
        let mut demo = LintDemo::new();
        demo.mount(PANEL).unwrap();
        take_host_effects();
        demo.handle_io(IoEvent::ContextChanged(selection(
            "a.rs",
            Some("// TODO x\nok"),
        )))
        .unwrap();
        demo.handle_io(IoEvent::ContextChanged(selection(".env", None)))
            .unwrap();
        let published: Vec<_> = take_host_effects()
            .into_iter()
            .filter_map(|effect| match effect {
                HostEffect::Context(ContextEffect::PublishDiagnostics { path, diagnostics }) => {
                    Some((path.path, diagnostics.len()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(published, [("a.rs".to_string(), 1)]);
        assert_eq!(demo.row_count(), 1);

        let clear = demo.nodes.as_ref().unwrap().clear;
        demo.handle_event(PANEL, UiEvent::Click(clear)).unwrap();
        assert_eq!(demo.row_count(), 0);
        assert!(take_host_effects().iter().any(|effect| matches!(
            effect,
            HostEffect::Context(ContextEffect::ClearDiagnostics(None))
        )));
    }
}
