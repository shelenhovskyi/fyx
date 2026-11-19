//! The staged 0.4 world against `fixtures/probe_v0_4`: the `terminals` import over a fake
//! [`TerminalHost`], the `terminal` node, and the interfaces that still trap.

use std::{
    collections::HashMap,
    sync::{Mutex, atomic::AtomicU64},
};

use gpui::TestAppContext;
use ui_ext::{
    terminal::{
        self, CreateCommand, PseudoRequest, TerminalError, TerminalEvent, TerminalHandle,
        TerminalHost, TerminalInfo, TerminalOrigin, TerminalSink, TerminalStatus,
    },
    v0_2::{Props, UiEvent, Update},
};

use super::*;
use crate::{
    permissions::{CapabilityCheck, CapabilityDenied, PermissionState},
    process::{Workspace, WorktreeRoot},
};

const SURFACE: &str = "probe";

/// Records every call; owns the handles it allocated, like `terminal_host`.
#[derive(Default)]
struct FakeHost {
    next: AtomicU64,
    calls: Mutex<Vec<String>>,
    sinks: Mutex<HashMap<u64, TerminalSink>>,
    created: Mutex<Vec<CreateCommand>>,
}

impl FakeHost {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }

    fn open(&self, sink: TerminalSink) -> TerminalHandle {
        let handle = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        self.sinks.lock().unwrap().insert(handle, sink);
        TerminalHandle(handle)
    }

    fn known(&self, handle: TerminalHandle) -> Result<(), TerminalError> {
        if self.sinks.lock().unwrap().contains_key(&handle.0) {
            Ok(())
        } else {
            Err(TerminalError::NotFound)
        }
    }

    fn send(&self, handle: u64, event: TerminalEvent) {
        let sink = self.sinks.lock().unwrap().get(&handle).cloned().unwrap();
        sink(event);
    }
}

impl TerminalHost for FakeHost {
    fn open_pseudo(
        &self,
        ext: &str,
        request: PseudoRequest,
        sink: TerminalSink,
    ) -> Result<TerminalHandle, TerminalError> {
        self.record(format!("open-pseudo {ext} {}", request.title));
        Ok(self.open(sink))
    }

    fn create(
        &self,
        ext: &str,
        command: CreateCommand,
        sink: TerminalSink,
    ) -> Result<TerminalHandle, TerminalError> {
        self.record(format!("create {ext} {:?}", command.args));
        self.created.lock().unwrap().push(command);
        Ok(self.open(sink))
    }

    fn write_output(
        &self,
        _: &str,
        handle: TerminalHandle,
        bytes: Vec<u8>,
    ) -> Result<(), TerminalError> {
        self.known(handle)?;
        self.record(format!(
            "write-output {} {}",
            handle.0,
            String::from_utf8_lossy(&bytes)
        ));
        Ok(())
    }

    fn send_input(
        &self,
        _: &str,
        handle: TerminalHandle,
        bytes: Vec<u8>,
    ) -> Result<(), TerminalError> {
        self.known(handle)?;
        self.record(format!(
            "send-input {} {}",
            handle.0,
            String::from_utf8_lossy(&bytes)
        ));
        Ok(())
    }

    fn paste(&self, _: &str, handle: TerminalHandle, text: String) -> Result<(), TerminalError> {
        self.known(handle)?;
        self.record(format!("paste {} {text}", handle.0));
        Ok(())
    }

    fn set_status(
        &self,
        _: &str,
        handle: TerminalHandle,
        status: Option<TerminalStatus>,
    ) -> Result<(), TerminalError> {
        self.record(format!(
            "set-status {} {:?}",
            handle.0,
            status.map(|status| status.label)
        ));
        Ok(())
    }

    fn close(&self, _: &str, handle: TerminalHandle) {
        self.record(format!("close {}", handle.0));
    }

    fn observe(&self, ext: &str, sink: Option<TerminalSink>) -> Vec<TerminalInfo> {
        self.record(format!("observe {ext} {}", sink.is_some()));
        vec![TerminalInfo {
            handle: TerminalHandle(50),
            origin: TerminalOrigin::User,
            title: "•••".into(),
            cwd: None,
            status: None,
            account: None,
        }]
    }

    fn close_all(&self, ext: &str) {
        self.record(format!("close-all {ext}"));
    }
}

/// A broker that grants T1 and answers T2 from a map (`granted` when absent).
struct Broker(HashMap<&'static str, PermissionState>);

impl CapabilityCheck for Broker {
    fn check(&self, _: &str, _: &str) -> Result<(), CapabilityDenied> {
        Ok(())
    }

    fn state(&self, _: &str, capability: &str) -> PermissionState {
        self.0
            .get(capability)
            .copied()
            .unwrap_or(PermissionState::Granted)
    }
}

struct Worktrees(Vec<WorktreeRoot>);

impl Workspace for Worktrees {
    fn worktrees(&self) -> Vec<WorktreeRoot> {
        self.0.clone()
    }
}

/// The shell every platform has, on `PATH`.
fn shell() -> &'static str {
    if cfg!(windows) { "cmd" } else { "sh" }
}

struct Setup {
    host: Arc<FakeHost>,
    states: Vec<(&'static str, PermissionState)>,
    workspace: Option<Vec<WorktreeRoot>>,
    capabilities: Vec<&'static str>,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            host: Arc::default(),
            states: Vec::new(),
            workspace: Some(vec![WorktreeRoot {
                id: 1,
                root: std::env::temp_dir(),
                trusted: true,
            }]),
            capabilities: vec![
                "ui:panel",
                terminal::CAPABILITY_PSEUDO,
                terminal::CAPABILITY_CREATE,
                terminal::CAPABILITY_ENV,
                terminal::CAPABILITY_OBSERVE,
            ],
        }
    }
}

async fn load(setup: &Setup, cx: &mut TestAppContext) -> Result<UiExtension> {
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let env = terminal::check_terminal_env(
        terminal::TerminalEnvSpec {
            vars: [("PROBE_HOME".to_string(), "${package}/home".to_string())].into(),
            scope: terminal::EnvScope::Profiles,
            reason: "the probe's home".into(),
        },
        &|_| None,
    )
    .unwrap();
    let options = LoadOptions {
        allow_staged: Some(true),
        capabilities: Some(setup.capabilities.iter().map(|c| c.to_string()).collect()),
        capability_check: Some(Arc::new(Broker(setup.states.iter().copied().collect()))),
        terminals: Some(setup.host.clone()),
        terminal_programs: Some(vec![terminal::TerminalProgramSpec {
            name: shell().into(),
            args: vec!["*".into()],
            path: None,
            reason: "runs the probe's shell".into(),
        }]),
        terminal_env: Some(env),
        package_dir: Some(PathBuf::from("pkg")),
        workspace: setup
            .workspace
            .clone()
            .map(|worktrees| Arc::new(Worktrees(worktrees)) as Arc<dyn Workspace>),
        ..LoadOptions::default()
    };
    cx.update(|cx| {
        UiExtension::load_with_options(
            "probe-v0-4".into(),
            crate::test_support::fixture_wasm("fyx_ui_test_probe_v0_4"),
            options,
            &cx.to_async(),
        )
    })
    .await
    .map(|(extension, _pushed)| extension)
}

fn report(update: &Update) -> String {
    match update {
        Update::Replace(nodes) => match &nodes[0].props {
            Props::Text(text) => text.content.clone(),
            other => panic!("not a report: {other:?}"),
        },
        other => panic!("not a report: {other:?}"),
    }
}

async fn command(extension: &UiExtension, command: &str) -> String {
    let reply = extension
        .handle_event_v0_2(SURFACE, UiEvent::InputChanged(1, command.into()))
        .await
        .unwrap_or_else(|error| panic!("{command}: {error:#}"));
    report(&reply.value)
}

#[gpui::test]
async fn test_v0_4_pseudo_terminals(cx: &mut TestAppContext) {
    let setup = Setup::default();
    let extension = load(&setup, cx).await.unwrap();
    assert_eq!(extension.ui_api_version(), ApiVersion::V0_4_0);
    assert!(extension.terminals_available());
    assert_eq!(command(&extension, "pseudo build").await, "1");
    assert_eq!(command(&extension, "out 1 hello").await, "done");
    assert_eq!(command(&extension, "out-big 1").await, "limit-exceeded");
    // Not its own (another extension's, or none): the import refuses before the host sees it.
    assert_eq!(command(&extension, "out 7 x").await, "invalid-argument");
    // Its own terminal needs no `terminal:status`; another's does, and it is not declared.
    assert_eq!(command(&extension, "status 1 busy").await, "done");
    assert_eq!(command(&extension, "status 7 busy").await, "not-permitted");
    // `input-changed` is no gesture.
    assert_eq!(command(&extension, "paste 1 ls").await, "not-permitted");
    assert_eq!(
        setup.host.calls(),
        [
            "open-pseudo probe-v0-4 build",
            "write-output 1 hello",
            "set-status 1 Some(\"busy\")",
        ]
    );

    // The host's events reach `handle-io`, in order, as the 0.4 cases.
    let mut events = extension.take_io_events().unwrap();
    setup
        .host
        .send(1, TerminalEvent::Input(TerminalHandle(1), b"ab".to_vec()));
    setup
        .host
        .send(1, TerminalEvent::Resized(TerminalHandle(1), 100, 30));
    for expected in ["input 1 ab", "resized 1 100 30"] {
        let event = events.next().await.unwrap();
        let reply = extension.handle_io(event).await.unwrap();
        assert!(
            format!("{:?}", reply.effects).contains(expected),
            "{expected}: {:?}",
            reply.effects
        );
    }

    assert_eq!(command(&extension, "close 1").await, "done");
    assert_eq!(command(&extension, "out 1 x").await, "invalid-argument");
    drop(events);
    drop(extension);
    // The last clone gone: every terminal of the extension closes, once its call queue (on
    // tokio) drops the store.
    let closed = || {
        setup
            .host
            .calls()
            .contains(&"close-all probe-v0-4".to_string())
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !closed() && std::time::Instant::now() < deadline {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        setup
            .host
            .calls()
            .contains(&"close-all probe-v0-4".to_string()),
        "{:?}",
        setup.host.calls()
    );
}

#[gpui::test]
async fn test_v0_4_created_terminals(cx: &mut TestAppContext) {
    let setup = Setup::default();
    let extension = load(&setup, cx).await.unwrap();
    assert_eq!(command(&extension, "create nope").await, "not-permitted");
    // `*` is one argument: two are refused like `process:spawn`'s patterns.
    assert_eq!(
        command(&extension, &format!("create {} a b", shell())).await,
        "invalid-argument"
    );
    assert_eq!(
        command(&extension, &format!("create {} hi", shell())).await,
        "1"
    );
    let created = setup.host.created.lock().unwrap().clone();
    assert_eq!(created.len(), 1);
    assert_eq!(created[0].args, ["hi"]);
    // `terminal:env` is granted: its values, with `${package}` resolved.
    assert_eq!(
        created[0].env,
        [("PROBE_HOME".to_string(), "pkg/home".to_string())]
    );
    assert_eq!(command(&extension, "input 1 ls").await, "done");
    // A pseudo-terminal's call on a created one is refused.
    assert_eq!(command(&extension, "out 1 x").await, "invalid-argument");
}

#[gpui::test]
async fn test_v0_4_created_terminals_need_a_trusted_worktree(cx: &mut TestAppContext) {
    let untrusted = Setup {
        workspace: Some(vec![WorktreeRoot {
            id: 1,
            root: std::env::temp_dir(),
            trusted: false,
        }]),
        ..Setup::default()
    };
    let extension = load(&untrusted, cx).await.unwrap();
    assert_eq!(
        command(&extension, &format!("create {} hi", shell())).await,
        "untrusted-worktree"
    );
    let none = Setup {
        workspace: None,
        ..Setup::default()
    };
    let extension = load(&none, cx).await.unwrap();
    assert_eq!(
        command(&extension, &format!("create {} hi", shell())).await,
        "no-worktree"
    );
    assert!(untrusted.host.created.lock().unwrap().is_empty());
}

#[gpui::test]
async fn test_v0_4_t2_states(cx: &mut TestAppContext) {
    let setup = Setup {
        states: vec![
            (terminal::CAPABILITY_CREATE, PermissionState::Denied),
            (terminal::CAPABILITY_ENV, PermissionState::Denied),
        ],
        ..Setup::default()
    };
    let extension = load(&setup, cx).await.unwrap();
    assert_eq!(command(&extension, "perm terminal-create").await, "denied");
    assert_eq!(
        command(&extension, "perm terminal-observe").await,
        "granted"
    );
    // Not declared.
    assert_eq!(
        command(&extension, "perm network-listen").await,
        "unavailable"
    );
    assert_eq!(
        command(&extension, &format!("create {} hi", shell())).await,
        "denied"
    );
    assert_eq!(command(&extension, "list").await, "50:user:•••");
    assert_eq!(
        setup.host.calls(),
        ["observe probe-v0-4 true"],
        "a denied create never reaches the host"
    );
}

#[gpui::test]
async fn test_v0_4_env_needs_its_grant(cx: &mut TestAppContext) {
    let setup = Setup {
        states: vec![(terminal::CAPABILITY_ENV, PermissionState::Denied)],
        ..Setup::default()
    };
    let extension = load(&setup, cx).await.unwrap();
    assert_eq!(
        command(&extension, &format!("create {} hi", shell())).await,
        "1"
    );
    assert!(setup.host.created.lock().unwrap()[0].env.is_empty());
}

#[gpui::test]
async fn test_v0_4_without_a_host_terminals_are_unavailable(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    cx.update(gpui_tokio::init);
    let extension = cx
        .update(|cx| {
            UiExtension::load_with_options(
                "probe-v0-4".into(),
                crate::test_support::fixture_wasm("fyx_ui_test_probe_v0_4"),
                LoadOptions {
                    allow_staged: Some(true),
                    capabilities: Some(vec!["ui:panel".into(), "terminal:pseudo".into()]),
                    capability_check: Some(Arc::new(Broker(HashMap::new()))),
                    ..LoadOptions::default()
                },
                &cx.to_async(),
            )
        })
        .await
        .unwrap()
        .0;
    assert!(!extension.terminals_available());
    assert_eq!(command(&extension, "pseudo x").await, "unavailable");
    // Not declared: refused before the host.
    assert_eq!(command(&extension, "list").await, "not-permitted");
}

#[gpui::test]
async fn test_v0_4_terminal_nodes_and_unbuilt_interfaces(cx: &mut TestAppContext) {
    let extension = load(&Setup::default(), cx).await.unwrap();
    let error = extension
        .handle_event_v0_2(SURFACE, UiEvent::InputChanged(1, "node".into()))
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("terminal nodes are not drawn"),
        "{error:#}"
    );
    // T3 is not grantable yet.
    assert_eq!(command(&extension, "read 1").await, "not-permitted");
    let error = extension
        .handle_event_v0_2(SURFACE, UiEvent::InputChanged(1, "listen".into()))
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("not implemented by this build"),
        "{error:#}"
    );
}

#[test]
fn test_v0_4_icons_match_the_wit() {
    let mapped: Vec<ui_ext::v0_2::Icon> = fyx_ui_v0_4::WIT_ICONS
        .iter()
        .map(|&icon| icon.into())
        .collect();
    assert_eq!(mapped, ui_ext::v0_2::Icon::ALL);
}
