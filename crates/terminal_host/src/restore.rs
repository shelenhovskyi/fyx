//! Plan 7.14 (a): the terminals started from an extension's profile come back when their project
//! opens again, with the same profile and account, and resume the program's session where the
//! profile has a `resume_flag` (Claude Code's `--resume <session id>`).
//!
//! Upstream's workspace persistence would restore such a terminal as a plain shell, so
//! `terminal_view` skips it (`TerminalExt::is_profile_terminal`) and this module keeps its own
//! list in `<data dir>/fyx/profile_terminals.json`, keyed by the workspace's first local root. The
//! session id is the `session_id` the program's hooks send through `fyx-relay`; only an id
//! [`ui_ext::terminal::is_session_id`] accepts is ever passed on a command line.
//!
//! A terminal leaves the list when its tab closes; quitting Fyx keeps it. A record waits until its
//! extension has handed over its profiles (extensions load after the first window opens) and is
//! dropped when the setting is off.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{App, AppContext as _, Entity, EntityId, Global, WeakEntity, WindowHandle};
use serde::{Deserialize, Serialize};
use settings::Settings as _;
use terminal::Terminal;
use terminal_ext::TerminalId;
use workspace::{MultiWorkspace, Workspace};

use crate::settings::FyxTerminalSettings;

/// How long to wait after an extension hands over its profiles before reopening its terminals, so
/// its listeners (the IDE server a profile's env names) are open.
const SETTLE: Duration = Duration::from_secs(2);

/// One profile terminal to reopen.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Record {
    pub(crate) ext: String,
    pub(crate) profile: String,
    /// `<tool>=<name>`, as the "+" menu picked it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) account_choice: Option<String>,
    /// The program's session to resume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) session: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    workspaces: BTreeMap<String, Vec<Record>>,
}

struct Live {
    key: String,
    record: Record,
}

struct Pending {
    key: String,
    workspace: WeakEntity<Workspace>,
    window: WindowHandle<MultiWorkspace>,
    records: Vec<Record>,
}

#[derive(Default)]
struct Restore {
    file: Option<PathBuf>,
    /// Workspaces not opened in this run, as the file had them.
    untouched: BTreeMap<String, Vec<Record>>,
    live: HashMap<TerminalId, Live>,
    pending: Vec<Pending>,
    checked: HashSet<EntityId>,
    quitting: bool,
    /// Saves run in the background; an older one never overwrites a newer one.
    generation: u64,
    written: Arc<Mutex<u64>>,
}

impl Global for Restore {}

/// Reads the list and starts watching for workspaces. Called once, from `main.rs`.
pub(crate) fn init(data_dir: PathBuf, cx: &mut App) {
    let file = data_dir.join("fyx").join("profile_terminals.json");
    let untouched = match std::fs::read_to_string(&file) {
        Ok(text) => match serde_json::from_str::<Saved>(&text) {
            Ok(saved) => saved.workspaces,
            Err(error) => {
                log::warn!("fyx: {} is not readable: {error}", file.display());
                BTreeMap::default()
            }
        },
        Err(_) => BTreeMap::default(),
    };
    cx.set_global(Restore {
        file: Some(file),
        untouched,
        ..Restore::default()
    });
    cx.on_app_quit(|cx| {
        cx.default_global::<Restore>().quitting = true;
        async {}
    })
    .detach();
    cx.observe_new(|workspace: &mut Workspace, window, cx| {
        let Some(window) = window else {
            return;
        };
        let Some(handle) = window.window_handle().downcast::<MultiWorkspace>() else {
            return;
        };
        let project = workspace.project().clone();
        cx.subscribe(&project, move |workspace, _, event, cx| {
            if matches!(event, project::Event::WorktreeAdded(_)) {
                workspace_opened(workspace, handle, cx);
            }
        })
        .detach();
        workspace_opened(workspace, handle, cx);
    })
    .detach();
}

/// The key of a workspace: its first local root, where a profile's terminal starts.
pub(crate) fn key(roots: &[PathBuf]) -> Option<String> {
    roots
        .first()
        .map(|root| root.to_string_lossy().into_owned())
}

fn workspace_opened(
    workspace: &mut Workspace,
    window: WindowHandle<MultiWorkspace>,
    cx: &mut gpui::Context<Workspace>,
) {
    let Some(key) = key(&crate::profiles::local_roots(workspace, cx)) else {
        return;
    };
    let id = cx.entity_id();
    let Some(restore) = cx.try_global::<Restore>() else {
        return;
    };
    if restore.checked.contains(&id) {
        return;
    }
    let enabled = FyxTerminalSettings::get_global(cx).restore_profile_terminals;
    let weak = cx.weak_entity();
    let restore = cx.global_mut::<Restore>();
    restore.checked.insert(id);
    let Some(records) = restore.untouched.remove(&key) else {
        return;
    };
    if enabled {
        restore.pending.push(Pending {
            key,
            workspace: weak,
            window,
            records,
        });
        cx.defer(|cx| try_reopen(cx));
    } else {
        save(cx);
    }
}

/// An extension handed over its profiles: reopen its terminals once its listeners are up.
pub(crate) fn integration_changed(cx: &mut App) {
    if cx
        .try_global::<Restore>()
        .is_none_or(|restore| restore.pending.is_empty())
    {
        return;
    }
    cx.spawn(async move |cx| {
        cx.background_executor().timer(SETTLE).await;
        cx.update(try_reopen);
    })
    .detach();
}

/// Starts every pending record whose extension has its profiles now.
fn try_reopen(cx: &mut App) {
    let Some(restore) = cx.try_global::<Restore>() else {
        return;
    };
    if restore.pending.is_empty() {
        return;
    }
    let loaded = crate::profiles::loaded_profiles(cx);
    let mut ready = Vec::new();
    let restore = cx.global_mut::<Restore>();
    restore.pending.retain_mut(|pending| {
        if pending.workspace.upgrade().is_none() {
            return false;
        }
        let (now, later) = pending.records.drain(..).partition::<Vec<_>, _>(|record| {
            loaded.contains(&(record.ext.clone(), record.profile.clone()))
        });
        pending.records = later;
        if !now.is_empty() {
            ready.push((pending.workspace.clone(), pending.window, now));
        }
        !pending.records.is_empty()
    });
    for (workspace, window, records) in ready {
        let Some(workspace) = workspace.upgrade() else {
            continue;
        };
        window
            .update(cx, |_, window, cx| {
                for record in records {
                    crate::profiles::launch(workspace.clone(), record, window, cx);
                }
            })
            .ok();
    }
}

/// A profile's terminal started in a workspace keyed `key`.
pub(crate) fn started(terminal: &Entity<Terminal>, key: String, record: Record, cx: &mut App) {
    if !cx.has_global::<Restore>() {
        return;
    }
    let id = terminal.read(cx).fyx.id();
    cx.global_mut::<Restore>()
        .live
        .insert(id, Live { key, record });
    cx.observe_release(terminal, move |_, cx| {
        let restore = cx.default_global::<Restore>();
        if restore.quitting {
            return;
        }
        if restore.live.remove(&id).is_some() {
            save(cx);
        }
    })
    .detach();
    save(cx);
}

/// For every relay delivery: a profile terminal's hook reported its program's session.
pub fn note_relay_stdin(terminal: TerminalId, stdin: &str, cx: &mut App) {
    let tracked = cx
        .try_global::<Restore>()
        .is_some_and(|restore| restore.live.contains_key(&terminal));
    if !tracked || !stdin.contains("\"session_id\"") {
        return;
    }
    let Some(session) = session_id(stdin) else {
        return;
    };
    let restore = cx.global_mut::<Restore>();
    let Some(live) = restore.live.get_mut(&terminal) else {
        return;
    };
    if live.record.session.as_deref() != Some(session.as_str()) {
        live.record.session = Some(session);
        save(cx);
    }
}

/// The top-level `session_id` of a hook's JSON input, when it is safe to pass on.
fn session_id(stdin: &str) -> Option<String> {
    let value = serde_json::from_str::<serde_json::Value>(stdin).ok()?;
    let id = value.get("session_id")?.as_str()?;
    ui_ext::terminal::is_session_id(id).then(|| id.to_owned())
}

/// Whether reopened terminals resume their session.
pub(crate) fn resume_enabled(cx: &App) -> bool {
    !cx.has_global::<settings::SettingsStore>()
        || FyxTerminalSettings::get_global(cx).resume_profile_sessions
}

fn snapshot(restore: &Restore) -> Saved {
    let mut workspaces = restore.untouched.clone();
    for live in restore.live.values() {
        workspaces
            .entry(live.key.clone())
            .or_default()
            .push(live.record.clone());
    }
    for pending in &restore.pending {
        workspaces
            .entry(pending.key.clone())
            .or_default()
            .extend(pending.records.iter().cloned());
    }
    Saved { workspaces }
}

fn save(cx: &mut App) {
    if !cx.has_global::<Restore>() {
        return;
    }
    let restore = cx.global_mut::<Restore>();
    let Some(file) = restore.file.clone() else {
        return;
    };
    restore.generation += 1;
    let generation = restore.generation;
    let written = restore.written.clone();
    let saved = snapshot(restore);
    cx.background_spawn(async move {
        let mut written = match written.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if *written > generation {
            return;
        }
        match write(&file, &saved) {
            Ok(()) => *written = generation,
            Err(error) => log::warn!("fyx: {} not written: {error}", file.display()),
        }
    })
    .detach();
}

fn write(file: &Path, saved: &Saved) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let partial = file.with_extension("json.partial");
    std::fs::write(&partial, serde_json::to_vec_pretty(saved)?)?;
    std::fs::rename(&partial, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(ext: &str, session: Option<&str>) -> Record {
        Record {
            ext: ext.into(),
            profile: "claude".into(),
            account_choice: Some("claude=Work".into()),
            session: session.map(Into::into),
        }
    }

    #[test]
    fn a_hook_reports_its_session_only_when_it_is_safe_to_pass_on() {
        assert_eq!(
            session_id(r#"{"hook_event_name":"SessionStart","session_id":"0b6f-1c2e_9"}"#),
            Some("0b6f-1c2e_9".to_owned())
        );
        for stdin in [
            r#"{"session_id":"--dangerously-skip-permissions"}"#,
            r#"{"session_id":"a b"}"#,
            r#"{"session_id":7}"#,
            r#"{"nested":{"session_id":"x"}}"#,
            "not json",
        ] {
            assert_eq!(session_id(stdin), None, "{stdin}");
        }
    }

    #[test]
    fn the_saved_list_keeps_unopened_workspaces_live_terminals_and_waiting_ones() {
        let mut restore = Restore::default();
        restore
            .untouched
            .insert("/other".into(), vec![record("a", Some("s0"))]);
        restore.live.insert(
            terminal_ext::TerminalExt::new().id(),
            Live {
                key: "/work".into(),
                record: record("claude-code", Some("s1")),
            },
        );
        restore.pending.push(Pending {
            key: "/work".into(),
            workspace: WeakEntity::new_invalid(),
            window: WindowHandle::new(gpui::WindowId::from(1)),
            records: vec![record("gemini", None)],
        });
        let saved = snapshot(&restore);
        assert_eq!(
            saved.workspaces,
            BTreeMap::from([
                ("/other".to_owned(), vec![record("a", Some("s0"))]),
                (
                    "/work".to_owned(),
                    vec![record("claude-code", Some("s1")), record("gemini", None)]
                ),
            ])
        );
        let text = serde_json::to_string(&saved).unwrap();
        assert_eq!(serde_json::from_str::<Saved>(&text).unwrap(), saved);
        assert_eq!(
            serde_json::from_str::<Saved>(r#"{"workspaces":{"/w":[{"ext":"e","profile":"p"}]}}"#)
                .unwrap()
                .workspaces["/w"],
            [Record {
                ext: "e".into(),
                profile: "p".into(),
                ..Record::default()
            }]
        );
        assert_eq!(key(&[]), None);
    }
}
