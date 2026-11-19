//! The staged 0.4 world against `fixtures/probe_v0_4`: the `terminals` import over a fake
//! [`TerminalHost`], the `terminal` node, and the interfaces that still trap.

use std::{
    collections::HashMap,
    sync::{Mutex, atomic::AtomicU64},
};

use gpui::TestAppContext;
use ui_ext::{
    terminal::{
        self, CreateCommand, PseudoRequest, ReadScope, T3Target, TerminalError, TerminalEvent,
        TerminalHandle, TerminalHost, TerminalInfo, TerminalIntegration, TerminalOrigin,
        TerminalSink, TerminalStatus,
    },
    v0_2::{Props, UiEvent, Update},
};

use super::*;
use crate::{
    permissions::{
        CapabilityCheck, CapabilityDenied, PermissionState, T3Answer, T3Capability, T3Prompt,
    },
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
    /// The working directory of user terminal 50, for the T3 cards.
    t3_cwd: Mutex<Option<PathBuf>>,
    integrations: Mutex<Vec<(String, TerminalIntegration)>>,
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

    fn t3_target(&self, _: &str, handle: TerminalHandle) -> Result<T3Target, TerminalError> {
        if handle != TerminalHandle(50) {
            return Err(TerminalError::NotFound);
        }
        Ok(T3Target {
            terminal: 500,
            origin: TerminalOrigin::User,
            title: "zsh".into(),
            cwd: self.t3_cwd.lock().unwrap().clone(),
        })
    }

    fn read(
        &self,
        _: &str,
        handle: TerminalHandle,
        scope: ReadScope,
        lines: u16,
        id: u64,
        sink: TerminalSink,
    ) {
        self.record(format!("read {} {scope:?} {lines} {id}", handle.0));
        sink(TerminalEvent::ReadDone(id, Ok("$ ls\nsrc".into())));
    }

    fn write(
        &self,
        _: &str,
        handle: TerminalHandle,
        text: String,
        submit: bool,
        id: u64,
        sink: TerminalSink,
    ) {
        self.record(format!("write {} {text} {submit} {id}", handle.0));
        sink(TerminalEvent::WriteDone(id, Ok(())));
    }

    fn set_integration(&self, ext: &str, integration: TerminalIntegration) {
        self.integrations
            .lock()
            .unwrap()
            .push((ext.into(), integration));
    }
}

/// A broker that grants T1, answers T2 from a map (`granted` when absent), and answers every
/// T3 card with the second field, keeping the cards in the third.
struct Broker(
    HashMap<&'static str, PermissionState>,
    T3Answer,
    Arc<Mutex<Vec<T3Prompt>>>,
);

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

    fn item_state(
        &self,
        extension_id: &str,
        item: &crate::permissions::ConsentItem,
    ) -> PermissionState {
        match self.0.get(item.key().as_str()) {
            Some(state) => *state,
            None => self.state(extension_id, item.capability()),
        }
    }

    fn t3_request(
        &self,
        _: &str,
        prompt: T3Prompt,
    ) -> futures::future::BoxFuture<'static, T3Answer> {
        self.2.lock().unwrap().push(prompt);
        futures::future::ready(self.1).boxed()
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, crate::journal::JournalKind)>>);

impl crate::journal::Journal for Recorder {
    fn record(&self, extension_id: &str, kind: crate::journal::JournalKind) {
        self.0.lock().unwrap().push((extension_id.into(), kind));
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
    journal: Arc<Recorder>,
    t3: T3Answer,
    cards: Arc<Mutex<Vec<T3Prompt>>>,
    /// In place of a [`Broker`] over `states`.
    broker: Option<Arc<dyn CapabilityCheck>>,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            host: Arc::default(),
            states: Vec::new(),
            journal: Arc::default(),
            t3: T3Answer::Once,
            cards: Arc::default(),
            broker: None,
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
                ui_ext::approvals::CAPABILITY_APPROVALS,
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
        capability_check: Some(setup.broker.clone().unwrap_or_else(|| {
            Arc::new(Broker(
                setup.states.iter().copied().collect(),
                setup.t3,
                setup.cards.clone(),
            )) as Arc<dyn CapabilityCheck>
        })),
        terminals: Some(setup.host.clone()),
        terminal_programs: Some(vec![terminal::TerminalProgramSpec {
            name: shell().into(),
            args: vec!["*".into()],
            path: None,
            reason: "runs the probe's shell".into(),
        }]),
        terminal_env: Some(env),
        terminal_profiles: Some(
            terminal::check_profiles(vec![terminal::TerminalProfileSpec {
                id: "probe".into(),
                name: "Probe".into(),
                icon: None,
                program: shell().into(),
                path: None,
                args: Vec::new(),
                cwd: Default::default(),
                account_tool: None,
            }])
            .unwrap(),
        ),
        package_dir: Some(PathBuf::from("pkg")),
        journal: Some(setup.journal.clone()),
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
    // The start is in the journal, with its arguments.
    let journal = setup.journal.0.lock().unwrap().clone();
    assert_eq!(
        journal,
        [(
            "probe-v0-4".to_string(),
            crate::journal::JournalKind::ProgramStarted {
                capability: terminal::CAPABILITY_CREATE.into(),
                program: shell().into(),
                args: vec!["hi".into()],
                cwd: None,
            }
        )]
    );
}

/// §C.2: a program turned off on the extension page is denied, though the capability is on.
#[gpui::test]
async fn test_v0_4_a_program_turned_off(cx: &mut TestAppContext) {
    let key: &'static str = Box::leak(format!("terminal:create#{}", shell()).into_boxed_str());
    let setup = Setup {
        states: vec![(key, PermissionState::Denied)],
        ..Setup::default()
    };
    let extension = load(&setup, cx).await.unwrap();
    assert_eq!(
        command(&extension, &format!("create {} hi", shell())).await,
        "denied"
    );
    assert!(setup.host.created.lock().unwrap().is_empty());
    assert!(setup.journal.0.lock().unwrap().is_empty());
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
                    capability_check: Some(Arc::new(Broker(
                        HashMap::new(),
                        T3Answer::Deny,
                        Arc::default(),
                    ))),
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
    let reply = extension
        .handle_event_v0_2(SURFACE, UiEvent::InputChanged(1, "node".into()))
        .await
        .unwrap();
    match reply.value {
        Update::Replace(nodes) => assert_eq!(
            nodes[0].props,
            Props::Terminal(ui_ext::v0_2::TerminalNodeProps { handle: 1 })
        ),
        other => panic!("not a replace: {other:?}"),
    }
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

/// Runs `command` from a click, a gesture.
async fn click(extension: &UiExtension, command: &str) -> String {
    let armed = extension
        .handle_event_v0_2(SURFACE, UiEvent::InputChanged(1, format!("arm {command}")))
        .await
        .unwrap();
    assert_eq!(report(&armed.value), "armed");
    let reply = extension
        .handle_event_v0_2(SURFACE, UiEvent::Click(1))
        .await
        .unwrap_or_else(|error| panic!("{command}: {error:#}"));
    report(&reply.value)
}

/// The next `count` reports `handle-io` pushes.
async fn pushed(
    extension: &UiExtension,
    events: &mut (impl futures::Stream<Item = crate::network::IoEvent> + Unpin),
    count: usize,
) -> Vec<String> {
    let mut seen = Vec::new();
    while seen.len() < count {
        let event = events.next().await.unwrap();
        let reply = extension.handle_io(event).await.unwrap();
        seen.extend(reply.effects.iter().filter_map(|effect| match effect {
            HostEffect::Push { update, .. } => Some(report(update)),
            _ => None,
        }));
    }
    seen
}

fn t3_setup(t3: T3Answer) -> Setup {
    let mut setup = Setup {
        t3,
        ..Setup::default()
    };
    setup
        .capabilities
        .extend([terminal::CAPABILITY_READ, terminal::CAPABILITY_WRITE]);
    setup
}

fn t3_calls(host: &FakeHost) -> Vec<String> {
    host.calls()
        .into_iter()
        .filter(|call| call.starts_with("read ") || call.starts_with("write "))
        .collect()
}

#[gpui::test]
async fn test_v0_4_t3_requests(cx: &mut TestAppContext) {
    let setup = t3_setup(T3Answer::Once);
    let extension = load(&setup, cx).await.unwrap();
    let mut events = extension.take_io_events().unwrap();

    // Without a gesture, past 500 lines, or with control characters: refused at the call.
    assert_eq!(command(&extension, "read 50").await, "not-permitted");
    assert_eq!(command(&extension, "write 50 ls").await, "not-permitted");
    assert_eq!(click(&extension, "read 50 501").await, "invalid-argument");
    assert_eq!(click(&extension, "read 50 0").await, "invalid-argument");
    assert_eq!(
        click(&extension, "write 50 \u{1b}[A").await,
        "invalid-argument"
    );
    // A terminal the host does not share (another profile's, or none).
    assert_eq!(click(&extension, "read 7").await, "invalid-argument");

    // A card on the user's terminal, then the host's read; the journal has the size only.
    assert_eq!(click(&extension, "read 50 20").await, "1");
    assert_eq!(
        pushed(&extension, &mut events, 1).await,
        ["read-done 1 $ ls\nsrc"]
    );
    assert_eq!(click(&extension, "write-enter 50 git status").await, "2");
    assert_eq!(
        pushed(&extension, &mut events, 1).await,
        ["write-done 2 done"]
    );
    assert_eq!(
        *setup.cards.lock().unwrap(),
        [
            T3Prompt {
                capability: T3Capability::Read,
                terminal: 500,
                terminal_title: "zsh".into(),
                scope: Some((ReadScope::LastLines, 20)),
                text: None,
                submit: false,
            },
            T3Prompt {
                capability: T3Capability::Write,
                terminal: 500,
                terminal_title: "zsh".into(),
                scope: None,
                text: Some("git status".into()),
                submit: true,
            },
        ]
    );
    assert_eq!(
        t3_calls(&setup.host),
        ["read 50 LastLines 20 1", "write 50 git status true 2"]
    );
    let journal: Vec<_> = setup
        .journal
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|(_, kind)| kind.clone())
        .collect();
    assert_eq!(
        journal,
        [
            crate::journal::JournalKind::TerminalRead {
                terminal: "zsh".into(),
                scope: "last 20 lines".into(),
                bytes: 8,
            },
            crate::journal::JournalKind::TerminalWrite {
                terminal: "zsh".into(),
                text: "git status".into(),
                submit: true,
            },
        ]
    );

    // Its own terminal needs no card.
    assert_eq!(command(&extension, "pseudo own").await, "1");
    assert_eq!(click(&extension, "read-command 1").await, "3");
    assert_eq!(
        pushed(&extension, &mut events, 1).await,
        ["read-done 3 $ ls\nsrc"]
    );
    assert_eq!(setup.cards.lock().unwrap().len(), 2);
}

#[gpui::test]
async fn test_v0_4_t3_denied_and_untrusted(cx: &mut TestAppContext) {
    let setup = t3_setup(T3Answer::Deny);
    let extension = load(&setup, cx).await.unwrap();
    let mut events = extension.take_io_events().unwrap();
    assert_eq!(click(&extension, "read 50").await, "1");
    assert_eq!(click(&extension, "write 50 ls").await, "2");
    let mut seen = pushed(&extension, &mut events, 2).await;
    seen.sort();
    assert_eq!(
        seen,
        ["read-done 1 error: denied", "write-done 2 error: denied"]
    );
    assert!(t3_calls(&setup.host).is_empty());
    assert!(setup.journal.0.lock().unwrap().is_empty());

    // Never in a restricted worktree, and no card for it.
    let root = tempfile::tempdir().unwrap();
    let setup = Setup {
        workspace: Some(vec![WorktreeRoot {
            id: 1,
            root: root.path().into(),
            trusted: false,
        }]),
        ..t3_setup(T3Answer::Once)
    };
    *setup.host.t3_cwd.lock().unwrap() = Some(root.path().join("src"));
    let extension = load(&setup, cx).await.unwrap();
    assert_eq!(click(&extension, "read 50").await, "untrusted-worktree");
    assert!(setup.cards.lock().unwrap().is_empty());

    // Not declared.
    let extension = load(&Setup::default(), cx).await.unwrap();
    assert_eq!(click(&extension, "read 50").await, "not-permitted");
}

#[gpui::test]
async fn test_v0_4_approvals(cx: &mut TestAppContext) {
    use ui_ext::approvals::{ApprovalOption, ApprovalRequest, MAX_OPEN_APPROVALS};

    let extension = load(&Setup::default(), cx).await.unwrap();
    let reply = extension
        .handle_event_v0_2(
            SURFACE,
            UiEvent::InputChanged(1, "ask Codex run-tests once deny".into()),
        )
        .await
        .unwrap();
    assert_eq!(report(&reply.value), "1");
    assert_eq!(
        reply.effects,
        [HostEffect::AskApproval {
            id: 1,
            request: ApprovalRequest {
                tool: Some("Codex".into()),
                title: "run-tests".into(),
                detail: "detail".into(),
                options: vec![ApprovalOption::AllowOnce, ApprovalOption::Deny],
            },
        }]
    );
    // The text rules, and no options.
    assert_eq!(
        command(&extension, "ask - t once once").await,
        "invalid-argument"
    );
    assert_eq!(command(&extension, "ask - t").await, "invalid-argument");
    // Four open at most.
    for id in 2..=MAX_OPEN_APPROVALS {
        assert_eq!(command(&extension, "ask - t always").await, id.to_string());
    }
    assert_eq!(command(&extension, "ask - t deny").await, "limit-exceeded");

    // An answer frees the slot and reaches `handle-io`; a second one for the same card, or one
    // for a card never asked, is dropped.
    let mut events = extension.take_io_events().unwrap();
    extension
        .answer_approval(1, Some(ApprovalOption::AllowOnce))
        .await;
    extension
        .answer_approval(1, Some(ApprovalOption::Deny))
        .await;
    extension.answer_approval(99, None).await;
    extension.answer_approval(2, None).await;
    let mut seen = Vec::new();
    for _ in 0..4 {
        let event = events.next().await.unwrap();
        let reply = extension.handle_io(event).await.unwrap();
        seen.extend(reply.effects.iter().filter_map(|effect| match effect {
            HostEffect::Push { update, .. } => Some(report(update)),
            _ => None,
        }));
    }
    assert_eq!(seen, ["approval 1 once", "approval 2 none"]);
    assert_eq!(command(&extension, "ask - t deny").await, "5");
    assert_eq!(command(&extension, "ask - t deny").await, "6");
    assert_eq!(command(&extension, "ask - t deny").await, "limit-exceeded");
}

#[gpui::test]
async fn test_v0_4_approvals_need_the_capability(cx: &mut TestAppContext) {
    let mut setup = Setup::default();
    setup
        .capabilities
        .retain(|capability| *capability != ui_ext::approvals::CAPABILITY_APPROVALS);
    let extension = load(&setup, cx).await.unwrap();
    assert_eq!(command(&extension, "ask - t once").await, "not-permitted");
}

#[test]
fn test_v0_4_icons_match_the_wit() {
    let mapped: Vec<ui_ext::v0_2::Icon> = fyx_ui_v0_4::WIT_ICONS
        .iter()
        .map(|&icon| icon.into())
        .collect();
    assert_eq!(mapped, ui_ext::v0_2::Icon::ALL);
}

/// Waits, up to `millis`, for something the tokio side does (an instance's store is dropped there).
fn wait_for(cx: &mut TestAppContext, millis: u64, done: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(millis);
    while !done() && std::time::Instant::now() < deadline {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

/// P2.4: when the registry starts an instance, the host gets the profiles and env as granted.
/// The instance it replaced leaves the new one's integration and terminals alone when dropped;
/// the last instance's drop empties the integration and closes the terminals.
#[gpui::test]
async fn test_v0_4_terminal_integration(cx: &mut TestAppContext) {
    let mut capabilities = Setup::default().capabilities;
    capabilities.extend([terminal::CAPABILITY_PROFILES, "package:files"]);
    let setup = Setup {
        capabilities,
        ..Setup::default()
    };
    let integrations = || setup.host.integrations.lock().unwrap().clone();
    let old = load(&setup, cx).await.unwrap();
    assert!(
        integrations().is_empty(),
        "nothing before the registry starts it"
    );
    old.activate_terminals();
    let [(ext, sent)] = integrations().try_into().unwrap();
    assert_eq!(ext, "probe-v0-4");
    assert_eq!(sent.profiles.len(), 1);
    assert_eq!(sent.profiles[0].id, "probe");
    assert_eq!(sent.env.unwrap().vars[0].0, "PROBE_HOME");
    assert!(sent.env_all_new, "the broker grants the item");
    assert_eq!(sent.package_dir, Some(PathBuf::from("pkg")));

    let new = load(&setup, cx).await.unwrap();
    new.activate_terminals();
    assert_eq!(integrations().len(), 2);
    drop(old);
    // Give the old instance's store time to go; nothing reaches the host for it.
    wait_for(cx, 500, || false);
    assert_eq!(integrations().len(), 2, "{:?}", setup.host.calls());
    assert!(
        !setup
            .host
            .calls()
            .contains(&"close-all probe-v0-4".to_string())
    );

    drop(new);
    wait_for(cx, 10_000, || {
        setup
            .host
            .calls()
            .contains(&"close-all probe-v0-4".to_string())
    });
    assert!(
        setup
            .host
            .calls()
            .contains(&"close-all probe-v0-4".to_string())
    );
    assert_eq!(
        integrations().last().unwrap().1,
        TerminalIntegration::default()
    );
}

/// P2.4: only what is granted goes; undeclared tables never do.
#[gpui::test]
async fn test_v0_4_terminal_integration_follows_the_grants(cx: &mut TestAppContext) {
    let setup = Setup {
        states: vec![("terminal:env#all-new", PermissionState::Denied)],
        ..Setup::default()
    };
    let extension = load(&setup, cx).await.unwrap();
    extension.activate_terminals();
    let sent = setup.host.integrations.lock().unwrap()[0].1.clone();
    assert!(
        sent.profiles.is_empty(),
        "terminal:profiles is not declared"
    );
    assert!(sent.package_dir.is_none(), "package:files is not declared");
    assert!(sent.env.is_some());
    assert!(!sent.env_all_new);

    let denied = Setup {
        states: vec![(terminal::CAPABILITY_ENV, PermissionState::Denied)],
        ..Setup::default()
    };
    let extension = load(&denied, cx).await.unwrap();
    extension.activate_terminals();
    let sent = denied.host.integrations.lock().unwrap()[0].1.clone();
    assert_eq!(sent, TerminalIntegration::default());
}

/// A broker whose states change while the instance runs, and which tells it.
#[derive(Default)]
struct ChangingBroker {
    denied: Mutex<Vec<String>>,
    listeners: Mutex<Vec<futures::channel::mpsc::UnboundedSender<(String, PermissionState)>>>,
}

impl ChangingBroker {
    fn deny(&self, capability: &str) {
        self.denied.lock().unwrap().push(capability.into());
        for listener in self.listeners.lock().unwrap().iter() {
            listener
                .unbounded_send((capability.into(), PermissionState::Denied))
                .ok();
        }
    }
}

impl CapabilityCheck for ChangingBroker {
    fn check(&self, _: &str, _: &str) -> Result<(), CapabilityDenied> {
        Ok(())
    }

    fn state(&self, _: &str, capability: &str) -> PermissionState {
        if self.denied.lock().unwrap().iter().any(|c| c == capability) {
            PermissionState::Denied
        } else {
            PermissionState::Granted
        }
    }

    fn changes(
        &self,
        _: &str,
    ) -> futures::channel::mpsc::UnboundedReceiver<(String, PermissionState)> {
        let (sender, receiver) = futures::channel::mpsc::unbounded();
        self.listeners.lock().unwrap().push(sender);
        receiver
    }
}

/// P2.4: revoking `terminal:env` closes the terminals and sends the integration again, without
/// the env.
#[gpui::test]
async fn test_v0_4_terminal_integration_after_a_revocation(cx: &mut TestAppContext) {
    let broker = Arc::new(ChangingBroker::default());
    let setup = Setup {
        broker: Some(broker.clone()),
        ..Setup::default()
    };
    let extension = load(&setup, cx).await.unwrap();
    extension.activate_terminals();
    let integrations = || setup.host.integrations.lock().unwrap().clone();
    assert!(integrations()[0].1.env.is_some());
    broker.deny(terminal::CAPABILITY_ENV);
    wait_for(cx, 10_000, || integrations().len() == 2);
    assert!(integrations()[1].1.env.is_none(), "{:?}", integrations());
    assert!(
        setup
            .host
            .calls()
            .contains(&"close-all probe-v0-4".to_string())
    );
    drop(extension);
}
