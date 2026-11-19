//! The staged 0.4 world against `fixtures/probe_v0_4`: the `terminals` import over a fake
//! [`TerminalHost`], the `terminal` node, and the interfaces that still trap.

use std::{
    collections::HashMap,
    sync::{Mutex, atomic::AtomicU64},
};

use gpui::TestAppContext;
use ui_ext::{
    integration::{
        DiscoveryFileSpec, ListenerAuth, ListenerObserver, ListenerProtocol, ListenerSpec,
        OpenListener,
    },
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
    /// `[fyx_ui.terminal_env] vars`.
    env: Vec<(&'static str, &'static str)>,
    listeners: Vec<ListenerSpec>,
    discovery_files: Vec<DiscoveryFileSpec>,
    observer: Option<Arc<dyn ListenerObserver>>,
    context: Option<Arc<dyn ui_ext::context::ContextHost>>,
    review: Option<Arc<dyn ui_ext::review::ReviewHost>>,
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
            env: vec![("PROBE_HOME", "${package}/home")],
            listeners: Vec::new(),
            discovery_files: Vec::new(),
            observer: None,
            context: None,
            review: None,
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
            vars: setup
                .env
                .iter()
                .map(|(name, value)| (name.to_string(), value.to_string()))
                .collect(),
            scope: terminal::EnvScope::Profiles,
            reason: "the probe's home".into(),
        },
        &|name| {
            setup
                .listeners
                .iter()
                .find(|listener| listener.name == name)
                .map(|listener| listener.token_in_env)
        },
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
        listeners: Some(setup.listeners.clone()),
        discovery_files: Some(setup.discovery_files.clone()),
        listener_observer: setup.observer.clone(),
        context: setup.context.clone(),
        review: setup.review.clone(),
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
        .handle_event_v0_2(SURFACE, UiEvent::InputChanged(1, "relay".into()))
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

/// What the discovery-file writer hears, and the last it heard of each listener.
#[derive(Default)]
struct Observer {
    heard: Mutex<Vec<String>>,
    open: Mutex<HashMap<String, OpenListener>>,
}

impl Observer {
    fn heard(&self) -> Vec<String> {
        self.heard.lock().unwrap().clone()
    }

    fn open(&self, name: &str) -> OpenListener {
        self.open.lock().unwrap()[name].clone()
    }
}

impl ListenerObserver for Observer {
    fn opened(&self, extension_id: &str, listener: &OpenListener) {
        self.heard.lock().unwrap().push(format!(
            "opened {extension_id} {} {}",
            listener.name,
            listener.discovery_files.len()
        ));
        self.open
            .lock()
            .unwrap()
            .insert(listener.name.clone(), listener.clone());
    }

    fn closed(&self, extension_id: &str, listener: &str) {
        self.heard
            .lock()
            .unwrap()
            .push(format!("closed {extension_id} {listener}"));
    }
}

/// `ide` speaks MCP over streamable HTTP with a bearer token that may go into terminal env, and
/// has one discovery file; `hooks` is a raw `http` listener with a header token.
fn listening_setup(observer: &Arc<Observer>) -> Setup {
    let mut capabilities = Setup::default().capabilities;
    capabilities.extend([
        ui_ext::integration::CAPABILITY_NETWORK_LISTEN,
        ui_ext::integration::CAPABILITY_DISCOVERY_FILE,
    ]);
    Setup {
        capabilities,
        env: vec![
            ("IDE_PORT", "${listen.ide.port}"),
            ("IDE_TOKEN", "${listen.ide.token}"),
            ("HOOKS_PORT", "${listen.hooks.port}"),
        ],
        listeners: vec![
            ListenerSpec {
                name: "ide".into(),
                protocol: ListenerProtocol::McpStreamableHttp,
                auth: ListenerAuth {
                    bearer: true,
                    ..Default::default()
                },
                token_in_env: true,
                reason: "the IDE connection".into(),
            },
            ListenerSpec {
                name: "hooks".into(),
                protocol: ListenerProtocol::Http,
                auth: ListenerAuth {
                    header: Some("X-Probe-Token".into()),
                    ..Default::default()
                },
                token_in_env: false,
                reason: "hooks".into(),
            },
        ],
        discovery_files: vec![DiscoveryFileSpec {
            listener: "ide".into(),
            dir: "${tmp}/probe".into(),
            default_dir: None,
            name: "${port}.lock".into(),
            content: [("port".to_string(), "${port}".into())].into(),
            reason: "the IDE finds Fyx".into(),
        }],
        observer: Some(observer.clone()),
        ..Setup::default()
    }
}

/// One blocking HTTP/1.1 exchange on its own thread: the status, the head and the body.
fn http_exchange(port: u16, request: String) -> std::thread::JoinHandle<(u16, String, String)> {
    use std::io::{Read as _, Write as _};
    std::thread::spawn(move || {
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(20)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut seen = Vec::new();
        let mut byte = [0u8; 1];
        while !seen.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            seen.push(byte[0]);
        }
        let head = String::from_utf8(seen).unwrap().to_lowercase();
        let status = head[9..12].parse().unwrap();
        let length: usize = head
            .lines()
            .find_map(|line| line.strip_prefix("content-length: "))
            .map_or(0, |length| length.trim().parse().unwrap());
        let mut body = vec![0; length];
        stream.read_exact(&mut body).unwrap();
        (status, head, String::from_utf8(body).unwrap())
    })
}

fn mcp_post(port: u16, token: &str, session: Option<&str>, body: serde_json::Value) -> String {
    let body = body.to_string();
    let session = session.map_or(String::new(), |session| {
        format!("Mcp-Session-Id: {session}\r\n")
    });
    format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\n\
         {session}Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

/// The next io-event through `handle-io`, as the probe reports it.
async fn next_io_report(
    extension: &UiExtension,
    events: &mut futures::channel::mpsc::Receiver<crate::network::IoEvent>,
) -> String {
    let event = events.next().await.unwrap();
    let reply = extension.handle_io(event).await.unwrap();
    let effects = format!("{:?}", reply.effects);
    let start = effects.find("content: \"").unwrap() + "content: \"".len();
    let end = start + effects[start..].find("\",").unwrap();
    effects[start..end].replace("\\\"", "\"")
}

/// §B.7: listeners open when the registry starts the instance, with the granted discovery
/// files; tools registered before that apply then; an MCP `tools/call` and a raw request reach
/// `handle-io` and are answered by `respond` and `reply`; `${listen.*}` fills a created
/// terminal's env.
#[gpui::test]
async fn test_v0_4_listeners(cx: &mut TestAppContext) {
    let observer = Arc::new(Observer::default());
    let setup = listening_setup(&observer);
    let extension = load(&setup, cx).await.unwrap();
    let mut events = extension.take_io_events().unwrap();

    // Not started yet: tools are kept, nothing is open.
    assert_eq!(command(&extension, "tools ide openDiff").await, "done");
    assert_eq!(
        command(&extension, "tools hooks a").await,
        "invalid-argument"
    );
    assert_eq!(command(&extension, "tools nope a").await, "not-permitted");
    assert_eq!(command(&extension, "notify ide x/y").await, "unavailable");
    assert_eq!(extension.listener_port("ide"), None);
    assert!(observer.heard().is_empty());

    extension.activate_terminals();
    assert_eq!(
        observer.heard(),
        ["opened probe-v0-4 ide 1", "opened probe-v0-4 hooks 0"]
    );
    let ide = observer.open("ide");
    let hooks = observer.open("hooks");
    assert_eq!(extension.listener_port("ide"), Some(ide.port));
    assert_eq!(command(&extension, "notify ide x/y").await, "done");

    // `${listen.*}` in the env of a created terminal.
    assert_eq!(
        command(&extension, &format!("create {} hi", shell())).await,
        "1"
    );
    let env = setup.host.created.lock().unwrap()[0].env.clone();
    for var in [
        ("IDE_PORT".to_string(), ide.port.to_string()),
        ("IDE_TOKEN".to_string(), ide.token.clone()),
        ("HOOKS_PORT".to_string(), hooks.port.to_string()),
    ] {
        assert!(env.contains(&var), "{var:?} in {env:?}");
    }

    // MCP: the tool registered before the start is listed, and a call reaches `handle-io`.
    let (status, head, _) = http_exchange(
        ide.port,
        mcp_post(
            ide.port,
            &ide.token,
            None,
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        ),
    )
    .join()
    .unwrap();
    assert_eq!(status, 200);
    let session = head
        .lines()
        .find_map(|line| line.strip_prefix("mcp-session-id: "))
        .unwrap()
        .trim()
        .to_owned();
    let (_, _, list) = http_exchange(
        ide.port,
        mcp_post(
            ide.port,
            &ide.token,
            Some(&session),
            serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        ),
    )
    .join()
    .unwrap();
    assert!(list.contains("openDiff"), "{list}");
    let call = http_exchange(
        ide.port,
        mcp_post(
            ide.port,
            &ide.token,
            Some(&session),
            serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
                "params":{"name":"openDiff","arguments":{"a":1}}}),
        ),
    );
    let report = next_io_report(&extension, &mut events).await;
    let words: Vec<&str> = report.split(' ').collect();
    assert_eq!(
        (words[0], words[2], words[4], words[5]),
        ("mcp-call", "ide", "openDiff", "{\"a\":1}"),
        "{report}"
    );
    assert_eq!(
        command(
            &extension,
            &format!("respond {} {{\"content\":[],\"isError\":false}}", words[1])
        )
        .await,
        "done"
    );
    let (status, _, body) = call.join().unwrap();
    assert_eq!(status, 200);
    let body: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["id"], 3);
    assert_eq!(body["result"]["isError"], false);

    // A raw request, answered by `reply`.
    let request = http_exchange(
        hooks.port,
        format!(
            "POST /hook HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nX-Probe-Token: {}\r\n\
             Content-Length: 2\r\n\r\nhi",
            hooks.port, hooks.token
        ),
    );
    let report = next_io_report(&extension, &mut events).await;
    let words: Vec<&str> = report.split(' ').collect();
    assert_eq!(
        (words[0], words[2], words[3], words[4]),
        ("request", "hooks", "/hook", "hi"),
        "{report}"
    );
    assert_eq!(
        command(&extension, &format!("reply {} 201 ok", words[1])).await,
        "done"
    );
    let (status, _, body) = request.join().unwrap();
    assert_eq!((status, body.as_str()), (201, "ok"));
}

/// A broker whose T2 states and items change while the instance runs, and which tells it.
#[derive(Default)]
struct GrantingBroker {
    states: Mutex<HashMap<String, PermissionState>>,
    senders: Mutex<Vec<futures::channel::mpsc::UnboundedSender<(String, PermissionState)>>>,
}

impl GrantingBroker {
    fn set(&self, key: &str, state: PermissionState) {
        self.states.lock().unwrap().insert(key.into(), state);
        for sender in self.senders.lock().unwrap().iter() {
            sender.unbounded_send((key.into(), state)).ok();
        }
    }
}

impl CapabilityCheck for GrantingBroker {
    fn check(&self, _: &str, _: &str) -> Result<(), CapabilityDenied> {
        Ok(())
    }

    fn state(&self, _: &str, capability: &str) -> PermissionState {
        self.states
            .lock()
            .unwrap()
            .get(capability)
            .copied()
            .unwrap_or(PermissionState::Granted)
    }

    fn item_state(
        &self,
        extension_id: &str,
        item: &crate::permissions::ConsentItem,
    ) -> PermissionState {
        let state = self.states.lock().unwrap().get(&item.key()).copied();
        state.unwrap_or_else(|| self.state(extension_id, item.capability()))
    }

    fn changes(
        &self,
        _: &str,
    ) -> futures::channel::mpsc::UnboundedReceiver<(String, PermissionState)> {
        let (sender, receiver) = futures::channel::mpsc::unbounded();
        self.senders.lock().unwrap().push(sender);
        receiver
    }
}

/// §B.7, §B.8, §C.2: listeners follow their grants and their discovery files' grants; a
/// reloaded instance takes over (the old one closes first) and the last one closes them.
#[gpui::test]
async fn test_v0_4_listener_grants_and_reloads(cx: &mut TestAppContext) {
    let observer = Arc::new(Observer::default());
    let broker = Arc::new(GrantingBroker::default());
    broker.set("network:listen#hooks", PermissionState::Denied);
    let setup = Setup {
        broker: Some(broker.clone()),
        ..listening_setup(&observer)
    };
    let extension = load(&setup, cx).await.unwrap();
    extension.activate_terminals();
    assert_eq!(observer.heard(), ["opened probe-v0-4 ide 1"]);
    assert_eq!(command(&extension, "notify hooks x").await, "denied");
    let port = extension.listener_port("ide").unwrap();
    let heard = |count: usize| {
        let observer = observer.clone();
        move || observer.heard().len() == count
    };

    // The discovery file's grant goes: the listener stays, with no file.
    broker.set("fs:discovery-file#${port}.lock", PermissionState::Denied);
    wait_for(cx, 10_000, heard(2));
    assert_eq!(observer.heard()[1], "opened probe-v0-4 ide 0");
    assert_eq!(extension.listener_port("ide"), Some(port));

    broker.set("network:listen#hooks", PermissionState::Granted);
    wait_for(cx, 10_000, heard(3));
    assert_eq!(observer.heard()[2], "opened probe-v0-4 hooks 0");

    // A revocation closes the listener; the extension hears `denied`.
    broker.set("network:listen#ide", PermissionState::Denied);
    wait_for(cx, 10_000, heard(4));
    assert_eq!(observer.heard()[3], "closed probe-v0-4 ide");
    assert_eq!(extension.listener_port("ide"), None);
    assert_eq!(command(&extension, "notify ide x/y").await, "denied");
    broker.set("network:listen#ide", PermissionState::Granted);
    wait_for(cx, 10_000, heard(5));
    assert_eq!(observer.heard()[4], "opened probe-v0-4 ide 0");

    // The whole capability.
    broker.set("network:listen", PermissionState::Denied);
    wait_for(cx, 10_000, heard(7));
    assert_eq!(
        observer.heard()[5..],
        ["closed probe-v0-4 ide", "closed probe-v0-4 hooks"]
    );
    broker.set("network:listen", PermissionState::Granted);
    wait_for(cx, 10_000, heard(9));

    // A reload: the old instance closes before the new one opens, and leaves it alone after.
    let reloaded = load(&setup, cx).await.unwrap();
    reloaded.activate_terminals();
    assert_eq!(
        observer.heard()[9..],
        [
            "closed probe-v0-4 ide",
            "closed probe-v0-4 hooks",
            "opened probe-v0-4 ide 0",
            "opened probe-v0-4 hooks 0",
        ]
    );
    drop(extension);
    cx.run_until_parked();
    assert_eq!(observer.heard().len(), 13);
    assert!(reloaded.listener_port("ide").is_some());
    drop(reloaded);
    wait_for(cx, 10_000, heard(15));
    assert_eq!(
        observer.heard()[13..],
        ["closed probe-v0-4 ide", "closed probe-v0-4 hooks"]
    );
}

/// The editor side of `context`: keeps the sink, records `open` and `watch`; `.env` is private.
#[derive(Default)]
struct FakeEditor {
    sink: Mutex<Option<ui_ext::context::ContextSink>>,
    calls: Mutex<Vec<String>>,
    terminal_focused: std::sync::atomic::AtomicBool,
}

impl FakeEditor {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn send(&self, context: ui_ext::context::EditorContext) {
        let sink = self.sink.lock().unwrap().clone().unwrap();
        sink(context);
    }
}

impl ui_ext::context::ContextHost for FakeEditor {
    fn watch(&self, ext: &str, sink: Option<ui_ext::context::ContextSink>) {
        self.calls
            .lock()
            .unwrap()
            .push(format!("watch {ext} {}", sink.is_some()));
        *self.sink.lock().unwrap() = sink;
    }

    fn open(
        &self,
        _: &str,
        path: ui_ext::process::FileRef,
        _: Option<ui_ext::edit::TextRange>,
        focus: bool,
    ) -> Result<(), String> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("open {} {focus}", path.path));
        Ok(())
    }

    fn is_private(&self, absolute: &Path) -> bool {
        absolute.file_name().is_some_and(|name| name == ".env")
    }

    fn terminal_focused(&self, _: &str) -> bool {
        self.terminal_focused
            .load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// A fresh worktree with `a.rs` and a private `.env`.
fn context_worktree(name: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("fyx-probe-context-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.rs"), "fn main() {}").unwrap();
    std::fs::write(root.join(".env"), "SECRET=1").unwrap();
    root
}

fn context_setup(editor: &Arc<FakeEditor>, root: &Path) -> Setup {
    let mut capabilities = Setup::default().capabilities;
    capabilities.extend([
        ui_ext::context::CAPABILITY_CONTEXT_EDITOR,
        ui_ext::context::CAPABILITY_WORKSPACE_OPEN,
    ]);
    Setup {
        capabilities,
        context: Some(editor.clone()),
        workspace: Some(vec![WorktreeRoot {
            id: 1,
            root: root.to_path_buf(),
            trusted: true,
        }]),
        ..Setup::default()
    }
}

fn file(path: &str) -> ui_ext::process::FileRef {
    ui_ext::process::FileRef {
        worktree: Some(1),
        path: path.into(),
    }
}

fn selection(text: Option<String>) -> Option<(ui_ext::edit::TextRange, Option<String>)> {
    let at = ui_ext::edit::Position { line: 0, column: 0 };
    Some((ui_ext::edit::TextRange { start: at, end: at }, text))
}

/// §B.11: the feed carries worktree files only, never a private file's text, and at most
/// 64 KiB of selection; a tool's paths and URIs (both Windows spellings Claude Code sends) turn
/// into `file-ref`s and back; `open` keeps `focus` only with a gesture or a recent listener
/// request while a terminal has focus.
#[gpui::test]
async fn test_v0_4_editor_context(cx: &mut TestAppContext) {
    let editor = Arc::new(FakeEditor::default());
    let root = context_worktree("feed");
    let setup = context_setup(&editor, &root);
    let extension = load(&setup, cx).await.unwrap();
    assert!(extension.context_available());
    let mut events = extension.take_io_events().unwrap();

    assert_eq!(command(&extension, "watch on").await, "done");
    editor.send(ui_ext::context::EditorContext {
        active: Some(file("a.rs")),
        open: vec![
            file("a.rs"),
            ui_ext::process::FileRef {
                worktree: Some(9),
                path: "elsewhere.rs".into(),
            },
            file(".env"),
        ],
        selection: selection(Some("x".repeat(70_000))),
        trusted: Some(true),
    });
    assert_eq!(
        next_io_report(&extension, &mut events).await,
        format!(
            "context a.rs a.rs,.env {} true",
            ui_ext::context::MAX_SELECTION_BYTES
        )
    );
    editor.send(ui_ext::context::EditorContext {
        active: Some(file(".env")),
        open: vec![file(".env")],
        selection: selection(Some("SECRET=1".into())),
        trusted: Some(true),
    });
    assert_eq!(
        next_io_report(&extension, &mut events).await,
        "context .env .env none true"
    );

    // Paths in, `file-ref`s out: a plain path, `file:///C:/…` and `file://C:\…` alike.
    let plain = root.join("a.rs").to_string_lossy().into_owned();
    let forward = plain.replace('\\', "/");
    let slashed = format!("file:///{}", forward.trim_start_matches('/'));
    let raw = format!("file://{plain}");
    for input in [&plain, &slashed, &raw] {
        assert_eq!(
            command(&extension, &format!("to-ref {input}")).await,
            "1:a.rs",
            "{input}"
        );
    }
    let private = root.join(".env").to_string_lossy().into_owned();
    assert_eq!(
        command(&extension, &format!("to-ref {private}")).await,
        "not-permitted"
    );
    let outside = std::env::temp_dir()
        .join("x.rs")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        command(&extension, &format!("to-ref {outside}")).await,
        "invalid-argument"
    );
    assert_eq!(command(&extension, "to-ref a.rs").await, "invalid-argument");
    assert_eq!(command(&extension, "abs 1 a.rs").await, plain);
    assert_eq!(command(&extension, "abs - a.rs").await, plain);
    assert_eq!(command(&extension, "abs 1 .env").await, "not-permitted");
    assert_eq!(command(&extension, "abs 7 a.rs").await, "invalid-argument");

    // Focus needs a gesture, or a recent listener request while a terminal has focus.
    assert_eq!(command(&extension, "open 1 a.rs 1").await, "done");
    assert_eq!(click(&extension, "open 1 a.rs 1").await, "done");
    assert_eq!(
        command(&extension, "open 1 nope.rs 0").await,
        "invalid-argument"
    );
    editor
        .terminal_focused
        .store(true, std::sync::atomic::Ordering::SeqCst);
    // No listener has had a request.
    assert_eq!(command(&extension, "open 1 a.rs 1").await, "done");
    assert_eq!(command(&extension, "watch off").await, "done");
    assert_eq!(
        editor.calls(),
        [
            "watch probe-v0-4 true",
            "open a.rs false",
            "open a.rs true",
            "open a.rs false",
            "watch probe-v0-4 false",
        ]
    );
    std::fs::remove_dir_all(&root).ok();
}

/// Without the capabilities the calls are refused; a reloaded instance's watch survives the old
/// instance's drop.
#[gpui::test]
async fn test_v0_4_editor_context_rules_and_reloads(cx: &mut TestAppContext) {
    let editor = Arc::new(FakeEditor::default());
    let root = context_worktree("rules");
    let undeclared = Setup {
        capabilities: vec!["ui:panel"],
        ..context_setup(&editor, &root)
    };
    let extension = load(&undeclared, cx).await.unwrap();
    for refused in ["watch on", "open 1 a.rs 0", "abs 1 a.rs", "to-ref /a.rs"] {
        assert_eq!(
            command(&extension, refused).await,
            "not-permitted",
            "{refused}"
        );
    }
    drop(extension);

    let setup = context_setup(&editor, &root);
    let old = load(&setup, cx).await.unwrap();
    assert_eq!(command(&old, "watch on").await, "done");
    let new = load(&setup, cx).await.unwrap();
    assert_eq!(command(&new, "watch on").await, "done");
    drop(old);
    wait_for(cx, 2_000, || false);
    assert_eq!(
        editor.calls(),
        ["watch probe-v0-4 true", "watch probe-v0-4 true"]
    );
    drop(new);
    wait_for(cx, 10_000, || editor.calls().len() == 3);
    assert_eq!(editor.calls()[2], "watch probe-v0-4 false");
    std::fs::remove_dir_all(&root).ok();
}

/// The review bridge's side: keeps each proposal's sink, records the calls.
#[derive(Default)]
struct FakeReviewer {
    sinks: Mutex<HashMap<u64, ui_ext::review::ReviewSink>>,
    calls: Mutex<Vec<String>>,
}

impl FakeReviewer {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn call(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }

    /// What the user decided, or why the review ended; the sink of `id` hears it.
    fn decide(&self, id: u64, result: Result<(ui_ext::review::Decision, &str), &str>) {
        let sink = self.sinks.lock().unwrap().get(&id).cloned().unwrap();
        sink(
            id,
            result
                .map(|(decision, text)| ui_ext::review::ReviewOutcome {
                    decision,
                    final_content: text.into(),
                })
                .map_err(String::from),
        );
    }
}

impl ui_ext::review::ReviewHost for FakeReviewer {
    fn propose(
        &self,
        _: &str,
        id: u64,
        proposal: ui_ext::review::Proposal,
        sink: ui_ext::review::ReviewSink,
    ) -> Result<(), String> {
        if proposal.path.path == "refused.rs" {
            return Err("the diff does not apply".into());
        }
        let kind = match proposal.change {
            ui_ext::review::Proposed::NewContent(_) => "new",
            ui_ext::review::Proposed::UnifiedDiff(_) => "diff",
        };
        self.call(format!(
            "propose {} {kind} {} {}",
            proposal.path.path, proposal.apply, proposal.session
        ));
        self.sinks.lock().unwrap().insert(id, sink);
        Ok(())
    }

    fn withdraw(&self, _: &str, id: u64) -> Result<String, String> {
        self.call("withdraw".into());
        Ok(format!("current {id}"))
    }

    fn begin_turn(&self, ext: &str, session: &str) {
        self.call(format!("begin {ext} {session}"));
    }

    fn end_turn(&self, _: &str, session: &str, changed: Vec<ui_ext::process::FileRef>) {
        let paths: Vec<String> = changed.into_iter().map(|file| file.path).collect();
        self.call(format!("end {session} {}", paths.join(",")));
    }

    fn close(&self, ext: &str, ids: &[u64]) {
        self.call(format!("close {ext} {}", ids.len()));
    }
}

fn review_setup(reviewer: &Arc<FakeReviewer>, root: &Path) -> Setup {
    let mut capabilities = Setup::default().capabilities;
    capabilities.push(ui_ext::review::CAPABILITY_REVIEW_PROPOSE);
    Setup {
        capabilities,
        review: Some(reviewer.clone()),
        workspace: Some(vec![WorktreeRoot {
            id: 1,
            root: root.to_path_buf(),
            trusted: true,
        }]),
        ..Setup::default()
    }
}

/// The next `review-decided`, as the probe reports it; other io-events (permission changes) are
/// skipped.
async fn next_decided(
    extension: &UiExtension,
    events: &mut futures::channel::mpsc::Receiver<crate::network::IoEvent>,
) -> String {
    loop {
        let event = events.next().await.unwrap();
        if !matches!(event, crate::network::IoEvent::ReviewDecided(..)) {
            continue;
        }
        let reply = extension.handle_io(event).await.unwrap();
        let effects = format!("{:?}", reply.effects);
        let start = effects.find("content: \"").unwrap() + "content: \"".len();
        let end = start + effects[start..].find("\",").unwrap();
        return effects[start..end].replace("\\\"", "\"");
    }
}

async fn propose(extension: &UiExtension, words: &str) -> u64 {
    let reply = command(extension, &format!("propose {words}")).await;
    reply
        .parse()
        .unwrap_or_else(|_| panic!("propose {words}: {reply}"))
}

/// §B.10: each outcome reaches `handle-io` once, never after `withdraw`, which returns the
/// current text; paths are worktree files, never protected or private; sizes, labels and the
/// open limit hold; turns reach the bridge; the instance's end closes its open reviews.
#[gpui::test]
async fn test_v0_4_review(cx: &mut TestAppContext) {
    use ui_ext::review::{Decision, MAX_OPEN_PROPOSALS, MAX_PROPOSAL_BYTES};
    let reviewer = Arc::new(FakeReviewer::default());
    let root = context_worktree("review");
    let extension = load(&review_setup(&reviewer, &root), cx).await.unwrap();
    assert!(extension.review_available());
    let mut events = extension.take_io_events().unwrap();

    let first = propose(&extension, "1 a.rs new 1 s1 fn main() { run() }").await;
    reviewer.decide(first, Ok((Decision::Partial, "fn main() {}")));
    assert_eq!(
        next_io_report(&extension, &mut events).await,
        format!("decided {first} partial fn main() {{}}")
    );
    // Once only: a second outcome, or one after `withdraw`, goes nowhere.
    reviewer.decide(first, Ok((Decision::Accepted, "again")));
    let second = propose(&extension, "- a.rs diff 0 s1 @@ -1 +1 @@").await;
    assert_eq!(
        command(&extension, &format!("withdraw {second}")).await,
        format!("withdrawn current {second}")
    );
    assert_eq!(
        command(&extension, &format!("withdraw {second}")).await,
        "invalid-argument"
    );
    assert_eq!(
        command(&extension, &format!("withdraw {first}")).await,
        "invalid-argument"
    );
    reviewer.decide(second, Ok((Decision::Accepted, "late")));
    let third = propose(&extension, "1 new.rs new 0 s2 fresh").await;
    reviewer.decide(third, Err("the file went away"));
    assert_eq!(
        next_io_report(&extension, &mut events).await,
        format!("decided {third} error: the file went away")
    );

    for (words, code) in [
        ("1 missing.rs diff 0 s @@", "invalid-argument"),
        ("- missing.rs new 0 s x", "invalid-argument"),
        ("7 a.rs new 0 s x", "invalid-argument"),
        ("1 . new 0 s x", "invalid-argument"),
        ("1 ../a.rs new 0 s x", "invalid-argument"),
        ("1 .env new 0 s x", "not-permitted"),
        ("1 .git/config new 0 s x", "not-permitted"),
        ("1 .vscode/tasks.json new 0 s x", "not-permitted"),
        ("1 refused.rs new 0 s x", "unavailable"),
    ] {
        assert_eq!(
            command(&extension, &format!("propose {words}")).await,
            code,
            "{words}"
        );
    }
    assert_eq!(
        command(
            &extension,
            &format!("propose-big a.rs {}", MAX_PROPOSAL_BYTES + 1)
        )
        .await,
        "limit-exceeded"
    );
    assert_eq!(
        command(&extension, "propose-title a.rs 257").await,
        "invalid-argument"
    );
    assert!(
        command(&extension, "propose-title a.rs 256")
            .await
            .parse::<u64>()
            .is_ok()
    );
    // One is open now; fill up to the limit.
    for _ in 1..MAX_OPEN_PROPOSALS {
        propose(&extension, "1 a.rs new 0 s x").await;
    }
    assert_eq!(
        command(&extension, "propose 1 a.rs new 0 s x").await,
        "limit-exceeded"
    );

    assert_eq!(command(&extension, "begin s1").await, "done");
    assert_eq!(command(&extension, "end s1 1:a.rs 1:new.rs").await, "done");
    assert_eq!(
        command(&extension, "end s1 7:a.rs").await,
        "invalid-argument"
    );
    assert_eq!(
        command(&extension, "end s1 1:../x").await,
        "invalid-argument"
    );

    drop(extension);
    wait_for(cx, 10_000, || {
        reviewer
            .calls()
            .last()
            .is_some_and(|call| call.starts_with("close"))
    });
    let calls = reviewer.calls();
    assert_eq!(
        calls[..6],
        [
            "propose a.rs new true s1",
            "propose a.rs diff false s1",
            "withdraw",
            "propose new.rs new false s2",
            "propose a.rs new false s",
            "propose a.rs new false s",
        ]
    );
    assert_eq!(
        calls[calls.len() - 3..],
        [
            "begin probe-v0-4 s1".to_string(),
            "end s1 a.rs,new.rs".to_string(),
            format!("close probe-v0-4 {MAX_OPEN_PROPOSALS}"),
        ]
    );
    std::fs::remove_dir_all(&root).ok();
}

/// Without the capability or a bridge the calls are refused; revoking `review:propose` closes
/// the open reviews, and their outcomes go nowhere; a reloaded instance's reviews survive the
/// old instance's drop.
#[gpui::test]
async fn test_v0_4_review_rules_revocation_and_reloads(cx: &mut TestAppContext) {
    use ui_ext::review::Decision;
    let reviewer = Arc::new(FakeReviewer::default());
    let root = context_worktree("review-rules");
    let undeclared = Setup {
        capabilities: vec!["ui:panel"],
        ..review_setup(&reviewer, &root)
    };
    let extension = load(&undeclared, cx).await.unwrap();
    for refused in ["propose 1 a.rs new 0 s x", "withdraw 1", "begin s", "end s"] {
        assert_eq!(
            command(&extension, refused).await,
            "not-permitted",
            "{refused}"
        );
    }
    drop(extension);
    let unbridged = Setup {
        review: None,
        ..review_setup(&reviewer, &root)
    };
    let extension = load(&unbridged, cx).await.unwrap();
    assert!(!extension.review_available());
    assert_eq!(
        command(&extension, "propose 1 a.rs new 0 s x").await,
        "unavailable"
    );
    drop(extension);
    assert!(reviewer.calls().is_empty());

    let broker = Arc::new(GrantingBroker::default());
    let setup = Setup {
        broker: Some(broker.clone()),
        ..review_setup(&reviewer, &root)
    };
    let extension = load(&setup, cx).await.unwrap();
    let mut events = extension.take_io_events().unwrap();
    let revoked = propose(&extension, "1 a.rs new 0 s x").await;
    broker.set(
        ui_ext::review::CAPABILITY_REVIEW_PROPOSE,
        PermissionState::Denied,
    );
    wait_for(cx, 10_000, || {
        reviewer
            .calls()
            .last()
            .is_some_and(|call| call == "close probe-v0-4 1")
    });
    reviewer.decide(revoked, Ok((Decision::Accepted, "x")));
    // (Refusing calls while it is off is the broker's `check`, as for every T1 capability.)
    broker.set(
        ui_ext::review::CAPABILITY_REVIEW_PROPOSE,
        PermissionState::Granted,
    );
    let after = propose(&extension, "1 a.rs new 0 s y").await;
    reviewer.decide(after, Ok((Decision::Rejected, "x")));
    assert_eq!(
        next_decided(&extension, &mut events).await,
        format!("decided {after} rejected x")
    );

    // Reload: the old instance's drop closes only its own reviews.
    propose(&extension, "1 a.rs new 0 s old").await;
    let new = load(&setup, cx).await.unwrap();
    let mut new_events = new.take_io_events().unwrap();
    let kept = propose(&new, "1 a.rs new 0 s z").await;
    assert!(kept > after, "ids are unique in the process");
    drop(extension);
    wait_for(cx, 10_000, || {
        reviewer
            .calls()
            .last()
            .is_some_and(|call| call == "close probe-v0-4 1")
            && reviewer.calls().len() == 6
    });
    reviewer.decide(kept, Ok((Decision::Accepted, "z")));
    assert_eq!(
        next_decided(&new, &mut new_events).await,
        format!("decided {kept} accepted z")
    );
    drop(new);
    std::fs::remove_dir_all(&root).ok();
}
