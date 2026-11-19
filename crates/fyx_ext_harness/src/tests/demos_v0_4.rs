//! The pseudo-terminal demo's `fyx-ui-0-4` build, loaded as a package (plan 5.10): its shell
//! runs headless on the harness's fake terminals.

use std::sync::OnceLock;

use super::*;

const PANEL: &str = "pseudo-terminal";

/// `extensions/fyx_demo_pseudo_terminal` with its 0.4 component, in a folder of the test target
/// directory. Built on its own: the SDK's 0.4 feature must not reach the 0.3 demos.
fn demo_package() -> PathBuf {
    static BUILT: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    let wasm = BUILT
        .get_or_init(|| {
            build_guest(
                &workspace_dir(),
                "fyx-ui-fixtures-v0_4",
                &[
                    "-p",
                    "fyx_demo_pseudo_terminal",
                    "--features",
                    "fyx_demo_pseudo_terminal/fyx-ui-0-4",
                ],
                "debug/fyx_demo_pseudo_terminal.wasm",
            )
        })
        .clone()
        .unwrap_or_else(|error| panic!("{error}"));
    let package = wasm
        .ancestors()
        .nth(3)
        .expect("a target directory")
        .join(format!("fyx_demo_pseudo_terminal-{}", std::process::id()));
    if package.exists() {
        std::fs::remove_dir_all(&package).expect("clean");
    }
    std::fs::create_dir_all(&package).expect("package dir");
    let source = workspace_dir().join("extensions/fyx_demo_pseudo_terminal");
    std::fs::copy(
        source.join("extension.toml"),
        package.join("extension.toml"),
    )
    .expect("manifest");
    std::fs::copy(&wasm, package.join("fyx_ui.wasm"))
        .unwrap_or_else(|error| panic!("copying {wasm:?}: {error}"));
    package
}

fn status_line(harness: &Harness) -> String {
    harness
        .nodes(PANEL)
        .expect("mounted")
        .into_iter()
        .find_map(|node| match node.props {
            model::Props::Text(text) if text.content.starts_with("WASM shell") => {
                Some(text.content)
            }
            _ => None,
        })
        .expect("a status line")
}

#[test]
fn the_shell_runs_in_a_panel_terminal() {
    let mut harness = Harness::load_package(demo_package()).expect("loads");
    harness.mount(PANEL).expect("mounts");
    let [handle] = harness.terminal_nodes(PANEL)[..] else {
        panic!("one terminal node: {:?}", harness.terminal_nodes(PANEL));
    };
    let terminal = harness.terminal(handle).expect("open");
    assert!(terminal.in_panel);
    assert!(
        terminal.plain_output().starts_with("Fyx WASM shell"),
        "{:?}",
        terminal.plain_output()
    );

    harness.type_into_terminal(handle, b"echo hi\r").unwrap();
    harness.resize_terminal(handle, 80, 24).unwrap();
    harness.resize_terminal(handle, 100, 30).unwrap();
    harness.deliver_io().expect("delivered");
    let output = harness.terminal(handle).unwrap().plain_output();
    assert!(output.ends_with("echo hi\nhi\nwasm$ "), "{output:?}");
    assert_eq!(status_line(&harness), "WASM shell, 100×30");

    harness.type_into_terminal(handle, b"size\r").unwrap();
    harness.deliver_io().expect("delivered");
    let output = harness.terminal(handle).unwrap().plain_output();
    assert!(output.contains("100 columns, 30 rows"), "{output:?}");

    harness.click(PANEL, "Busy").expect("busy");
    let status = harness.terminal(handle).unwrap().status.clone();
    assert_eq!(
        status.map(|status| (status.state, status.label)),
        Some((ui_ext::terminal::StatusState::Working, "Busy".into()))
    );
    harness.click(PANEL, "Idle").expect("idle");
    assert_eq!(harness.terminal(handle).unwrap().status, None);

    // A remount shows the same terminal; it opens once.
    harness.mount(PANEL).expect("remounts");
    assert_eq!(harness.terminal_nodes(PANEL), [handle]);
    assert_eq!(harness.terminals().len(), 1);
}

#[test]
fn without_terminal_pseudo_the_panel_says_why() {
    let package = demo_package();
    let wasm = std::fs::read(package.join("fyx_ui.wasm")).expect("component");
    let mut harness = Harness::load_with_capabilities(&wasm, &["ui:panel"]).expect("loads");
    harness.mount(PANEL).expect("mounts");
    assert!(harness.terminal_nodes(PANEL).is_empty());
    assert!(
        status_line_or_error(&harness).contains("\"terminal:pseudo\" is not in the manifest"),
        "{:?}",
        harness.nodes(PANEL)
    );
}

fn status_line_or_error(harness: &Harness) -> String {
    harness
        .nodes(PANEL)
        .expect("mounted")
        .into_iter()
        .find_map(|node| match node.props {
            model::Props::Text(text) if text.content.starts_with("No terminal") => {
                Some(text.content)
            }
            _ => None,
        })
        .unwrap_or_default()
}
