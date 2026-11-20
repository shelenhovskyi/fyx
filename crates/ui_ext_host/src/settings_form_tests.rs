//! Plan 6.1: the Settings form's model and its writes (design wit-0.4 §D.2, §D.3).

use std::rc::Rc;

use gpui::{TestAppContext, VisualTestContext};
use serde_json::{Map, Value, json};

use crate::settings_form::{
    ExtensionSettingsForm, checked_field, checked_items, row_items, row_text, take_pending_open,
};
use crate::settings_model::{
    ExtensionSettingsStore as _, MemorySettingsStore, SettingChoice, SettingKind, SettingSpec,
    SettingValue, SettingsEdit, SettingsForm, set_extension_settings_store,
};

fn specs() -> Vec<SettingSpec> {
    vec![
        SettingSpec {
            key: "block-on-diff".into(),
            title: "Wait for review".into(),
            description: Some("Claude waits until you decide.".into()),
            kind: SettingKind::Boolean,
            default: SettingValue::Boolean(true),
        },
        SettingSpec {
            key: "max-open-diffs".into(),
            title: "Open diffs at most".into(),
            description: None,
            kind: SettingKind::Integer {
                min: Some(1),
                max: Some(64),
            },
            default: SettingValue::Integer(8),
        },
        SettingSpec {
            key: "severity".into(),
            title: "Report as".into(),
            description: None,
            kind: SettingKind::Choice {
                choices: vec![
                    SettingChoice {
                        value: "error".into(),
                        title: "Error".into(),
                    },
                    SettingChoice {
                        value: "warning".into(),
                        title: "Warning".into(),
                    },
                ],
            },
            default: SettingValue::Choice("warning".into()),
        },
        SettingSpec {
            key: "extra-args".into(),
            title: "Extra arguments".into(),
            description: None,
            kind: SettingKind::TextList {
                max_items: 2,
                max_length: 8,
            },
            default: SettingValue::TextList(Vec::new()),
        },
    ]
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

#[test]
fn invalid_values_fall_back_to_the_default_with_an_error() {
    let form = SettingsForm::new(
        &specs(),
        &object(json!({
            "block-on-diff": false,
            "max-open-diffs": 100,
            "severity": "fatal",
            "dropped": 3,
        })),
    );
    let diff = form.row("block-on-diff").expect("row");
    assert_eq!(diff.effective, SettingValue::Boolean(false));
    assert!(diff.error.is_none() && diff.can_reset());
    let max = form.row("max-open-diffs").expect("row");
    assert_eq!(max.effective, SettingValue::Integer(8));
    assert_eq!(max.error.as_deref(), Some("100 is outside 1..=64"));
    assert_eq!(row_text(max), "100");
    let severity = form.row("severity").expect("row");
    assert_eq!(severity.effective, SettingValue::Choice("warning".into()));
    assert!(severity.error.is_some());
    let args = form.row("extra-args").expect("row");
    assert!(!args.can_reset());
    assert!(row_items(args).is_empty());
    assert_eq!(form.unknown, vec![("dropped".to_string(), json!(3))]);
    assert!(form.has_user_values());
    assert!(!SettingsForm::new(&specs(), &Map::new()).has_user_values());
}

#[test]
fn fields_write_only_checked_values() {
    let specs = specs();
    let max = &specs[1];
    assert_eq!(checked_field(max, "12"), Ok(json!(12)));
    assert_eq!(checked_field(max, " 12 "), Ok(json!(12)));
    assert_eq!(checked_field(max, "x"), Err("Enter a whole number".into()));
    assert_eq!(checked_field(max, "0"), Err("0 is outside 1..=64".into()));
    let args = &specs[3];
    assert_eq!(checked_items(args, vec!["-v".into()]), Ok(json!(["-v"])));
    assert!(checked_items(args, vec!["much-too-long".into()]).is_err());
    assert!(checked_items(args, vec!["a".into(), "b".into(), "c".into()]).is_err());
}

#[test]
fn edits_apply_to_the_extension_object() {
    let mut values = object(json!({ "a": 1, "b": 2 }));
    SettingsEdit::Set {
        key: "a".into(),
        value: json!(5),
    }
    .apply(&mut values);
    SettingsEdit::Reset { key: "b".into() }.apply(&mut values);
    assert_eq!(Value::Object(values.clone()), json!({ "a": 5 }));
    SettingsEdit::ResetAll.apply(&mut values);
    assert!(values.is_empty());
}

/// A typed field writes its checked value, keeps the file on a bad one, and a reset brings the
/// default back into the field.
#[gpui::test]
async fn test_form_fields_write_through_the_store(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    let store = Rc::new(MemorySettingsStore::default());
    store.set_specs("ext", specs());
    cx.update(|cx| set_extension_settings_store(store.clone(), cx));
    let window = cx.add_window(|window, cx| ExtensionSettingsForm::new("ext".into(), window, cx));
    let cx = &mut VisualTestContext::from_window(*window, cx);
    let form = window.root(cx).expect("form");
    let field = form.read_with(cx, |form, _| form.field("max-open-diffs").expect("field"));
    assert_eq!(field.read_with(cx, |editor, cx| editor.text(cx)), "8");

    field.update_in(cx, |editor, window, cx| editor.set_text("12", window, cx));
    cx.run_until_parked();
    let user = cx.update(|_, cx| store.user_values("ext", cx));
    assert_eq!(user.get("max-open-diffs"), Some(&json!(12)));

    field.update_in(cx, |editor, window, cx| editor.set_text("99", window, cx));
    cx.run_until_parked();
    let user = cx.update(|_, cx| store.user_values("ext", cx));
    assert_eq!(user.get("max-open-diffs"), Some(&json!(12)));
    assert_eq!(
        form.read_with(cx, |form, _| form.field_error("max-open-diffs")),
        Some("99 is outside 1..=64".into())
    );

    form.update_in(cx, |form, window, cx| {
        form.apply(SettingsEdit::ResetAll, window, cx)
    });
    cx.run_until_parked();
    assert_eq!(field.read_with(cx, |editor, cx| editor.text(cx)), "8");
    assert!(form.read_with(cx, |form, _| !form.form().has_user_values()));
}

#[gpui::test]
fn test_open_extension_settings_leaves_a_pending_open(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    cx.update(|cx| {
        crate::open_extension_settings("ext", Some("severity"), cx);
        assert_eq!(
            take_pending_open(cx),
            Some(("ext".to_string(), Some("severity".to_string())))
        );
        assert_eq!(take_pending_open(cx), None);
    });
}

/// Plan 6.1: the real store reads `fyx_extensions.<id>` from the user settings file only, and the
/// form's edits land there; "Reset all" removes the extension's object.
#[gpui::test]
async fn user_settings_file_store_reads_and_writes_fyx_extensions(cx: &mut TestAppContext) {
    use crate::settings_model::UserSettingsFileStore;
    crate::tests::init_test(cx);
    let fs = fs::FakeFs::new(cx.executor());
    fs.insert_tree(paths::config_dir(), json!({})).await;
    cx.update(|cx| <dyn fs::Fs>::set_global(fs.clone(), cx));
    let store = UserSettingsFileStore;

    cx.update(|cx| {
        gpui::BorrowAppContext::update_global::<settings::SettingsStore, _>(cx, |settings, cx| {
            settings
                .set_user_settings(
                    r#"{ "fyx_extensions": { "acme.todo": { "limit": 5 }, "acme.bad": 3 } }"#,
                    cx,
                )
                .expect("valid user settings");
        });
    });
    cx.update(|cx| {
        assert_eq!(
            Value::Object(store.user_values("acme.todo", cx)),
            json!({ "limit": 5 })
        );
        assert!(
            store.user_values("acme.bad", cx).is_empty(),
            "not an object"
        );
        assert!(store.user_values("acme.none", cx).is_empty());
        // Nothing is loaded, so nothing declares a setting.
        assert!(store.specs("acme.todo", cx).is_empty());
    });

    // The file keeps Zed's comments, so it is read as text.
    let written = |fs: &std::sync::Arc<fs::FakeFs>| {
        let text = fs
            .read_file_sync(paths::settings_file())
            .expect("the settings file");
        String::from_utf8(text).expect("utf-8")
    };
    cx.update(|cx| {
        store.write(
            "acme.todo",
            SettingsEdit::Set {
                key: "mode".into(),
                value: json!("fast"),
            },
            cx,
        )
    });
    cx.run_until_parked();
    let text = written(&fs);
    assert!(
        text.contains("\"acme.todo\"") && text.contains("\"mode\": \"fast\""),
        "the edit goes to the user settings file: {text}"
    );
    cx.update(|cx| store.write("acme.todo", SettingsEdit::ResetAll, cx));
    cx.run_until_parked();
    let text = written(&fs);
    assert!(
        !text.contains("acme.todo"),
        "reset all removes the object: {text}"
    );
}
