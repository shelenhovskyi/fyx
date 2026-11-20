//! The Settings section of an extension on the Fyx Extensions page (design wit-0.4 §D.3, plan
//! 6.1): one control per `[[fyx_ui.settings]]` entry, drawn from the declarations.
//!
//! | Type | Control |
//! |---|---|
//! | `boolean` | switch |
//! | `integer`, `number` | one-line field, parsed and checked against `min`/`max` |
//! | `text` | one-line field |
//! | `choice` | dropdown of the titles |
//! | `text-list` | one field per item, with add and remove |
//!
//! Each control shows the title, the description, "Reset to default" while the user settings
//! file holds a value, and the error of an invalid value. A field's text is written only when it
//! passes the declaration's check; until then its error shows under it and the file keeps the
//! last valid value. Keys the extension no longer declares are greyed out with their own reset.
//! The values come from [`ExtensionSettingsStore`] and are re-read whenever Zed's settings change
//! (a hand edit of `settings.json` too).

use std::collections::HashMap;

use editor::{Editor, EditorEvent};
use gpui::{
    App, Context, Entity, FocusHandle, Focusable as _, Global, SharedString, Subscription,
    WeakEntity, Window, prelude::*,
};
use serde_json::Value;
use ui::{
    Button, ButtonCommon as _, ButtonStyle, Clickable as _, Color, ContextMenu, DropdownMenu,
    Label, LabelCommon as _, LabelSize, Switch, ToggleState, h_flex, v_flex,
};

use crate::settings_model::{
    SettingKind, SettingRow, SettingSpec, SettingSpecJson as _, SettingValue, SettingsEdit,
    SettingsForm, field_text, settings_store,
};

/// What a one-line field shows for a row: the user's value as the file holds it (an invalid one
/// too, so the user sees what is wrong), else the effective value.
pub(crate) fn row_text(row: &SettingRow) -> String {
    match &row.user {
        Some(Value::String(text)) => text.clone(),
        Some(value) => value.to_string(),
        None => field_text(&row.effective),
    }
}

/// The items a `text-list` row's fields show.
pub(crate) fn row_items(row: &SettingRow) -> Vec<String> {
    match (&row.user, &row.effective) {
        (Some(Value::Array(items)), _) => items
            .iter()
            .map(|item| match item {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            })
            .collect(),
        (_, SettingValue::TextList(items)) => items.clone(),
        _ => Vec::new(),
    }
}

/// The value a field's text writes, or why it writes nothing.
pub(crate) fn checked_field(spec: &SettingSpec, text: &str) -> Result<Value, String> {
    let value = spec.json_from_field(text)?;
    spec.value_from_json(&value)?;
    Ok(value)
}

/// The value a `text-list`'s item texts write, or why they write nothing.
pub(crate) fn checked_items(spec: &SettingSpec, items: Vec<String>) -> Result<Value, String> {
    let value = Value::from(items);
    spec.value_from_json(&value)?;
    Ok(value)
}

pub struct ExtensionSettingsForm {
    extension_id: String,
    form: SettingsForm,
    /// `integer`, `number` and `text` fields, by key.
    fields: HashMap<String, Entity<Editor>>,
    /// A `text-list`'s item fields, by key.
    lists: HashMap<String, Vec<Entity<Editor>>>,
    /// The error of a field whose text was not written.
    field_errors: HashMap<String, String>,
    /// The key `settings.open` named, drawn with an accent until the next open.
    highlighted: Option<String>,
    focus_handle: FocusHandle,
    _settings: Subscription,
    /// The fields' edit subscriptions, by key; replaced with the key's fields.
    editor_subscriptions: HashMap<String, Vec<Subscription>>,
}

impl ExtensionSettingsForm {
    pub fn new(extension_id: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx
            .observe_global_in::<settings::SettingsStore>(window, |form, window, cx| {
                form.reload(window, cx)
            });
        let mut form = Self {
            extension_id,
            form: SettingsForm::default(),
            fields: HashMap::default(),
            lists: HashMap::default(),
            field_errors: HashMap::default(),
            highlighted: None,
            focus_handle: cx.focus_handle(),
            _settings: settings,
            editor_subscriptions: HashMap::default(),
        };
        form.reload(window, cx);
        form
    }

    pub fn form(&self) -> &SettingsForm {
        &self.form
    }

    /// The one-line field of an `integer`, `number` or `text` setting.
    pub fn field(&self, key: &str) -> Option<Entity<Editor>> {
        self.fields.get(key).cloned()
    }

    /// The error of a field whose text was not written.
    pub fn field_error(&self, key: &str) -> Option<String> {
        self.field_errors.get(key).cloned()
    }

    /// Writes `edit` and re-reads the form, as the form's buttons do.
    pub fn apply(&mut self, edit: SettingsEdit, window: &mut Window, cx: &mut Context<Self>) {
        self.write(edit, window, cx);
    }

    /// Whether the extension declares any setting.
    pub fn has_settings(&self) -> bool {
        !self.form.rows.is_empty()
    }

    /// Re-reads the declarations and the user's values, and brings every field the user is not
    /// typing in up to date.
    pub fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let store = settings_store(cx);
        let specs = store.specs(&self.extension_id, cx);
        let user = store.user_values(&self.extension_id, cx);
        self.form = SettingsForm::new(&specs, &user);
        let rows = self.form.rows.clone();
        self.fields
            .retain(|key, _| rows.iter().any(|row| &row.spec.key == key));
        self.lists
            .retain(|key, _| rows.iter().any(|row| &row.spec.key == key));
        self.editor_subscriptions
            .retain(|key, _| rows.iter().any(|row| &row.spec.key == key));
        for row in &rows {
            let key = row.spec.key.clone();
            match row.spec.kind {
                SettingKind::Integer { .. }
                | SettingKind::Number { .. }
                | SettingKind::Text { .. } => {
                    let text = row_text(row);
                    if let Some(editor) = self.fields.get(&key).cloned() {
                        if !editor.focus_handle(cx).is_focused(window) {
                            self.field_errors.remove(&key);
                            set_text_if_changed(&editor, &text, window, cx);
                        }
                    } else {
                        let editor = self.new_field(&text, key.clone(), None, window, cx);
                        self.fields.insert(key, editor);
                    }
                }
                SettingKind::TextList { .. } => {
                    let items = row_items(row);
                    let editors = self.lists.get(&key).cloned().unwrap_or_default();
                    let typing = editors
                        .iter()
                        .any(|editor| editor.focus_handle(cx).is_focused(window));
                    if editors.len() != items.len() {
                        self.field_errors.remove(&key);
                        self.editor_subscriptions.remove(&key);
                        let editors = items
                            .iter()
                            .enumerate()
                            .map(|(index, item)| {
                                self.new_field(item, key.clone(), Some(index), window, cx)
                            })
                            .collect();
                        self.lists.insert(key, editors);
                    } else if !typing {
                        self.field_errors.remove(&key);
                        for (editor, item) in editors.iter().zip(&items) {
                            set_text_if_changed(editor, item, window, cx);
                        }
                    }
                }
                SettingKind::Boolean | SettingKind::Choice { .. } => {}
            }
        }
        cx.notify();
    }

    fn new_field(
        &mut self,
        text: &str,
        key: String,
        item: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<Editor> {
        let editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(text, window, cx);
            editor
        });
        let field_key = key.clone();
        let subscription = cx.subscribe_in(
            &editor,
            window,
            move |form, _, event: &EditorEvent, window, cx| {
                if matches!(event, EditorEvent::BufferEdited) {
                    form.field_edited(&field_key, item.is_some(), window, cx);
                }
            },
        );
        let subscriptions = self.editor_subscriptions.entry(key).or_default();
        if item.is_none_or(|index| index == 0) {
            subscriptions.clear();
        }
        subscriptions.push(subscription);
        editor
    }

    fn field_edited(
        &mut self,
        key: &str,
        is_list: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self.form.row(key).cloned() else {
            return;
        };
        // The form's own `set_text` (a reload) also arrives here: text that shows what the file
        // already holds writes nothing, so a reset is not undone by writing the default back.
        let checked = if is_list {
            let items: Vec<String> = self
                .lists
                .get(key)
                .map(|editors| {
                    editors
                        .iter()
                        .map(|editor| editor.read(cx).text(cx))
                        .collect()
                })
                .unwrap_or_default();
            if items == row_items(&row) {
                self.field_errors.remove(key);
                cx.notify();
                return;
            }
            checked_items(&row.spec, items)
        } else {
            let Some(editor) = self.fields.get(key) else {
                return;
            };
            let text = editor.read(cx).text(cx);
            if text == row_text(&row) {
                self.field_errors.remove(key);
                cx.notify();
                return;
            }
            checked_field(&row.spec, &text)
        };
        match checked {
            Ok(value) => {
                self.field_errors.remove(key);
                if row.user.as_ref() != Some(&value) {
                    self.write(
                        SettingsEdit::Set {
                            key: key.to_owned(),
                            value,
                        },
                        window,
                        cx,
                    );
                }
            }
            Err(error) => {
                self.field_errors.insert(key.to_owned(), error);
            }
        }
        cx.notify();
    }

    fn write(&mut self, edit: SettingsEdit, window: &mut Window, cx: &mut Context<Self>) {
        settings_store(cx).write(&self.extension_id, edit, cx);
        self.reload(window, cx);
    }

    /// `settings.open`: marks `key` and puts the keyboard in its field, if it has one.
    pub fn highlight(&mut self, key: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = &key {
            let editor = self.fields.get(key).cloned().or_else(|| {
                self.lists
                    .get(key)
                    .and_then(|editors| editors.first().cloned())
            });
            if let Some(editor) = editor {
                editor.focus_handle(cx).focus(window, cx);
            }
        }
        self.highlighted = key;
        cx.notify();
    }

    fn render_control(
        &self,
        row: &SettingRow,
        this: WeakEntity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let id = &self.extension_id;
        let key = row.spec.key.clone();
        match &row.spec.kind {
            SettingKind::Boolean => {
                let on = matches!(row.effective, SettingValue::Boolean(true));
                Switch::new(
                    SharedString::from(format!("fyx-setting-{id}-{key}")),
                    if on {
                        ToggleState::Selected
                    } else {
                        ToggleState::Unselected
                    },
                )
                .on_click(move |_, window, cx| {
                    let value = Value::from(!on);
                    let key = key.clone();
                    this.update(cx, |form, cx| {
                        form.write(SettingsEdit::Set { key, value }, window, cx)
                    })
                    .ok();
                })
                .into_any_element()
            }
            SettingKind::Choice { choices } => {
                let current = match &row.effective {
                    SettingValue::Choice(value) => value.clone(),
                    _ => String::new(),
                };
                let label = choices
                    .iter()
                    .find(|choice| choice.value == current)
                    .map(|choice| choice.title.clone())
                    .unwrap_or_default();
                let choices = choices.clone();
                let menu = ContextMenu::build(window, cx, move |mut menu, _, _| {
                    for choice in choices {
                        let (this, key, value) = (this.clone(), key.clone(), choice.value.clone());
                        menu = menu.toggleable_entry(
                            choice.title,
                            choice.value == current,
                            ui::IconPosition::Start,
                            None,
                            move |window, cx| {
                                let edit = SettingsEdit::Set {
                                    key: key.clone(),
                                    value: Value::from(value.clone()),
                                };
                                this.update(cx, |form, cx| form.write(edit, window, cx))
                                    .ok();
                            },
                        );
                    }
                    menu
                });
                DropdownMenu::new(
                    SharedString::from(format!("fyx-setting-{id}-{}", row.spec.key)),
                    label,
                    menu,
                )
                .into_any_element()
            }
            SettingKind::TextList { max_items, .. } => {
                let editors = self.lists.get(&key).cloned().unwrap_or_default();
                let items = row_items(row);
                let can_add = items.len() < *max_items;
                v_flex()
                    .gap_1()
                    .children(editors.into_iter().enumerate().map(|(index, editor)| {
                        let (this, key) = (this.clone(), key.clone());
                        let mut rest = items.clone();
                        if index < rest.len() {
                            rest.remove(index);
                        }
                        let rest = Value::from(rest);
                        h_flex().gap_1().child(field_box(editor, cx)).child(
                            Button::new(
                                SharedString::from(format!("fyx-setting-{id}-{key}-rm-{index}")),
                                "Remove",
                            )
                            .style(ButtonStyle::Subtle)
                            .label_size(LabelSize::Small)
                            .on_click(move |_, window, cx| {
                                let edit = SettingsEdit::Set {
                                    key: key.clone(),
                                    value: rest.clone(),
                                };
                                this.update(cx, |form, cx| form.write(edit, window, cx))
                                    .ok();
                            }),
                        )
                    }))
                    .when(can_add, |this_element| {
                        let (this, key) = (this.clone(), key.clone());
                        let mut more = items.clone();
                        more.push(String::new());
                        let more = Value::from(more);
                        this_element.child(
                            Button::new(
                                SharedString::from(format!("fyx-setting-{id}-{key}-add")),
                                "Add",
                            )
                            .style(ButtonStyle::Subtle)
                            .label_size(LabelSize::Small)
                            .on_click(move |_, window, cx| {
                                let edit = SettingsEdit::Set {
                                    key: key.clone(),
                                    value: more.clone(),
                                };
                                this.update(cx, |form, cx| form.write(edit, window, cx))
                                    .ok();
                            }),
                        )
                    })
                    .into_any_element()
            }
            SettingKind::Integer { .. } | SettingKind::Number { .. } | SettingKind::Text { .. } => {
                match self.fields.get(&key).cloned() {
                    Some(editor) => field_box(editor, cx).into_any_element(),
                    None => gpui::Empty.into_any_element(),
                }
            }
        }
    }
}

fn set_text_if_changed(editor: &Entity<Editor>, text: &str, window: &mut Window, cx: &mut App) {
    if editor.read(cx).text(cx) != text {
        editor.update(cx, |editor, cx| editor.set_text(text, window, cx));
    }
}

fn field_box(editor: Entity<Editor>, cx: &App) -> impl IntoElement {
    use theme::ActiveTheme as _;
    h_flex()
        .w_64()
        .px_2()
        .py_1()
        .border_1()
        .rounded_sm()
        .border_color(cx.theme().colors().border_variant)
        .child(editor)
}

/// "min to max" under a number field.
fn bounds_note(kind: &SettingKind) -> Option<String> {
    let (min, max) = match kind {
        SettingKind::Integer { min, max } => (
            min.map(|min| min.to_string()),
            max.map(|max| max.to_string()),
        ),
        SettingKind::Number { min, max } => (
            min.map(|min| min.to_string()),
            max.map(|max| max.to_string()),
        ),
        _ => return None,
    };
    match (min, max) {
        (Some(min), Some(max)) => Some(format!("{min} to {max}")),
        (Some(min), None) => Some(format!("{min} or more")),
        (None, Some(max)) => Some(format!("{max} or less")),
        (None, None) => None,
    }
}

impl gpui::Focusable for ExtensionSettingsForm {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ExtensionSettingsForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let id = self.extension_id.clone();
        let this = cx.entity().downgrade();
        let rows = self.form.rows.clone();
        let unknown = self.form.unknown.clone();
        let has_user_values = self.form.has_user_values();
        let reset = |label: &'static str, element_id: String, edit: SettingsEdit| {
            let this = this.clone();
            Button::new(SharedString::from(element_id), label)
                .style(ButtonStyle::Subtle)
                .label_size(LabelSize::Small)
                .on_click(move |_, window, cx| {
                    let edit = edit.clone();
                    this.update(cx, |form, cx| form.write(edit, window, cx))
                        .ok();
                })
        };
        let mut controls = Vec::new();
        for row in &rows {
            let key = row.spec.key.clone();
            let error = self
                .field_errors
                .get(&key)
                .cloned()
                .or_else(|| row.error.clone());
            let highlighted = self.highlighted.as_deref() == Some(key.as_str());
            let control = self.render_control(row, this.clone(), window, cx);
            controls.push(
                v_flex()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Label::new(row.spec.title.clone()).color(if highlighted {
                                Color::Accent
                            } else {
                                Color::Default
                            }))
                            .when(row.can_reset(), |this_element| {
                                this_element.child(reset(
                                    "Reset to default",
                                    format!("fyx-setting-{id}-{key}-reset"),
                                    SettingsEdit::Reset { key: key.clone() },
                                ))
                            }),
                    )
                    .when_some(row.spec.description.clone(), |this_element, description| {
                        this_element.child(
                            Label::new(description)
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                    })
                    .child(control)
                    .when_some(bounds_note(&row.spec.kind), |this_element, note| {
                        this_element
                            .child(Label::new(note).size(LabelSize::XSmall).color(Color::Muted))
                    })
                    .when_some(error, |this_element, error| {
                        this_element
                            .child(Label::new(error).size(LabelSize::Small).color(Color::Error))
                    })
                    .into_any_element(),
            );
        }
        for (key, value) in unknown {
            controls.push(
                h_flex()
                    .gap_2()
                    .child(
                        Label::new(format!("{key}: {value}, not used by this version"))
                            .size(LabelSize::Small)
                            .color(Color::Disabled),
                    )
                    .child(reset(
                        "Reset",
                        format!("fyx-setting-{id}-{key}-reset-unknown"),
                        SettingsEdit::Reset { key },
                    ))
                    .into_any_element(),
            );
        }
        v_flex()
            .gap_2()
            .pl_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Label::new("Settings")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .when(has_user_values, |this_element| {
                        this_element.child(reset(
                            "Reset All",
                            format!("fyx-setting-{id}-reset-all"),
                            SettingsEdit::ResetAll,
                        ))
                    }),
            )
            .children(controls)
    }
}

/// A `settings.open` waiting for the Fyx Extensions page to draw.
#[derive(Default)]
pub(crate) struct PendingSettingsOpen(pub Option<(String, Option<String>)>);

impl Global for PendingSettingsOpen {}

/// `settings.open` (§D.4): opens the Fyx Extensions page in the active window with
/// `extension_id`'s Settings section shown, `key` marked. The runtime checks first that the call
/// came from an event of one of the extension's surfaces.
pub fn open_extension_settings(extension_id: &str, key: Option<&str>, cx: &mut App) {
    cx.set_global(PendingSettingsOpen(Some((
        extension_id.to_owned(),
        key.map(str::to_owned),
    ))));
    if let Some(window) = cx.active_window() {
        window
            .update(cx, |_, window, cx| {
                window.dispatch_action(Box::new(crate::OpenFyxExtensions), cx)
            })
            .ok();
    }
}

pub(crate) fn take_pending_open(cx: &mut App) -> Option<(String, Option<String>)> {
    cx.try_global::<PendingSettingsOpen>()?;
    cx.global_mut::<PendingSettingsOpen>().0.take()
}
