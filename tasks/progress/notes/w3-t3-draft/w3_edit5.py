import shutil
S='tasks/progress/notes/w3-t3-draft'
shutil.copy(f'{S}/t3_cards.rs','crates/ui_ext_host/src/t3_cards.rs')

def edit(p, pairs):
    s=open(p,encoding='utf-8').read()
    for old,new in pairs:
        n=s.count(old); assert n==1,(p,old[:70],n); s=s.replace(old,new)
    open(p,'w',encoding='utf-8',newline='\n').write(s)

edit('crates/ui_ext_host/src/ui_ext_host.rs', [
("""mod surface;
mod view;""","""mod surface;
pub mod t3_cards;
mod view;"""),
])

edit('crates/ui_ext_host/src/registry.rs', [
("""    /// The last revoke's write to the install store's approvals (each waits for the one before).
    approval_writes: Option<futures::future::Shared<Task<()>>>,
}
""","""    /// The last revoke's write to the install store's approvals (each waits for the one before).
    approval_writes: Option<futures::future::Shared<Task<()>>>,
    /// T3 cards (§C.3) until the workspace takes them to draw, by the registry's id.
    t3_requests: HashMap<u64, crate::consent::T3PromptRequest>,
    next_t3_request: u64,
}
"""),
("""    /// `approvals.ask` (0.4, §B.12): the workspace named shows the card (the active window's).
    ShowApproval {
        extension_id: Arc<str>,
        id: u64,
        workspace: gpui::EntityId,
    },
}""","""    /// `approvals.ask` (0.4, §B.12): the workspace named shows the card (the active window's).
    ShowApproval {
        extension_id: Arc<str>,
        id: u64,
        workspace: gpui::EntityId,
    },
    /// A T3 request (0.4, §C.3): the workspace named shows its card (the active window's).
    ShowT3 { id: u64, workspace: gpui::EntityId },
}"""),
("""    fn show_permission_prompt(
        &mut self,""","""    /// Routes T3 requests (§C.3) to a card in the active window. Called once when the registry
    /// is set up; without it every T3 request is `Deny`.
    pub fn start_t3_prompts(&mut self, cx: &mut Context<Self>) {
        let (sender, mut requests) = futures::channel::mpsc::unbounded();
        self.consent.set_t3_prompter(sender);
        cx.spawn(async move |this, cx| {
            while let Some(request) = requests.next().await {
                if this
                    .update(cx, |this, cx| this.show_t3_prompt(request, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// With no window the request is dropped: `Deny`.
    fn show_t3_prompt(&mut self, request: crate::consent::T3PromptRequest, cx: &mut Context<Self>) {
        let Some(workspace) = self.active_workspace else {
            log::info!(
                "fyx:ui: {} asks for {} with no window to ask in",
                request.0,
                request.1.capability.name()
            );
            return;
        };
        self.next_t3_request += 1;
        let id = self.next_t3_request;
        self.t3_requests.insert(id, request);
        cx.emit(RegistryEvent::ShowT3 { id, workspace });
    }

    /// The request behind a [`RegistryEvent::ShowT3`], once, with the provenance name and
    /// whether "Always allow" can be offered.
    pub(crate) fn take_t3_request(
        &mut self,
        id: u64,
    ) -> Option<(crate::consent::T3PromptRequest, SharedString, bool)> {
        let request = self.t3_requests.remove(&id)?;
        let name = self.provenance_name(&request.0);
        let can_always_allow = self.consent.can_always_allow(&request.0);
        Some((request, name, can_always_allow))
    }

    /// T3 "Always allow" on or off (§C.3), from a card, the extensions page or the status bar.
    /// Returns whether it was saved.
    pub fn set_always_allow(
        &mut self,
        extension_id: &str,
        capability: ui_ext_wasm::permissions::T3Capability,
        on: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.consent.set_always_allow(extension_id, capability, on) {
            Ok(write) => {
                if let Some((path, text)) = write {
                    let fs = <dyn Fs>::global(cx);
                    cx.background_spawn(
                        async move { consent::save(path, text, fs.as_ref()).await },
                    )
                    .detach();
                }
                log::info!(
                    "fyx:ui: {extension_id}: always allow {} {}",
                    capability.name(),
                    if on { "on" } else { "off" }
                );
                cx.notify();
                true
            }
            Err(error) => {
                log::error!(
                    "fyx:ui: {}",
                    ui_ext::sanitize_for_log(&format!("{error:#}"))
                );
                false
            }
        }
    }

    /// Every T3 "Always allow" that holds now.
    pub fn always_allowed(&self) -> Vec<(String, ui_ext_wasm::permissions::T3Capability)> {
        self.consent.always_allowed()
    }

    /// Whether "Always allow" can be given to the extension: a component, not a dev folder.
    pub fn can_always_allow(&self, extension_id: &str) -> bool {
        self.consent.can_always_allow(extension_id)
    }

    fn show_permission_prompt(
        &mut self,"""),
("""        registry.start_permission_prompts(cx)
    });""","""        registry.start_permission_prompts(cx);
        registry.start_t3_prompts(cx);
    });"""),
("""                RegistryEvent::PermissionPrompt {
                    extension_id,
                    capability,
                    workspace: target,
                } => {""","""                RegistryEvent::ShowT3 {
                    id,
                    workspace: target,
                } => {
                    if *target == cx.entity_id() {
                        show_t3_card(workspace, &registry, *id, cx);
                    }
                    return;
                }
                RegistryEvent::PermissionPrompt {
                    extension_id,
                    capability,
                    workspace: target,
                } => {"""),
("""    // Extension progress in the status bar (design.md M.2), one item for all extensions.""","""    // T3 "Always allow" (§C.3): who has terminal access, with one-click revoke.
    let access = cx.new(|cx| crate::t3_cards::T3AccessStatus::new(registry.clone(), cx));
    workspace.status_bar().update(cx, |status_bar, cx| {
        status_bar.add_right_item(access, window, cx)
    });

    // Extension progress in the status bar (design.md M.2), one item for all extensions."""),
("""/// The T2 first-use prompt (decision Q10): a [`crate::notifications::PermissionCard`] in the""","""/// A T3 request (§C.3): a [`crate::t3_cards::T3Card`] in the workspace's notification stack.
fn show_t3_card(
    workspace: &mut Workspace,
    registry: &Entity<UiExtensionRegistry>,
    id: u64,
    cx: &mut Context<Workspace>,
) {
    let Some(((extension_id, prompt, answer), name, can_always_allow)) =
        registry.update(cx, |registry, _| registry.take_t3_request(id))
    else {
        return;
    };
    let notification_id =
        NotificationId::composite::<crate::t3_cards::T3Card>(SharedString::from(format!(
            "{extension_id}/{id}"
        )));
    let extension_id: Arc<str> = extension_id.into();
    let registry = registry.downgrade();
    workspace.show_notification(notification_id, cx, move |cx| {
        cx.new(|cx| {
            crate::t3_cards::T3Card::new(
                extension_id,
                name,
                prompt,
                answer,
                can_always_allow,
                registry,
                cx,
            )
        })
    });
}

/// The T2 first-use prompt (decision Q10): a [`crate::notifications::PermissionCard`] in the"""),
])
print('ok')
