//! The "Agent secrets" modal (plan 7.4): add a secret by name and value, see the names, remove one.
//! Values are typed into a masked field, go straight to the system credential store and are never
//! shown again.

use editor::Editor;
use gpui::{DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, Task, Window};
use ui::{Button, ButtonStyle, IconButton, IconName, Label, LabelSize, prelude::*};
use workspace::{ModalView, Workspace};

use crate::secrets_vault::{SecretsVault, check_name};

pub(crate) fn open(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(vault) = SecretsVault::global(cx) else {
        return;
    };
    workspace.toggle_modal(window, cx, |window, cx| {
        SecretsModal::new(vault, window, cx)
    });
}

pub(crate) struct SecretsModal {
    vault: Entity<SecretsVault>,
    name: Entity<Editor>,
    value: Entity<Editor>,
    names: Vec<String>,
    message: Option<(SharedString, bool)>,
    pending: Task<()>,
    focus_handle: FocusHandle,
}

impl SecretsModal {
    fn new(vault: Entity<SecretsVault>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("name, such as github", window, cx);
            editor
        });
        let value = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("value", window, cx);
            editor.set_masked(true, cx);
            editor
        });
        window.focus(&name.focus_handle(cx), cx);
        let mut modal = Self {
            vault,
            name,
            value,
            names: Vec::new(),
            message: None,
            pending: Task::ready(()),
            focus_handle: cx.focus_handle(),
        };
        modal.refresh(cx);
        modal
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let names = self.vault.read(cx).names(cx);
        self.pending = cx.spawn(async move |this, cx| {
            let names = names.await;
            this.update(cx, |this, cx| {
                match names {
                    Ok(names) => this.names = names,
                    Err(error) => this.message = Some((format!("{error:#}").into(), true)),
                }
                cx.notify();
            })
            .ok();
        });
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.name.read(cx).text(cx).trim().to_string();
        let value = self.value.read(cx).text(cx);
        if let Err(error) = check_name(&name) {
            self.message = Some((error.to_string().into(), true));
            cx.notify();
            return;
        }
        let save = self.vault.read(cx).add_secret(&name, &value, cx);
        self.value
            .update(cx, |editor, cx| editor.set_text("", window, cx));
        self.pending = cx.spawn(async move |this, cx| {
            let result = save.await;
            this.update(cx, |this, cx| {
                this.message = Some(match result {
                    Ok(()) => (
                        format!("Saved. Agents can use secret://{name}.").into(),
                        false,
                    ),
                    Err(error) => (format!("{error:#}").into(), true),
                });
                this.refresh(cx);
            })
            .ok();
        });
    }

    fn remove(&mut self, name: String, cx: &mut Context<Self>) {
        let remove = self
            .vault
            .update(cx, |vault, cx| vault.remove_secret(&name, cx));
        self.pending = cx.spawn(async move |this, cx| {
            let result = remove.await;
            this.update(cx, |this, cx| {
                this.message = Some(match result {
                    Ok(()) => (format!("Removed secret://{name}.").into(), false),
                    Err(error) => (format!("{error:#}").into(), true),
                });
                this.refresh(cx);
            })
            .ok();
        });
    }
}

impl EventEmitter<DismissEvent> for SecretsModal {}

impl ModalView for SecretsModal {}

impl Focusable for SecretsModal {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SecretsModal {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let field = |editor: &Entity<Editor>, cx: &mut Context<Self>| {
            div()
                .px_2()
                .py_1()
                .border_1()
                .rounded_sm()
                .border_color(cx.theme().colors().border)
                .child(editor.clone())
        };
        let names = self.names.clone();
        let names = names
            .into_iter()
            .map(|name| {
                let remove = name.clone();
                h_flex()
                    .justify_between()
                    .child(Label::new(format!("secret://{name}")).size(LabelSize::Small))
                    .child(
                        IconButton::new(
                            SharedString::from(format!("remove-{name}")),
                            IconName::Trash,
                        )
                        .tooltip(ui::Tooltip::text("Remove from the credential store"))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.remove(remove.clone(), cx)),
                        ),
                    )
            })
            .collect::<Vec<_>>();
        v_flex()
            .key_context("AgentSecrets")
            .track_focus(&self.focus_handle)
            .w(rems(30.))
            .elevation_3(cx)
            .p_3()
            .gap_2()
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" {
                    this.save(window, cx);
                }
            }))
            .child(Label::new("Agent secrets"))
            .child(
                Label::new(
                    "Agents write secret://name in a command. Fyx asks you each time, puts the \
                     value only into that command's environment and shows the agent the \
                     reference instead of the value.",
                )
                .size(LabelSize::Small)
                .color(Color::Muted),
            )
            .child(
                Label::new(
                    "Limit: a command can still leak a value it changes (splits, hashes, encrypts) or sends to a server. Allow only commands you trust.",
                )
                .size(LabelSize::Small)
                .color(Color::Warning),
            )
            .child(field(&self.name, cx))
            .child(field(&self.value, cx))
            .child(
                h_flex().justify_end().child(
                    Button::new("save-secret", "Save to the credential store")
                        .style(ButtonStyle::Filled)
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                ),
            )
            .when_some(self.message.clone(), |this, (message, error)| {
                this.child(Label::new(message).size(LabelSize::Small).color(if error {
                    Color::Error
                } else {
                    Color::Success
                }))
            })
            .when(!self.names.is_empty(), |this| this.children(names))
    }
}
