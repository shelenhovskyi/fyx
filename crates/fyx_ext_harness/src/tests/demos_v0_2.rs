//! The demos' `fyx-ui-0-2` builds, which show what 0.2 adds: the dev-build demos for the 0.2
//! renderer.

use std::sync::OnceLock;

use super::*;

fn gallery_v0_2_wasm() -> Vec<u8> {
    static BUILT: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    demo("fyx_demo_gallery", true, &BUILT)
}

fn decorations_v0_2_wasm() -> Vec<u8> {
    static BUILT: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    demo("fyx_demo_decorations", true, &BUILT)
}

fn decorations_wasm() -> Vec<u8> {
    static BUILT: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    demo("fyx_demo_decorations", false, &BUILT)
}

/// The first text node of the surface that starts with `prefix`.
fn text_starting(harness: &Harness, surface_id: &str, prefix: &str) -> Option<String> {
    let id = harness.find_v0_1(
        surface_id,
        |props| matches!(props, Props::Text(text) if text.content.starts_with(prefix)),
    )?;
    harness.text(surface_id, id).map(ToString::to_string)
}

/// The nodes of the surface whose props match, in tree order.
fn all(harness: &Harness, surface_id: &str, matches: impl Fn(&Props) -> bool) -> Vec<NodeId> {
    harness
        .tree_v0_1(surface_id)
        .map(|tree| tree.to_nodes())
        .unwrap_or_default()
        .into_iter()
        .filter(|node| matches(&node.props))
        .map(|node| node.id)
        .collect()
}

fn row_texts(tree: &Tree) -> Vec<String> {
    tree.to_nodes()
        .into_iter()
        .filter_map(|node| match node.props {
            Props::Text(text) => Some(text.content),
            _ => None,
        })
        .collect()
}

mod gallery {
    pub const PANEL: &str = "gallery-next";
    pub const PICKER: &str = "gallery-picker";
    pub const CLOCK: &str = "gallery-clock";
}

fn is_clock_text(text: &str) -> bool {
    let bytes = text.as_bytes();
    text.len() == 12
        && text.ends_with(" UTC")
        && bytes[2] == b':'
        && bytes[5] == b':'
        && [0, 1, 3, 4, 6, 7]
            .iter()
            .all(|&index| bytes[index].is_ascii_digit())
}

#[test]
fn the_gallery_clock_ticks_every_second() {
    let mut harness = Harness::load(&gallery_v0_2_wasm()).expect("load");
    assert_eq!(
        ids(harness.surfaces().iter().map(|s| s.id.as_str())),
        [
            "gallery",
            "gallery-status",
            gallery::PANEL,
            gallery::PICKER,
            gallery::CLOCK
        ]
    );
    harness.mount(gallery::CLOCK).expect("mount the clock");
    assert_eq!(
        harness.pending_tick(gallery::CLOCK),
        Some(Duration::from_secs(1))
    );
    let clock = all(&harness, gallery::CLOCK, |props| {
        matches!(props, Props::Text(_))
    })[0];
    let text = harness.text(gallery::CLOCK, clock).expect("time");
    assert!(is_clock_text(text), "{text:?}");

    let outcome = harness.tick(gallery::CLOCK).expect("tick");
    assert!(matches!(
        &outcome.effects[..],
        [HostEffect::RequestTick { surface_id, delay }]
            if surface_id == gallery::CLOCK && *delay == Duration::from_secs(1)
    ));
    let text = harness.text(gallery::CLOCK, clock).expect("time");
    assert!(is_clock_text(text), "{text:?}");
}

#[test]
fn the_gallery_shows_tooltips_secret_input_indent_and_every_icon() {
    let mut harness = Harness::load(&gallery_v0_2_wasm()).expect("load");
    harness.mount(gallery::PANEL).expect("mount");

    let delete = harness
        .find_button(gallery::PANEL, "Delete")
        .expect("Delete");
    assert!(matches!(
        harness.props(gallery::PANEL, delete),
        Some(model::Props::Button(button))
            if button.tooltip.as_deref() == Some("Delete for good")
                && button.icon == Some(model::IconRef::Frozen(model::Icon::Trash))
    ));
    harness.click_v0_1(gallery::PANEL, "Delete").expect("click");
    assert_eq!(
        text_starting(&harness, gallery::PANEL, "Last action").as_deref(),
        Some("Last action: Delete")
    );

    let token = all(&harness, gallery::PANEL, |props| {
        matches!(props, Props::Input(_))
    })[0];
    assert!(matches!(
        harness.props(gallery::PANEL, token),
        Some(model::Props::Input(input)) if input.secret
    ));
    harness
        .event(
            gallery::PANEL,
            model::UiEvent::InputChanged(token, "s3cret".into()),
        )
        .expect("typing");
    assert_eq!(
        text_starting(&harness, gallery::PANEL, "Token").as_deref(),
        Some("Token: 6 characters")
    );

    let lists = all(&harness, gallery::PANEL, |props| {
        matches!(props, Props::List(_))
    });
    let [tree_list, icon_list] = lists[..] else {
        panic!("two lists: {lists:?}");
    };
    let rows = harness
        .rows(gallery::PANEL, tree_list, 0, 50)
        .expect("tree rows");
    assert_eq!(
        rows.iter().map(|row| row.indent).collect::<Vec<_>>(),
        [0, 1, 2, 3, 3, 2, 1, 2, 3, 0]
    );

    let outcome = harness
        .click(gallery::PANEL, "Refresh rows")
        .expect("refresh");
    assert_eq!(
        outcome.invalidated,
        [model::InvalidateRowsOp {
            target: tree_list,
            rows: None,
        }]
    );
    let rows = harness
        .rows_v0_1(gallery::PANEL, tree_list, 0, 1)
        .expect("refetched rows");
    assert_eq!(row_texts(&rows[0].1), ["crates", "(scan 1)"]);

    let icons = harness
        .rows_v0_1(gallery::PANEL, icon_list, 0, 200)
        .expect("icon rows");
    assert_eq!(icons.len(), 101);
    assert_eq!(row_texts(&icons[0].1), ["arrow-up"]);
    assert_eq!(row_texts(&icons[100].1), ["blocks"]);
}

#[test]
fn the_gallery_picker_is_driven_by_the_keyboard() {
    let mut harness = Harness::load(&gallery_v0_2_wasm()).expect("load");
    harness.mount(gallery::PANEL).expect("mount");

    harness
        .click_v0_1(gallery::PANEL, "Pick a fruit")
        .expect("open the picker");
    assert!(harness.is_shown(gallery::PICKER));
    let filter = all(&harness, gallery::PICKER, |props| {
        matches!(props, Props::Input(_))
    })[0];
    let list = all(&harness, gallery::PICKER, |props| {
        matches!(props, Props::List(_))
    })[0];
    assert!(matches!(
        harness.props(gallery::PICKER, list),
        Some(model::Props::List(props)) if props.row_count == 12 && props.selected == Some(0)
    ));

    let outcome = harness
        .event(
            gallery::PICKER,
            model::UiEvent::InputChanged(filter, "an".into()),
        )
        .expect("filter");
    assert_eq!(outcome.invalidated.len(), 1);
    assert!(matches!(
        harness.props(gallery::PICKER, list),
        Some(model::Props::List(props)) if props.row_count == 3 && props.selected == Some(0)
    ));
    let rows = harness
        .rows_v0_1(gallery::PICKER, list, 0, 10)
        .expect("rows");
    assert_eq!(
        rows.iter()
            .flat_map(|(_, row)| row_texts(row))
            .collect::<Vec<_>>(),
        ["Banana", "Mango", "Orange"]
    );

    // Down twice: the host moves `selected` and sends row-selected each time.
    for row in [1, 2] {
        harness
            .event(gallery::PICKER, model::UiEvent::RowSelected(list, row))
            .expect("select");
    }
    assert!(matches!(
        harness.props(gallery::PICKER, list),
        Some(model::Props::List(props)) if props.selected == Some(2)
    ));
    let outcome = harness
        .event(
            gallery::PICKER,
            model::UiEvent::InputSubmitted(filter, "an".into()),
        )
        .expect("Enter");
    assert!(matches!(
        &outcome.effects[..],
        [HostEffect::HideSurface { .. }, HostEffect::Push { surface_id, .. }]
            if surface_id == gallery::PANEL
    ));
    assert!(!harness.is_shown(gallery::PICKER));
    assert_eq!(
        text_starting(&harness, gallery::PANEL, "Picked").as_deref(),
        Some("Picked: Orange")
    );

    harness
        .click_v0_1(gallery::PANEL, "Pick a fruit")
        .expect("open");
    let list = all(&harness, gallery::PICKER, |props| {
        matches!(props, Props::List(_))
    })[0];
    harness
        .event(gallery::PICKER, model::UiEvent::RowClick(list, 1))
        .expect("click a row");
    assert_eq!(
        text_starting(&harness, gallery::PANEL, "Picked").as_deref(),
        Some("Picked: Apricot")
    );

    harness
        .click_v0_1(gallery::PANEL, "Pick a fruit")
        .expect("open");
    harness
        .dismiss(gallery::PICKER, model::DismissReason::Escape)
        .expect("Escape");
    assert_eq!(
        text_starting(&harness, gallery::PANEL, "Picked").as_deref(),
        Some("Picked: nothing (escape)")
    );
    assert!(harness.disabled().is_none());
}

const HIGHLIGHTS: &str = "todo-highlights";

fn summary(harness: &Harness) -> Option<String> {
    text_starting(harness, HIGHLIGHTS, "6 comments")
}

#[test]
fn the_todo_highlighter_places_and_removes_decorations() {
    let mut harness = Harness::load(&decorations_v0_2_wasm()).expect("load");
    harness.mount(HIGHLIGHTS).expect("mount");
    assert_eq!(
        summary(&harness).as_deref(),
        Some("6 comments in 4 files, 0 highlighted")
    );

    harness
        .click_v0_1(HIGHLIGHTS, "Highlight all")
        .expect("highlight");
    assert_eq!(harness.decorations().count(), 6);
    let location = harness.decoration("todo-0").expect("todo-0");
    assert_eq!(
        (location.path.as_str(), location.line, location.placement),
        ("src/main.rs", 1, model::DecorationPlacement::Above)
    );
    assert_eq!(
        harness.text("todo-0", 3),
        Some("TODO: read the config path from the command line")
    );
    assert_eq!(
        summary(&harness).as_deref(),
        Some("6 comments in 4 files, 6 highlighted")
    );

    harness
        .click_v0_1("todo-2", "Done")
        .expect("Done in the decoration");
    assert!(harness.decoration("todo-2").is_none());
    assert_eq!(
        summary(&harness).as_deref(),
        Some("6 comments in 4 files, 5 highlighted"),
        "pushed to the panel"
    );

    let list = all(&harness, HIGHLIGHTS, |props| {
        matches!(props, Props::List(_))
    })[0];
    let outcome = harness
        .event(HIGHLIGHTS, model::UiEvent::RowClick(list, 2))
        .expect("row click");
    assert!(harness.decoration("todo-2").is_some());
    assert_eq!(
        outcome.invalidated,
        [model::InvalidateRowsOp {
            target: list,
            rows: Some(model::RowRange { start: 2, count: 1 }),
        }]
    );

    harness.click_v0_1(HIGHLIGHTS, "Clear").expect("clear");
    assert_eq!(harness.decorations().count(), 0);
}

#[test]
fn the_todo_highlighter_reports_refused_decorations() {
    let mut harness =
        Harness::load_with_capabilities(&decorations_v0_2_wasm(), &["ui:panel"]).expect("load");
    harness.mount(HIGHLIGHTS).expect("mount");
    harness
        .click_v0_1(HIGHLIGHTS, "Highlight all")
        .expect("highlight");
    assert_eq!(harness.decorations().count(), 0);
    assert!(text_starting(&harness, HIGHLIGHTS, "not-permitted:").is_some());

    let mut harness = Harness::load(&decorations_wasm()).expect("load the 0.1 build");
    assert_eq!(harness.ui_api_version(), ApiVersion::V0_1_0);
    harness.mount(HIGHLIGHTS).expect("mount");
    harness
        .click_v0_1(HIGHLIGHTS, "Highlight all")
        .expect("highlight");
    let status = text_starting(&harness, HIGHLIGHTS, "unavailable:").expect("status");
    assert!(status.contains("0.2"), "{status}");
}

/// §M.8 in the gallery's 0.2 build: two notifications, a button pressed, and a job that runs on
/// ticks and can be cancelled.
#[test]
fn the_gallery_notifies_and_runs_a_job() {
    use ui_ext::v0_2::notifications::ProgressLocation;
    let mut harness = Harness::load(&gallery_v0_2_wasm()).expect("load");
    harness.mount(gallery::PANEL).expect("mount");

    harness.click(gallery::PANEL, "Notify").expect("notify");
    assert_eq!(harness.notifications().len(), 2);
    let (&info, _) = harness
        .notifications()
        .iter()
        .find(|(_, notification)| notification.actions.len() == 2)
        .expect("the info notification");
    harness.notification_action(info, "open").expect("open");
    assert_eq!(
        text_starting(&harness, gallery::PANEL, "Notifications:").as_deref(),
        Some(format!("Notifications: #{info} pressed open").as_str())
    );

    harness.click(gallery::PANEL, "Run job").expect("run");
    let locations = harness
        .progresses()
        .values()
        .map(|progress| progress.options.location)
        .collect::<Vec<_>>();
    assert_eq!(
        locations,
        [ProgressLocation::StatusBar, ProgressLocation::Panel]
    );
    harness.tick(gallery::PANEL).expect("tick");
    assert_eq!(
        text_starting(&harness, gallery::PANEL, "Job:").as_deref(),
        Some("Job: 10 %")
    );
    let (&bar, _) = harness
        .progresses()
        .iter()
        .find(|(_, progress)| progress.options.cancellable)
        .expect("the status-bar progress");
    harness.cancel_progress(bar).expect("cancel");
    assert!(harness.progresses().is_empty());
    assert_eq!(
        text_starting(&harness, gallery::PANEL, "Job:").as_deref(),
        Some("Job: cancelled")
    );
    assert_eq!(harness.pending_tick(gallery::PANEL), None);
}
