//! Plan 2.6 end to end: W4's `fyx_demo_decorations` (the TODO highlighter), built for
//! `fyx:ui` 0.2, places, repaints and removes decorations in a real editor through the registry.

use std::{
    cell::Cell,
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
    sync::{LazyLock, Mutex},
    time::Duration,
};

use anyhow::{Result, anyhow};
use fs::{FakeFs, Fs as _};
use futures::{FutureExt as _, future::LocalBoxFuture};
use gpui::{AppContext as _, Entity, Modifiers, TestAppContext, VisualTestContext};
use language::{Buffer, Point};
use project::Project;
use ui_ext::v0_2::{NodeId, Props, Surface, UiEvent, Update, tree::Tree};
use ui_ext_wasm::UiExtension;
use util::path;

use crate::{
    Decorations, SurfaceState, UiExtensionRegistry,
    backend::{Reply, Row, UiBackend},
    decoration_tests::{block_row, init, insert, only_block, open_editor, redraw},
};

const EXTENSION: &str = "fyx-demo-decorations";
const PANEL: &str = "todo-highlights";
/// The demo's comment 0 is `src/main.rs` line 1 (zero-based), placed above it as `todo-0`.
const FIRST: &str = "todo-0";
/// `Done` in a decoration, pinned by the demo's crate doc.
const DONE_SELECTOR: &str = "fyx-node-4";

/// The demo's `fyx-ui-0-2` build. `ui_ext_wasm::test_support::demo_wasm` builds the default
/// (0.1) world only, so this builds the feature into a target directory of its own, which keeps
/// the two builds of the package from overwriting each other.
fn decorations_v0_2_wasm() -> Vec<u8> {
    static BUILT: LazyLock<Mutex<Option<std::result::Result<PathBuf, String>>>> =
        LazyLock::new(|| Mutex::new(None));
    let mut built = BUILT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = built
        .get_or_insert_with(build_v0_2)
        .clone()
        .unwrap_or_else(|error| panic!("{error}"));
    std::fs::read(&path).unwrap_or_else(|error| panic!("reading {path:?}: {error}"))
}

fn build_v0_2() -> std::result::Result<PathBuf, String> {
    let package = "fyx_demo_decorations";
    // <target>/debug/deps/<test binary>
    let target_dir = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .ancestors()
        .nth(3)
        .ok_or("test binary is not inside a cargo target directory")?
        .join("fyx-ui-fixtures-v0_2");
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let feature = format!("{package}/fyx-ui-0-2");
    let args = [
        "build",
        "--locked",
        "--target",
        "wasm32-wasip2",
        "-p",
        package,
        "--features",
        feature.as_str(),
    ];
    // Blocking is fine: once per test process, before executor work.
    #[allow(clippy::disallowed_methods)]
    let status = Command::new(option_env!("CARGO").unwrap_or("cargo"))
        .args(args)
        .arg("--target-dir")
        .arg(&target_dir)
        .current_dir(&workspace)
        .status()
        .map_err(|error| format!("failed to run cargo: {error}"))?;
    if !status.success() {
        return Err(format!("cargo {args:?} failed: {status}"));
    }
    Ok(target_dir
        .join("wasm32-wasip2")
        .join("debug")
        .join(format!("{package}.wasm")))
}

/// The real component, except that one chosen call fails the way a trapped or disabled instance
/// does (an error without a `ReplyError`). The demo cannot trap on purpose.
struct Trappable {
    inner: UiExtension,
    trap_next_event: Cell<bool>,
}

impl UiBackend for Trappable {
    fn extension_id(&self) -> &str {
        self.inner.extension_id()
    }

    fn api_version(&self) -> semver::Version {
        UiBackend::api_version(&self.inner)
    }

    fn mount(&self, surface_id: &str) -> LocalBoxFuture<'static, Result<Reply<Tree>>> {
        UiBackend::mount(&self.inner, surface_id)
    }

    fn handle_event(
        &self,
        surface_id: &str,
        event: UiEvent,
    ) -> LocalBoxFuture<'static, Result<Reply<Update>>> {
        if self.trap_next_event.replace(false) {
            return async { Err(anyhow!("wasm trap: unreachable")) }.boxed_local();
        }
        UiBackend::handle_event(&self.inner, surface_id, event)
    }

    fn list_rows(
        &self,
        surface_id: &str,
        list_node: NodeId,
        start: u32,
        count: u32,
    ) -> LocalBoxFuture<'static, Result<Reply<Vec<Row>>>> {
        UiBackend::list_rows(&self.inner, surface_id, list_node, start, count)
    }

    fn tick(&self, surface_id: &str) -> LocalBoxFuture<'static, Result<Reply<Update>>> {
        UiBackend::tick(&self.inner, surface_id)
    }
}

/// Extension calls run on tokio, so wait by the wall clock.
fn run_until(
    cx: &mut VisualTestContext,
    what: &str,
    mut done: impl FnMut(&mut VisualTestContext) -> bool,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        redraw(cx);
        if done(cx) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

/// `<root>/demo/src/main.rs` with the demo's mocked text, so its comments' lines exist.
async fn demo_project(cx: &mut TestAppContext) -> (Entity<Project>, Entity<Buffer>) {
    let fs = FakeFs::new(cx.executor());
    fs.create_dir(Path::new(path!("/root/demo/src")))
        .await
        .expect("demo dir");
    fs.insert_file(path!("/root/demo/src/main.rs"), MAIN_RS.as_bytes().to_vec())
        .await;
    let project = Project::test(fs, [Path::new(path!("/root/demo"))], cx).await;
    let buffer = project
        .update(cx, |project, cx| {
            project.open_local_buffer(path!("/root/demo/src/main.rs"), cx)
        })
        .await
        .expect("buffer opens");
    (project, buffer)
}

/// The demo's mocked `src/main.rs` (its `MOCK_FILES`), copied: line 1 is a TODO, line 3 a FIXME.
const MAIN_RS: &str = "fn main() {\n    // TODO: read the config path from the command line\n    let config = load_config(\"fyx.toml\");\n    // FIXME(sam): this panics when the file is missing\n    run(config.unwrap());\n}\n";

fn panel_state(
    registry: &Entity<UiExtensionRegistry>,
    cx: &mut VisualTestContext,
) -> Entity<SurfaceState> {
    registry.read_with(cx, |registry, _| {
        registry
            .surface_of(EXTENSION, PANEL)
            .expect("the panel is registered")
            .state
            .clone()
    })
}

/// The first node of the panel's tree whose props match.
fn panel_node(
    state: &Entity<SurfaceState>,
    cx: &mut VisualTestContext,
    matches: impl Fn(&Props) -> bool,
) -> Option<NodeId> {
    state.read_with(cx, |state, _| {
        state
            .tree()?
            .to_nodes()
            .into_iter()
            .find(|node| matches(&node.props))
            .map(|node| node.id)
    })
}

fn summary(state: &Entity<SurfaceState>, cx: &mut VisualTestContext) -> Option<String> {
    state.read_with(cx, |state, _| {
        state
            .tree()?
            .to_nodes()
            .into_iter()
            .find_map(|node| match node.props {
                Props::Text(text) if text.content.contains(" highlighted") => Some(text.content),
                _ => None,
            })
    })
}

fn live(decorations: &Entity<Decorations>, id: &str, cx: &mut VisualTestContext) -> bool {
    decorations.read_with(cx, |decorations, _| {
        decorations.extension_decoration(EXTENSION, id).is_some()
    })
}

/// A row click in the panel toggles comment 0's decoration.
fn toggle_first(registry: &Entity<UiExtensionRegistry>, cx: &mut VisualTestContext) {
    toggle_row(registry, 0, cx);
}

/// A row click in the panel toggles that comment's decoration (`todo-<row>`).
fn toggle_row(registry: &Entity<UiExtensionRegistry>, row: u32, cx: &mut VisualTestContext) {
    let panel = panel_state(registry, cx);
    let list = panel_node(&panel, cx, |props| matches!(props, Props::List(_)))
        .expect("the panel has its comment list");
    panel.update(cx, |state, cx| {
        state.dispatch(UiEvent::RowClick(list, row), cx)
    });
}

#[gpui::test]
async fn test_todo_highlighter_decorations_end_to_end(cx: &mut TestAppContext) {
    let decorations = init(cx);
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let (extension, _pushed) = cx
        .update(|cx| UiExtension::load(EXTENSION.into(), decorations_v0_2_wasm(), &cx.to_async()))
        .await
        .expect("the 0.2 component loads");
    assert_eq!(extension.api_version().minor, 2, "the 0.2 world");
    let surfaces: Vec<Surface> = extension.surfaces_v0_2().to_vec();
    let backend = Rc::new(Trappable {
        inner: extension,
        trap_next_event: Cell::new(false),
    });
    let registry = cx.new(|_| UiExtensionRegistry::trusting());
    registry.update(cx, |registry, cx| {
        let backend: Rc<dyn UiBackend> = backend.clone();
        registry.register_backend(backend, &surfaces, cx)
    });

    let (project, buffer) = demo_project(cx).await;
    let (editor, mut vcx) = open_editor(&project, &buffer, cx);
    let vcx = &mut vcx;
    let panel = panel_state(&registry, vcx);
    run_until(vcx, "the panel mounts", |cx| summary(&panel, cx).is_some());
    assert!(summary(&panel, vcx).unwrap().ends_with("0 highlighted"));

    // A row click: the extension's reply carries `place-decoration(todo-0)`, which the registry
    // applies; the block sits above `src/main.rs` line 1.
    toggle_first(&registry, vcx);
    run_until(vcx, "todo-0 is placed", |cx| live(&decorations, FIRST, cx));
    let id = decorations
        .read_with(vcx, |decorations, _| {
            decorations.extension_decoration(EXTENSION, FIRST)
        })
        .expect("live");
    let block = only_block(&decorations, &editor, id, vcx);
    assert_eq!(block_row(&editor, block, vcx), 1, "above buffer row 1");
    assert!(summary(&panel, vcx).unwrap().ends_with("1 highlighted"));

    // The block follows its line.
    insert(&buffer, Point::new(0, 0), "use std::env;\n\n", vcx);
    assert_eq!(block_row(&editor, block, vcx), 3);

    // `Done` in the block: `handle-event(todo-0, click 4)`, whose effects remove the decoration
    // and push to the panel (another surface).
    let done = vcx
        .debug_bounds(DONE_SELECTOR)
        .expect("the decoration's Done button is drawn");
    vcx.simulate_click(done.center(), Modifiers::none());
    run_until(vcx, "Done removes todo-0", |cx| {
        !live(&decorations, FIRST, cx)
    });
    run_until(vcx, "the panel hears of it", |cx| {
        summary(&panel, cx).is_some_and(|summary| summary.ends_with("0 highlighted"))
    });
    assert!(
        decorations
            .read_with(vcx, |decorations, _| decorations
                .blocks_in(editor.entity_id(), id))
            .is_empty()
    );
    assert!(vcx.debug_bounds(DONE_SELECTOR).is_none());

    // A reload (a new instance registered under the same id) starts without decorations.
    toggle_first(&registry, vcx);
    run_until(vcx, "todo-0 is placed again", |cx| {
        live(&decorations, FIRST, cx)
    });
    let (reloaded, _pushed) = vcx
        .update(|_, cx| {
            UiExtension::load(EXTENSION.into(), decorations_v0_2_wasm(), &cx.to_async())
        })
        .await
        .expect("the 0.2 component loads again");
    let backend = Rc::new(Trappable {
        inner: reloaded,
        trap_next_event: Cell::new(false),
    });
    registry.update(vcx, |registry, cx| {
        let backend: Rc<dyn UiBackend> = backend.clone();
        registry.register_backend(backend, &surfaces, cx)
    });
    redraw(vcx);
    assert!(!live(&decorations, FIRST, vcx));
    assert!(vcx.debug_bounds(DONE_SELECTOR).is_none());

    // An instance that dies during a call in the block takes its decorations with it.
    let panel = panel_state(&registry, vcx);
    run_until(vcx, "the panel mounts again", |cx| {
        summary(&panel, cx).is_some()
    });
    toggle_first(&registry, vcx);
    toggle_row(&registry, 1, vcx);
    run_until(vcx, "todo-0 and todo-1 are placed", |cx| {
        live(&decorations, FIRST, cx) && live(&decorations, "todo-1", cx)
    });
    // The trap hits a call in one block; the other decoration goes because the instance did
    // (`fail_extension`), not because its own call failed.
    backend.trap_next_event.set(true);
    let done = vcx
        .debug_bounds(DONE_SELECTOR)
        .expect("the decoration's Done button is drawn");
    vcx.simulate_click(done.center(), Modifiers::none());
    run_until(vcx, "the trap removes every decoration", |cx| {
        decorations.read_with(cx, |decorations, _| decorations.count_of(EXTENSION)) == 0
    });
}
