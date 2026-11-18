//! Fyx demo: a sign-up form with validation.
//!
//! Three inputs (name, email, age) are validated as the user types. A field shows its error, in
//! the `error` colour token, once the user has edited it; the error text node is inserted under
//! the field and removed again when the value becomes valid. **Submit** stays disabled until
//! every field is valid. Enter in a field submits once the form is valid.
//!
//! Host tests and the docs rely on this behaviour, so keep it stable:
//!
//! - One surface, id `"form"`: a right dock panel titled "Sign-up form", icon `user_check`,
//!   default size 320.
//! - The first `mount("form")` returns 15 nodes, parents first:
//!
//!   | id | parent | props |
//!   |---|---|---|
//!   | 1 | none | column flex, gap 12, padding 8, grow |
//!   | 2 | 1 | text `Create an account`, bold |
//!   | 3, 6, 9 | 1 | column flex, gap 4: the name, email and age fields |
//!   | 4, 7, 10 | 3, 6, 9 | text `Name`, `Email`, `Age` |
//!   | 5, 8, 11 | 3, 6, 9 | input, empty, placeholders `Ada Lovelace`, `ada@example.com`, `36` |
//!   | 12 | 1 | row flex, gap 8, align center |
//!   | 13 | 12 | button `Submit`, icon `check`, filled, disabled until the form is valid |
//!   | 14 | 12 | button `Reset`, subtle, disabled while the form is untouched |
//!   | 15 | 1 | text `Fill in every field to enable Submit.`, `text-muted` |
//!
//! - Every event that changes something answers with patches, never `replace`: `set-props` for
//!   the buttons, the status line (15) and error texts, `insert` of an error text as the last
//!   child of a field's column, `remove` of it. Typed values are never echoed back. A submit or
//!   reset sets the inputs to empty. Events that change nothing answer `unchanged`. An unknown
//!   surface id is an error.
//! - After a submit, 15 reads `Submitted: <name> <<email>>, age <age>` in `success` until the next
//!   edit.

use ui_ext_sdk::*;

const SURFACE_ID: &str = "form";
const MAX_NAME_CHARACTERS: usize = 40;
const AGE_RANGE: std::ops::RangeInclusive<u32> = 13..=120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Email,
    Age,
}

impl Field {
    const ALL: [Self; 3] = [Self::Name, Self::Email, Self::Age];

    fn label(self) -> &'static str {
        match self {
            Self::Name => "Name",
            Self::Email => "Email",
            Self::Age => "Age",
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Self::Name => "Ada Lovelace",
            Self::Email => "ada@example.com",
            Self::Age => "36",
        }
    }

    fn input(self, value: &str) -> InputProps {
        InputProps::new(value).placeholder(self.placeholder())
    }

    fn validate(self, value: &str) -> Result<(), &'static str> {
        match self {
            Self::Name => validate_name(value),
            Self::Email => validate_email(value),
            Self::Age => validate_age(value).map(|_| ()),
        }
    }
}

fn validate_name(value: &str) -> Result<(), &'static str> {
    let name = value.trim();
    if name.is_empty() {
        Err("Enter your name.")
    } else if name.chars().count() > MAX_NAME_CHARACTERS {
        Err("Use at most 40 characters.")
    } else {
        Ok(())
    }
}

fn validate_email(value: &str) -> Result<(), &'static str> {
    let email = value.trim();
    if email.is_empty() {
        return Err("Enter your email address.");
    }
    let is_valid = match email.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && !domain.contains('@')
                && !email.chars().any(char::is_whitespace)
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !domain.contains("..")
        }
        None => false,
    };
    if is_valid {
        Ok(())
    } else {
        Err("Enter an address like name@example.com.")
    }
}

fn validate_age(value: &str) -> Result<u32, &'static str> {
    let age = value.trim();
    if age.is_empty() {
        return Err("Enter your age.");
    }
    let age = age.parse::<u32>().map_err(|_| "Enter a whole number.")?;
    if AGE_RANGE.contains(&age) {
        Ok(age)
    } else {
        Err("Enter an age from 13 to 120.")
    }
}

#[derive(Default)]
struct FieldState {
    value: String,
    /// Errors are shown only for fields the user has edited, so an empty form is not all red.
    touched: bool,
}

#[derive(Clone, Copy)]
struct FieldNodes {
    column: NodeId,
    input: NodeId,
    error: Option<NodeId>,
}

struct FormNodes {
    fields: [FieldNodes; 3],
    submit: NodeId,
    reset: NodeId,
    status: NodeId,
}

struct FormPanel {
    fields: [FieldState; 3],
    submitted: Option<String>,
    tree: SurfaceTree,
    nodes: Option<FormNodes>,
}

impl FormPanel {
    fn field(&self, field: Field) -> &FieldState {
        match field {
            Field::Name => &self.fields[0],
            Field::Email => &self.fields[1],
            Field::Age => &self.fields[2],
        }
    }

    fn field_mut(&mut self, field: Field) -> &mut FieldState {
        match field {
            Field::Name => &mut self.fields[0],
            Field::Email => &mut self.fields[1],
            Field::Age => &mut self.fields[2],
        }
    }

    fn is_valid(&self) -> bool {
        Field::ALL
            .into_iter()
            .all(|field| field.validate(&self.field(field).value).is_ok())
    }

    fn is_pristine(&self) -> bool {
        self.submitted.is_none()
            && self
                .fields
                .iter()
                .all(|field| field.value.is_empty() && !field.touched)
    }

    fn error(&self, field: Field) -> Option<TextProps> {
        let state = self.field(field);
        if !state.touched {
            return None;
        }
        let message = field.validate(&state.value).err()?;
        Some(TextProps::new(message).color(ColorToken::Error))
    }

    fn submit_button(&self) -> ButtonProps {
        ButtonProps::new("Submit")
            .icon(Icon::Check)
            .style(ButtonStyle::Filled)
            .disabled(!self.is_valid())
    }

    fn reset_button(&self) -> ButtonProps {
        ButtonProps::new("Reset")
            .style(ButtonStyle::Subtle)
            .disabled(self.is_pristine())
    }

    fn status(&self) -> TextProps {
        match &self.submitted {
            Some(message) => TextProps::new(message.as_str()).color(ColorToken::Success),
            None if self.is_valid() => {
                TextProps::new("Ready to submit.").color(ColorToken::TextMuted)
            }
            None => {
                TextProps::new("Fill in every field to enable Submit.").color(ColorToken::TextMuted)
            }
        }
    }

    fn field_for_input(&self, id: NodeId) -> Option<Field> {
        let nodes = self.nodes.as_ref()?;
        Field::ALL
            .into_iter()
            .zip(nodes.fields)
            .find(|(_, field_nodes)| field_nodes.input == id)
            .map(|(field, _)| field)
    }

    fn sync(&mut self) -> Result<Update, String> {
        let errors = Field::ALL.map(|field| self.error(field));
        let (submit, reset, status) = (self.submit_button(), self.reset_button(), self.status());
        let Some(nodes) = &mut self.nodes else {
            return Err(TreeError::NotMounted.into());
        };
        for (field_nodes, error) in nodes.fields.iter_mut().zip(errors) {
            match (field_nodes.error, error) {
                (Some(node), Some(error)) => {
                    self.tree.set_props(node, error)?;
                }
                (None, Some(error)) => {
                    let inserted = self.tree.insert(field_nodes.column, None, |tree| {
                        tree.text(error);
                    })?;
                    field_nodes.error = inserted.first().copied();
                }
                (Some(node), None) => {
                    self.tree.remove(node)?;
                    field_nodes.error = None;
                }
                (None, None) => {}
            }
        }
        self.tree.set_props(nodes.submit, submit)?;
        self.tree.set_props(nodes.reset, reset)?;
        self.tree.set_props(nodes.status, status)?;
        Ok(self.tree.take_update())
    }

    fn record_input(&mut self, field: Field, input: NodeId, text: String) -> Result<(), String> {
        // The user owns the text while typing: record it without echoing it back.
        self.tree.set_props_from_host(input, field.input(&text))?;
        let state = self.field_mut(field);
        state.value = text;
        state.touched = true;
        self.submitted = None;
        Ok(())
    }

    /// Empties every input. The mirror already holds what the host shows, so the `set-props` to
    /// an empty value is a real change and is sent.
    fn clear_inputs(&mut self) -> Result<(), String> {
        let Some(nodes) = &self.nodes else {
            return Err(TreeError::NotMounted.into());
        };
        let inputs = nodes.fields.map(|field_nodes| field_nodes.input);
        for (field, input) in Field::ALL.into_iter().zip(inputs) {
            self.tree.set_props(input, field.input(""))?;
            *self.field_mut(field) = FieldState::default();
        }
        Ok(())
    }

    fn submit(&mut self) -> Result<Update, String> {
        if !self.is_valid() {
            for field in &mut self.fields {
                field.touched = true;
            }
            return self.sync();
        }
        let age = validate_age(&self.field(Field::Age).value)?;
        self.submitted = Some(format!(
            "Submitted: {} <{}>, age {age}",
            self.field(Field::Name).value.trim(),
            self.field(Field::Email).value.trim()
        ));
        self.clear_inputs()?;
        self.sync()
    }

    fn reset(&mut self) -> Result<Update, String> {
        if self.is_pristine() {
            return Ok(Update::Unchanged);
        }
        self.clear_inputs()?;
        self.submitted = None;
        self.sync()
    }
}

fn check_surface(surface_id: &str) -> Result<(), String> {
    if surface_id == SURFACE_ID {
        Ok(())
    } else {
        Err(format!("unknown surface {surface_id:?}"))
    }
}

impl Extension for FormPanel {
    fn new() -> Self {
        Self {
            fields: Default::default(),
            submitted: None,
            tree: SurfaceTree::new(),
            nodes: None,
        }
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            SURFACE_ID,
            "Sign-up form",
            DockPosition::Right,
            Icon::Person,
            320,
        )]
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        check_surface(surface_id)?;
        let inputs = Field::ALL.map(|field| field.input(&self.field(field).value));
        let errors = Field::ALL.map(|field| self.error(field));
        let (submit, reset, status) = (self.submit_button(), self.reset_button(), self.status());
        let mut field_nodes = [FieldNodes {
            column: 0,
            input: 0,
            error: None,
        }; 3];
        let mut submit_node = 0;
        let mut reset_node = 0;
        let mut status_node = 0;
        let mounted = self.tree.mount(|tree| {
            tree.flex(FlexProps::column().gap(12).padding(8).grow(), |tree| {
                tree.text(TextProps::new("Create an account").bold());
                for (((field, input), error), nodes) in Field::ALL
                    .into_iter()
                    .zip(inputs)
                    .zip(errors)
                    .zip(field_nodes.iter_mut())
                {
                    nodes.column = tree.flex(FlexProps::column().gap(4), |tree| {
                        tree.text(field.label());
                        nodes.input = tree.input(input);
                        nodes.error = error.map(|error| tree.text(error));
                    });
                }
                tree.flex(FlexProps::row().gap(8).align(Align::Center), |tree| {
                    submit_node = tree.button(submit);
                    reset_node = tree.button(reset);
                });
                status_node = tree.text(status);
            });
        })?;
        self.nodes = Some(FormNodes {
            fields: field_nodes,
            submit: submit_node,
            reset: reset_node,
            status: status_node,
        });
        Ok(mounted)
    }

    fn handle_event(&mut self, surface_id: &str, event: UiEvent) -> Result<Update, String> {
        check_surface(surface_id)?;
        let Some(nodes) = &self.nodes else {
            return Err(TreeError::NotMounted.into());
        };
        let (submit, reset) = (nodes.submit, nodes.reset);
        match event {
            UiEvent::InputChanged((id, text)) => {
                let Some(field) = self.field_for_input(id) else {
                    return Ok(Update::Unchanged);
                };
                if text == self.field(field).value {
                    return Ok(Update::Unchanged);
                }
                self.record_input(field, id, text)?;
                self.sync()
            }
            UiEvent::InputSubmitted((id, text)) => {
                let Some(field) = self.field_for_input(id) else {
                    return Ok(Update::Unchanged);
                };
                self.record_input(field, id, text)?;
                if self.is_valid() {
                    self.submit()
                } else {
                    self.sync()
                }
            }
            UiEvent::Click(id) if id == submit => self.submit(),
            UiEvent::Click(id) if id == reset => self.reset(),
            UiEvent::Click(_)
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
register_extension!(FormPanel, v0_1);
#[cfg(feature = "fyx-ui-0-2")]
register_extension!(FormPanel);

#[cfg(test)]
mod tests {
    use super::*;

    const NAME: NodeId = 5;
    const EMAIL: NodeId = 8;
    const AGE: NodeId = 11;
    const SUBMIT: NodeId = 13;
    const RESET: NodeId = 14;
    const STATUS: NodeId = 15;

    fn mounted() -> FormPanel {
        let mut panel = FormPanel::new();
        panel.mount("form").expect("mount");
        panel
    }

    fn expect_patches(update: Update) -> Vec<Patch> {
        let Update::Patches(patches) = update else {
            panic!("expected patches, got {update:?}");
        };
        patches
    }

    fn type_into(panel: &mut FormPanel, input: NodeId, text: &str) -> Update {
        panel
            .handle_event("form", UiEvent::InputChanged((input, text.into())))
            .expect("type")
    }

    fn type_and_expect_patches(panel: &mut FormPanel, input: NodeId, text: &str) -> Vec<Patch> {
        expect_patches(type_into(panel, input, text))
    }

    fn set_props_of(patches: &[Patch], id: NodeId) -> Option<&Props> {
        patches.iter().find_map(|patch| match patch {
            Patch::SetProps((patched, props)) if *patched == id => Some(props),
            _ => None,
        })
    }

    #[test]
    fn validation_rules() {
        assert_eq!(validate_name("  "), Err("Enter your name."));
        assert_eq!(validate_name(" Ada "), Ok(()));
        assert_eq!(
            validate_name(&"x".repeat(41)),
            Err("Use at most 40 characters.")
        );
        for email in ["ada@example.com", " ada.l@mail.example.org "] {
            assert_eq!(validate_email(email), Ok(()), "{email}");
        }
        for email in [
            "ada",
            "@example.com",
            "ada@example",
            "ada@.com",
            "ada@example.",
            "ada@exa..mple",
            "a@b@example.com",
            "ada lovelace@example.com",
        ] {
            assert!(validate_email(email).is_err(), "{email}");
        }
        assert_eq!(validate_age("36"), Ok(36));
        assert_eq!(validate_age("abc"), Err("Enter a whole number."));
        assert_eq!(validate_age("-1"), Err("Enter a whole number."));
        assert_eq!(validate_age("12"), Err("Enter an age from 13 to 120."));
        assert_eq!(validate_age("121"), Err("Enter an age from 13 to 120."));
    }

    #[test]
    fn first_mount_is_pinned() {
        let mut panel = FormPanel::new();
        assert_eq!(panel.surfaces()[0].id, "form");
        let nodes = panel.mount("form").expect("mount");
        let shape = nodes
            .iter()
            .map(|node| (node.id, node.parent))
            .collect::<Vec<_>>();
        assert_eq!(
            shape,
            [
                (1, None),
                (2, Some(1)),
                (3, Some(1)),
                (4, Some(3)),
                (5, Some(3)),
                (6, Some(1)),
                (7, Some(6)),
                (8, Some(6)),
                (9, Some(1)),
                (10, Some(9)),
                (11, Some(9)),
                (12, Some(1)),
                (13, Some(12)),
                (14, Some(12)),
                (15, Some(1)),
            ]
        );
        assert_eq!(nodes[7].props, Field::Email.input("").into());
        assert_eq!(
            nodes[12].props,
            ButtonProps::new("Submit")
                .icon(Icon::Check)
                .style(ButtonStyle::Filled)
                .disabled(true)
                .into()
        );
        assert_eq!(
            nodes[14].props,
            TextProps::new("Fill in every field to enable Submit.")
                .color(ColorToken::TextMuted)
                .into()
        );
    }

    #[test]
    fn errors_are_inserted_and_removed() {
        let mut panel = mounted();

        let patches = type_and_expect_patches(&mut panel, EMAIL, "ada@");
        let error =
            TextProps::new("Enter an address like name@example.com.").color(ColorToken::Error);
        assert!(patches.contains(&Patch::Insert(InsertOp {
            node: Node {
                id: 16,
                parent: Some(6),
                props: error.into(),
            },
            before: None,
        })));
        assert!(set_props_of(&patches, EMAIL).is_none());
        assert_eq!(
            set_props_of(&patches, RESET),
            Some(&ButtonProps::new("Reset").style(ButtonStyle::Subtle).into())
        );
        assert!(set_props_of(&patches, SUBMIT).is_none());

        let patches = type_and_expect_patches(&mut panel, EMAIL, "ada@example.com");
        assert_eq!(patches, [Patch::Remove(16)]);
        assert_eq!(
            panel
                .handle_event(
                    "form",
                    UiEvent::InputChanged((EMAIL, "ada@example.com".into()))
                )
                .expect("same text"),
            Update::Unchanged
        );

        let patches = type_and_expect_patches(&mut panel, AGE, "7");
        assert_eq!(
            patches,
            [Patch::Insert(InsertOp {
                node: Node {
                    id: 17,
                    parent: Some(9),
                    props: TextProps::new("Enter an age from 13 to 120.")
                        .color(ColorToken::Error)
                        .into(),
                },
                before: None,
            })]
        );
        let patches = type_and_expect_patches(&mut panel, AGE, "x");
        assert_eq!(
            patches,
            [Patch::SetProps((
                17,
                TextProps::new("Enter a whole number.")
                    .color(ColorToken::Error)
                    .into()
            ))]
        );
    }

    #[test]
    fn submit_is_enabled_only_when_valid() {
        let mut panel = mounted();
        type_into(&mut panel, NAME, "Ada");
        assert_eq!(
            type_into(&mut panel, EMAIL, "ada@example.com"),
            Update::Unchanged
        );
        assert!(panel.handle_event("form", UiEvent::Click(SUBMIT)).is_ok());
        assert_eq!(panel.field(Field::Name).value, "Ada");

        let patches = type_and_expect_patches(&mut panel, AGE, "36");
        assert_eq!(
            set_props_of(&patches, SUBMIT),
            Some(
                &ButtonProps::new("Submit")
                    .icon(Icon::Check)
                    .style(ButtonStyle::Filled)
                    .into()
            )
        );

        let patches = expect_patches(
            panel
                .handle_event("form", UiEvent::Click(SUBMIT))
                .expect("submit"),
        );
        for (field, input) in Field::ALL.into_iter().zip([NAME, EMAIL, AGE]) {
            assert_eq!(set_props_of(&patches, input), Some(&field.input("").into()));
        }
        assert_eq!(
            set_props_of(&patches, STATUS),
            Some(
                &TextProps::new("Submitted: Ada <ada@example.com>, age 36")
                    .color(ColorToken::Success)
                    .into()
            )
        );
        assert!(matches!(
            set_props_of(&patches, SUBMIT),
            Some(Props::Button(button)) if button.disabled
        ));
        assert!(
            !patches
                .iter()
                .any(|patch| matches!(patch, Patch::Insert(_)))
        );

        let patches = type_and_expect_patches(&mut panel, NAME, "B");
        assert!(matches!(
            set_props_of(&patches, STATUS),
            Some(Props::Text(text)) if text.content == "Fill in every field to enable Submit."
        ));
    }

    #[test]
    fn enter_submits_a_valid_form_and_reset_clears() {
        let mut panel = mounted();
        type_into(&mut panel, NAME, "Ada");
        type_into(&mut panel, EMAIL, "ada@example.com");
        let patches = expect_patches(
            panel
                .handle_event("form", UiEvent::InputSubmitted((AGE, "36".into())))
                .expect("enter"),
        );
        assert!(matches!(
            set_props_of(&patches, STATUS),
            Some(Props::Text(text)) if text.content.starts_with("Submitted: Ada")
        ));

        let patches = expect_patches(
            panel
                .handle_event("form", UiEvent::InputSubmitted((NAME, "".into())))
                .expect("enter on an invalid form"),
        );
        assert!(patches.iter().any(|patch| matches!(
            patch,
            Patch::Insert(InsertOp {
                node: Node {
                    parent: Some(3),
                    ..
                },
                ..
            })
        )));

        let patches = expect_patches(
            panel
                .handle_event("form", UiEvent::Click(RESET))
                .expect("reset"),
        );
        assert!(patches.contains(&Patch::Remove(16)));
        assert!(panel.is_pristine());
        assert_eq!(
            panel
                .handle_event("form", UiEvent::Click(RESET))
                .expect("reset again"),
            Update::Unchanged
        );
        assert!(panel.handle_event("other", UiEvent::Click(RESET)).is_err());
    }

    #[test]
    fn errors_survive_a_remount() {
        let mut panel = mounted();
        type_into(&mut panel, AGE, "200");
        let nodes = panel.mount("form").expect("remount");
        let error = nodes
            .iter()
            .find(|node| {
                node.props
                    == TextProps::new("Enter an age from 13 to 120.")
                        .color(ColorToken::Error)
                        .into()
            })
            .expect("error node");
        let age_column = panel.nodes.as_ref().expect("mounted").fields[2].column;
        assert_eq!(error.parent, Some(age_column));

        let age_input = panel.nodes.as_ref().expect("mounted").fields[2].input;
        let patches = type_and_expect_patches(&mut panel, age_input, "40");
        assert_eq!(patches, [Patch::Remove(error.id)]);
    }

    /// Every update, applied to the host's own tree model, must be accepted and leave it equal to
    /// the extension's mirror.
    fn assert_host_accepts(panel: &mut FormPanel, events: Vec<UiEvent>) {
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
        let mut panel = FormPanel::new();
        assert_host_accepts(
            &mut panel,
            vec![
                UiEvent::Click(SUBMIT),
                UiEvent::InputChanged((NAME, "Ada".into())),
                UiEvent::InputChanged((EMAIL, "ada@".into())),
                UiEvent::InputChanged((EMAIL, "ada@example.com".into())),
                UiEvent::InputChanged((AGE, "x".into())),
                UiEvent::InputChanged((AGE, "36".into())),
                UiEvent::Click(SUBMIT),
                UiEvent::InputChanged((NAME, "Grace".into())),
                UiEvent::InputSubmitted((NAME, "Grace".into())),
                UiEvent::Click(RESET),
                UiEvent::InputChanged((NAME, "Ada".into())),
                UiEvent::InputChanged((EMAIL, "ada@example.com".into())),
                UiEvent::InputSubmitted((AGE, "36".into())),
                UiEvent::Click(RESET),
            ],
        );
    }
}
