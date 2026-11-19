//! The fyx:ui 0.4 `terminals` import over [`TerminalHost`] (docs/fyx/wit-0.4/design.md §B.1–§B.5,
//! §B.11). The split agreed with W1 is at the top of `ui_ext::terminal`: this side checks the
//! capabilities, the declared programs and their argument patterns, untrusted worktrees, per-call
//! sizes, the per-extension caps and which handles are the extension's own; the host enforces
//! rates, sanitises output, owns the process trees and allocates handles.
//!
//! The host's events reach `handle-io` through a [`TerminalSink`] that queues onto the instance's
//! io channel without blocking ([`TerminalsHandle::sink`]).

use std::{
    collections::HashMap,
    ffi::OsStr,
    path::PathBuf,
    sync::{
        Arc, LazyLock, Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
};

use futures::{StreamExt as _, channel::mpsc};
use ui_ext::{
    process::{SpawnRequest, check_spawn},
    terminal::{
        self, CreateCommand, PseudoRequest, TemplateContext, TerminalEnv, TerminalError,
        TerminalEvent, TerminalHandle, TerminalHost, TerminalIntegration, TerminalOrigin,
        TerminalProfile, TerminalProgramSpec, TerminalSink, TerminalStatus,
    },
    v0_2::{HostError, HostErrorCode},
};

use crate::{
    network::{IoEvent, Running},
    permissions::{CapabilityCheck, PermissionState},
    process::{self, Environment, FileRef, ProcessPolicy, Workspace},
};

fn error(code: HostErrorCode, message: impl Into<String>) -> HostError {
    HostError {
        code,
        message: message.into(),
    }
}

fn too_large(what: &str, max: usize) -> HostError {
    error(
        HostErrorCode::LimitExceeded,
        format!("{what} is more than {max} bytes"),
    )
}

/// The `${package}` of `terminal:env` values; listeners and the relay come with plan 5.11 and
/// 5.13, so a value that uses them is left out until then.
struct PackageContext(Option<String>);

impl TemplateContext for PackageContext {
    fn package(&self) -> Option<String> {
        self.0.clone()
    }
    fn listen_port(&self, _: &str) -> Option<u16> {
        None
    }
    fn listen_token(&self, _: &str) -> Option<String> {
        None
    }
    fn relay(&self) -> Option<String> {
        None
    }
    fn account_dir(&self, _: &str) -> Option<String> {
        None
    }
}

/// What the manifest declares for terminals, checked at load.
#[derive(Clone, Default)]
pub(crate) struct TerminalPolicy {
    /// `[[fyx_ui.terminal_programs]]`, as `[[fyx_ui.processes]]` entries for [`check_spawn`].
    pub(crate) programs: ProcessPolicy,
    /// `[fyx_ui.terminal_env]`, checked; empty without one.
    pub(crate) env: TerminalEnv,
    /// The extension's directory, for `${package}`.
    pub(crate) package_dir: Option<PathBuf>,
}

impl TerminalPolicy {
    pub(crate) fn new(
        programs: &[TerminalProgramSpec],
        env: Option<TerminalEnv>,
        package_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            programs: ProcessPolicy {
                programs: programs
                    .iter()
                    .map(TerminalProgramSpec::as_process)
                    .collect(),
                env: Default::default(),
            },
            env: env.unwrap_or_default(),
            package_dir,
        }
    }
}

/// P2.4: what the manifest declares for [`TerminalIntegration`]; the grants decide, each time
/// it is sent, which of it goes.
#[derive(Clone, Default)]
pub(crate) struct IntegrationSource {
    /// `[[fyx_ui.terminal_profiles]]`, checked, when `terminal:profiles` is declared.
    pub(crate) profiles: Vec<TerminalProfile>,
    /// `[fyx_ui.terminal_env]`, checked, when `terminal:env` is declared.
    pub(crate) env: Option<TerminalEnv>,
    /// The package directory, when `package:files` is declared.
    pub(crate) package_dir: Option<PathBuf>,
}

impl IntegrationSource {
    /// The integration as granted now: profiles with `terminal:profiles` (T1, accepted at
    /// install), the env with `terminal:env`, and its `all-new` scope with the second checkbox.
    /// Without a broker nothing is granted.
    pub(crate) fn granted(
        &self,
        broker: Option<&dyn CapabilityCheck>,
        extension_id: &str,
    ) -> TerminalIntegration {
        let Some(broker) = broker else {
            return TerminalIntegration::default();
        };
        let profiles = if broker
            .check(extension_id, terminal::CAPABILITY_PROFILES)
            .is_ok()
        {
            self.profiles.clone()
        } else {
            Vec::new()
        };
        let env = self.env.clone().filter(|_| {
            broker.state(extension_id, terminal::CAPABILITY_ENV) == PermissionState::Granted
        });
        let env_all_new = env.is_some()
            && broker.item_state(extension_id, &crate::consent_items::ConsentItem::EnvAllNew)
                == PermissionState::Granted;
        TerminalIntegration {
            profiles,
            env,
            env_all_new,
            package_dir: self.package_dir.clone(),
        }
    }
}

/// Which instance of an extension speaks to a terminal host, by host and extension id. A
/// reloaded instance takes over when the registry starts it ([`TerminalsHandle::activate`]);
/// the old one, dropped later (an `io` loop may still hold it), then leaves the new one's
/// terminals and integration alone.
static CURRENT_INSTANCES: LazyLock<Mutex<HashMap<(usize, Arc<str>), u64>>> =
    LazyLock::new(Default::default);
static NEXT_INSTANCE: AtomicU64 = AtomicU64::new(1);

/// One instance's tie to the terminal host: the sink its events leave through, and the cleanup
/// when the instance goes (reload, disable, uninstall) or a capability is revoked.
pub(crate) struct TerminalsHandle {
    extension_id: Arc<str>,
    host: Option<Arc<dyn TerminalHost>>,
    sender: mpsc::UnboundedSender<TerminalEvent>,
    receiver: Mutex<Option<mpsc::UnboundedReceiver<TerminalEvent>>>,
    instance: u64,
    integration: IntegrationSource,
    broker: Option<Arc<dyn CapabilityCheck>>,
}

impl TerminalsHandle {
    pub(crate) fn new(
        extension_id: Arc<str>,
        host: Option<Arc<dyn TerminalHost>>,
        integration: IntegrationSource,
        broker: Option<Arc<dyn CapabilityCheck>>,
    ) -> Self {
        let (sender, receiver) = mpsc::unbounded();
        Self {
            extension_id,
            host,
            sender,
            receiver: Mutex::new(Some(receiver)),
            instance: NEXT_INSTANCE.fetch_add(1, Ordering::Relaxed),
            integration,
            broker,
        }
    }

    pub(crate) fn available(&self) -> bool {
        self.host.is_some()
    }

    fn instance_key(&self, host: &Arc<dyn TerminalHost>) -> (usize, Arc<str>) {
        (
            Arc::as_ptr(host) as *const () as usize,
            self.extension_id.clone(),
        )
    }

    /// The host this instance may act on: it is the extension's current instance, or no instance
    /// is (one that was never started, as in tests and the harness).
    fn current_host(&self) -> Option<&Arc<dyn TerminalHost>> {
        let host = self.host.as_ref()?;
        let current = CURRENT_INSTANCES
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&self.instance_key(host))
            .copied();
        current
            .is_none_or(|current| current == self.instance)
            .then_some(host)
    }

    /// The registry started this instance (after consent, at every load and reload): it becomes
    /// the extension's current one and sends its integration.
    pub(crate) fn activate(&self) {
        let Some(host) = &self.host else {
            return;
        };
        CURRENT_INSTANCES
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(self.instance_key(host), self.instance);
        self.send_integration();
    }

    /// P2.4: sends the profiles and env as granted now; again after a grant changes.
    pub(crate) fn send_integration(&self) {
        if let Some(host) = self.current_host() {
            host.set_integration(
                &self.extension_id,
                self.integration
                    .granted(self.broker.as_deref(), &self.extension_id),
            );
        }
    }

    /// Whether a change of `capability` (or of one of its items, `terminal:env#all-new`) changes
    /// what [`TerminalsHandle::send_integration`] sends.
    pub(crate) fn affects_integration(capability: &str) -> bool {
        let capability = capability
            .split_once('#')
            .map_or(capability, |(capability, _)| capability);
        capability == terminal::CAPABILITY_ENV || capability == terminal::CAPABILITY_PROFILES
    }

    /// Where the host sends this instance's terminal events. It never blocks: events queue here
    /// and [`TerminalsHandle::forward`] hands them to `handle-io` in order. The host bounds the
    /// rate (30 events per second per terminal, §B.2–§B.3), so the queue stays short.
    fn sink(&self) -> TerminalSink {
        let sender = self.sender.clone();
        Arc::new(move |event| {
            sender.unbounded_send(event).ok();
        })
    }

    /// Moves the host's events onto the io channel, on the current tokio runtime. Once.
    pub(crate) fn forward(&self, running: &Arc<Running>) {
        let Some(mut receiver) = self
            .receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        else {
            return;
        };
        let running = Arc::downgrade(running);
        tokio::spawn(async move {
            while let Some(event) = receiver.next().await {
                let Some(running) = running.upgrade() else {
                    return;
                };
                running.deliver(IoEvent::Terminal(event)).await;
            }
        });
    }

    /// `terminal:create` or `terminal:env` was revoked, or the instance is gone: every terminal
    /// of the extension closes, created ones with their process trees (§C.2).
    pub(crate) fn close_all(&self) {
        if let Some(host) = self.current_host() {
            host.close_all(&self.extension_id);
        }
    }

    /// `terminal:observe` was revoked: no more events about other terminals.
    pub(crate) fn stop_observing(&self) {
        if let Some(host) = self.current_host() {
            host.observe(&self.extension_id, None);
        }
    }
}

impl Drop for TerminalsHandle {
    /// The instance is gone (reload, disable, uninstall, revocation): if it was the current one,
    /// the extension's integration empties and its terminals close.
    fn drop(&mut self) {
        let Some(host) = self.current_host().cloned() else {
            return;
        };
        host.set_integration(&self.extension_id, TerminalIntegration::default());
        host.close_all(&self.extension_id);
        let key = self.instance_key(&host);
        let mut current = CURRENT_INSTANCES
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if current.get(&key) == Some(&self.instance) {
            current.remove(&key);
        }
    }
}

/// The extension's own terminal, as the import knows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Owned {
    origin: TerminalOrigin,
    /// A created terminal whose program has not exited: what [`terminal::MAX_CREATED_TERMINALS`]
    /// counts.
    running: bool,
}

/// The `terminals` state of one instance, in its store.
pub(crate) struct TerminalState {
    extension_id: Arc<str>,
    handle: Arc<TerminalsHandle>,
    policy: Arc<TerminalPolicy>,
    workspace: Option<Arc<dyn Workspace>>,
    environment: Environment,
    owned: HashMap<TerminalHandle, Owned>,
}

impl TerminalState {
    pub(crate) fn new(
        extension_id: Arc<str>,
        handle: Arc<TerminalsHandle>,
        policy: Arc<TerminalPolicy>,
        workspace: Option<Arc<dyn Workspace>>,
        environment: Option<Environment>,
    ) -> Self {
        Self {
            extension_id,
            handle,
            policy,
            workspace,
            environment: environment.unwrap_or_else(|| Arc::new(|| std::env::vars_os().collect())),
            owned: HashMap::new(),
        }
    }

    fn host(&self) -> Result<&Arc<dyn TerminalHost>, HostError> {
        self.handle.host.as_ref().ok_or_else(|| {
            error(
                HostErrorCode::Unavailable,
                "terminals are not available in this window",
            )
        })
    }

    fn own(&self, handle: TerminalHandle, origin: TerminalOrigin) -> Result<(), HostError> {
        match self.owned.get(&handle) {
            Some(owned) if owned.origin == origin => Ok(()),
            _ => Err(TerminalError::NotFound.into()),
        }
    }

    /// `open-pseudo`; the caller checked `terminal:pseudo` (T1).
    pub(crate) fn open_pseudo(
        &mut self,
        request: PseudoRequest,
    ) -> Result<TerminalHandle, HostError> {
        request.check()?;
        let handle = self
            .host()?
            .open_pseudo(&self.extension_id, request, self.handle.sink())?;
        self.owned.insert(
            handle,
            Owned {
                origin: TerminalOrigin::Pseudo,
                running: false,
            },
        );
        Ok(handle)
    }

    /// `write-output`, to one of its pseudo-terminals.
    pub(crate) fn write_output(
        &mut self,
        handle: TerminalHandle,
        bytes: Vec<u8>,
    ) -> Result<(), HostError> {
        self.own(handle, TerminalOrigin::Pseudo)?;
        if bytes.len() > terminal::MAX_WRITE_OUTPUT_BYTES {
            return Err(too_large("write-output", terminal::MAX_WRITE_OUTPUT_BYTES));
        }
        Ok(self
            .host()?
            .write_output(&self.extension_id, handle, bytes)?)
    }

    /// `create`: a declared program, with arguments its patterns allow, in a trusted worktree,
    /// with `terminal:env` when `env_granted`. The caller checked `terminal:create` (T2).
    pub(crate) fn create(
        &mut self,
        env_granted: bool,
        program: String,
        args: Vec<String>,
        cwd: Option<FileRef>,
        title: Option<String>,
    ) -> Result<TerminalHandle, HostError> {
        if let Some(title) = &title
            && title.len() > terminal::MAX_TITLE_BYTES
        {
            return Err(too_large("the terminal title", terminal::MAX_TITLE_BYTES));
        }
        let request = SpawnRequest {
            program,
            args,
            cwd,
            env: Vec::new(),
            stdin: false,
        };
        let spec = match check_spawn(&request, &self.policy.programs)? {
            ui_ext::process::SpawnCheck::Spawn(spec) => spec.clone(),
            ui_ext::process::SpawnCheck::NotDeclared => {
                return Err(error(
                    HostErrorCode::NotPermitted,
                    format!(
                        "{} is not in the manifest's [[fyx_ui.terminal_programs]]",
                        request.program
                    ),
                ));
            }
        };
        let running = self.owned.values().filter(|owned| owned.running).count();
        if running >= terminal::MAX_CREATED_TERMINALS {
            return Err(error(
                HostErrorCode::LimitExceeded,
                format!(
                    "at most {} created terminals run at once",
                    terminal::MAX_CREATED_TERMINALS
                ),
            ));
        }
        let worktrees = self
            .workspace
            .as_ref()
            .map(|workspace| workspace.worktrees())
            .unwrap_or_default();
        if worktrees.is_empty() {
            return Err(error(HostErrorCode::NoWorktree, "no worktree is open"));
        }
        let (cwd, worktree) = process::resolve_cwd(request.cwd.as_ref(), &worktrees)
            .map_err(|why| error(HostErrorCode::InvalidArgument, format!("cwd: {why:?}")))?;
        if !worktree.trusted {
            return Err(error(
                HostErrorCode::UntrustedWorktree,
                "the worktree is restricted: it is not trusted",
            ));
        }
        let roots: Vec<PathBuf> = worktrees
            .iter()
            .filter_map(|worktree| worktree.root.canonicalize().ok())
            .collect();
        let program = match &spec.path {
            Some(relative) => process::resolve_in_worktree(&worktree.root, relative),
            None => {
                let environment = (self.environment)();
                let path = environment
                    .iter()
                    .find(|(name, _)| {
                        name.to_str()
                            .is_some_and(|name| name.eq_ignore_ascii_case("PATH"))
                    })
                    .map(|(_, value)| value.clone())
                    .unwrap_or_default();
                process::resolve_on_path(&spec.name, OsStr::new(&path), &roots)
            }
        }
        .map_err(|why| error(HostErrorCode::Unavailable, why))?;
        let env = if env_granted {
            self.policy.env.resolve(&PackageContext(
                self.policy
                    .package_dir
                    .as_ref()
                    .map(|dir| dir.to_string_lossy().into_owned()),
            ))
        } else {
            Vec::new()
        };
        let command = CreateCommand {
            program,
            args: request.args,
            cwd,
            title,
            env,
        };
        let handle = self
            .host()?
            .create(&self.extension_id, command, self.handle.sink())?;
        self.owned.insert(
            handle,
            Owned {
                origin: TerminalOrigin::Created,
                running: true,
            },
        );
        Ok(handle)
    }

    /// `send-input`, to one of its created terminals.
    pub(crate) fn send_input(
        &mut self,
        handle: TerminalHandle,
        bytes: Vec<u8>,
    ) -> Result<(), HostError> {
        self.own(handle, TerminalOrigin::Created)?;
        if bytes.len() > terminal::MAX_SEND_INPUT_BYTES {
            return Err(too_large("send-input", terminal::MAX_SEND_INPUT_BYTES));
        }
        Ok(self.host()?.send_input(&self.extension_id, handle, bytes)?)
    }

    /// `close`: only its own terminals; anything else is ignored.
    pub(crate) fn close(&mut self, handle: TerminalHandle) {
        if self.owned.remove(&handle).is_some()
            && let Ok(host) = self.host()
        {
            host.close(&self.extension_id, handle);
        }
    }

    /// Whether `handle` is one of the extension's own terminals (pseudo or created).
    pub(crate) fn owns(&self, handle: TerminalHandle) -> bool {
        self.owned.contains_key(&handle)
    }

    /// `set-status`; on a terminal it does not [`own`](Self::owns), the caller checked
    /// `terminal:status` (T1).
    pub(crate) fn set_status(
        &mut self,
        handle: TerminalHandle,
        status: Option<TerminalStatus>,
    ) -> Result<(), HostError> {
        if let Some(status) = &status {
            status.check()?;
        }
        Ok(self
            .host()?
            .set_status(&self.extension_id, handle, status)?)
    }

    /// `list`: the terminals now, and their later changes as events. The caller checked
    /// `terminal:observe` (T2).
    pub(crate) fn list(&mut self) -> Result<Vec<terminal::TerminalInfo>, HostError> {
        let host = self.host()?;
        Ok(host.observe(&self.extension_id, Some(self.handle.sink())))
    }

    /// `paste`, after a gesture: the host refuses a terminal that is not the extension's own or
    /// one of its profiles' ([`TerminalError::NotFound`]).
    pub(crate) fn paste(
        &mut self,
        gesture: bool,
        handle: TerminalHandle,
        text: String,
    ) -> Result<(), HostError> {
        if !gesture {
            return Err(error(
                HostErrorCode::NotPermitted,
                "paste needs a click or Enter in the extension's UI",
            ));
        }
        if text.len() > terminal::MAX_PASTE_BYTES {
            return Err(too_large("paste", terminal::MAX_PASTE_BYTES));
        }
        Ok(self.host()?.paste(&self.extension_id, handle, text)?)
    }

    /// Before `handle-io` delivers a terminal event: an exited created terminal stops counting
    /// toward the cap, and a closed one is no longer the extension's.
    pub(crate) fn delivered(&mut self, event: &TerminalEvent) {
        match event {
            TerminalEvent::Exited(handle, _) => {
                if let Some(owned) = self.owned.get_mut(handle) {
                    owned.running = false;
                }
            }
            TerminalEvent::Closed(handle) => {
                self.owned.remove(handle);
            }
            _ => {}
        }
    }
}

/// A T2 capability a synchronous call needs now: only `granted` passes (§C.2).
pub(crate) fn t2_now(
    broker: Option<&Arc<dyn CapabilityCheck>>,
    extension_id: &str,
    state: PermissionState,
    capability: &'static str,
) -> Result<(), HostError> {
    match state {
        PermissionState::Granted => Ok(()),
        PermissionState::Ask => {
            // The dev path keeps 0.3's first-use prompt (§C.2); the call that needs the answer
            // cannot wait for it, so it is refused and the answer arrives as an event.
            if let Some(broker) = broker {
                let request = broker.request(extension_id, capability);
                tokio::spawn(request);
            }
            Err(error(
                HostErrorCode::Denied,
                format!(
                    "{capability} is not granted yet: the user is asked, and the answer arrives \
                     as permission-changed"
                ),
            ))
        }
        state => Err(error(
            HostErrorCode::Denied,
            format!("{capability} is {state:?}"),
        )),
    }
}
