use super::*;

/// The doc-drift gate: regenerate every page and compare it with the committed one.
#[test]
fn committed_pages_are_current() {
    let root = repository_root();
    let mut out_of_date = Vec::new();
    for page in generate(&root).expect("generate") {
        let committed = std::fs::read_to_string(root.join(&page.path))
            .unwrap_or_default()
            .replace("\r\n", "\n");
        if committed != page.content {
            out_of_date.push(page.path.display().to_string());
        }
    }
    assert!(
        out_of_date.is_empty(),
        "out of date: {out_of_date:?}; run script/fyx-api-docs and commit the result"
    );
}

/// The hand-written reference may only name items that exist (platform spec decision 11).
#[test]
fn hand_written_reference_names_existing_items() {
    let root = repository_root();
    let mut items = Vec::new();
    for (_, source) in RUST_CRATES {
        items.extend(rust_items(&root.join(source)).expect("items"));
    }
    let mut names = BTreeSet::new();
    for dir in WIT_DIRS {
        names.extend(wit_names(&root.join(dir)).expect("names"));
    }
    let reference = std::fs::read_to_string(root.join(HAND_WRITTEN_REFERENCE)).expect("read");
    let stale = stale_references(&reference, &items, &names);
    assert!(
        stale.is_empty(),
        "{HAND_WRITTEN_REFERENCE} names items that do not exist: {stale:?}"
    );
}

#[test]
fn finds_renamed_items() {
    let items = vec![
        RustItem {
            path: "SurfaceTree".into(),
            kind: "struct",
            summary: String::new(),
            note: None,
        },
        RustItem {
            path: "SurfaceTree::set_props".into(),
            kind: "method",
            summary: String::new(),
            note: None,
        },
        RustItem {
            path: "register_extension!".into(),
            kind: "macro",
            summary: String::new(),
            note: None,
        },
    ];
    let names = BTreeSet::from(["push-update".to_string(), "list-rows".to_string()]);
    let markdown = "Use `SurfaceTree::set_props()` and `register_extension!`, then `push-update`.\n\
        Renamed: `SurfaceTree::set_prop`, `export_extension!`, `push-updates`.\n\
        Ignored: `Vec::new`, `wasm32-wasip2`, `wit-bindgen`; caught: `tick-tock`.\n\
        ```rust\nSurfaceTree::gone();\n```\n";
    assert_eq!(
        stale_references(markdown, &items, &names),
        [
            "SurfaceTree::set_prop",
            "export_extension!",
            "push-updates",
            "tick-tock"
        ]
    );
}

#[test]
fn the_rust_page_lists_sdk_items() {
    let items =
        rust_items(&repository_root().join("crates/ui_ext_sdk/src/ui_ext_sdk.rs")).expect("items");
    let paths = items
        .iter()
        .map(|item| item.path.as_str())
        .collect::<BTreeSet<_>>();
    for expected in [
        "Extension",
        "Extension::handle_event",
        "register_extension!",
        "SurfaceTree::set_props",
        "FlexProps::column",
        "list_rows",
        "TreeError::NotAContainer",
    ] {
        assert!(
            paths.contains(expected),
            "{expected} missing from {paths:?}"
        );
    }
    assert!(
        !paths.iter().any(|path| path.starts_with("__private")),
        "hidden items are left out"
    );
    let host_items =
        rust_items(&repository_root().join("crates/ui_ext_sdk/src/host.rs")).expect("items");
    for name in ["take_pushed_updates", "take_host_effects"] {
        let host_only = host_items
            .iter()
            .find(|item| item.path == name)
            .expect("item");
        assert_eq!(host_only.note, Some("host only"), "{name}");
    }
}
