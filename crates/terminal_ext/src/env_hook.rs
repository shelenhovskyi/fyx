//! The hook `TerminalBuilder::new` calls on the main thread, before the pty starts, to add Fyx's
//! variables to a new terminal's environment: the account profile's config dir (design §C.6) and,
//! later, extensions' `terminal:env` (§B.4). `terminal_host` provides it; with no provider set
//! (tests, other binaries) the environment is left as upstream builds it.

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::path::Path;
use std::rc::Rc;

use gpui::{App, Global};

/// Env entry with the account the terminal was opened for: `<tool>=<profile name>`, or `none` for
/// no account even where the project has one. [`apply_env_hook`] removes it.
pub const ACCOUNT_CHOICE_ENV: &str = "FYX_ACCOUNT";

/// Env entry carrying the [`AccountBadge`]s from [`apply_env_hook`] to [`crate::TerminalExt::insert_env`],
/// which removes it.
pub const PARKED_ACCOUNTS_ENV: &str = "FYX_PARKED_ACCOUNTS";

/// An account profile a terminal runs with, for its tab and the sessions list. Only the name and
/// the colour: the dir stays in the terminal's environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountBadge {
    pub tool: String,
    pub name: String,
    /// The colour's settings name (`blue`, `red`, ...).
    pub color: String,
}

/// What the hook knows about the terminal being built.
#[derive(Clone, Debug, Default)]
pub struct EnvRequest<'a> {
    /// The working directory the terminal starts in.
    pub cwd: Option<&'a Path>,
    /// The terminal runs on another machine (SSH, WSL, ...): local account dirs mean nothing there.
    pub remote: bool,
    /// [`ACCOUNT_CHOICE_ENV`], when the terminal was opened for an account.
    pub account_choice: Option<&'a str>,
}

/// What the hook adds.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EnvAdditions {
    pub vars: Vec<(String, String)>,
    pub accounts: Vec<AccountBadge>,
}

pub trait TerminalEnvHook: 'static {
    fn env_for(&self, request: &EnvRequest<'_>, cx: &App) -> EnvAdditions;
}

struct GlobalEnvHook(Rc<dyn TerminalEnvHook>);

impl Global for GlobalEnvHook {}

pub fn set_env_hook(hook: impl TerminalEnvHook, cx: &mut App) {
    cx.set_global(GlobalEnvHook(Rc::new(hook)));
}

/// Called by `TerminalBuilder::new` with the environment it is about to start the program with.
/// Takes [`ACCOUNT_CHOICE_ENV`] out, asks the hook, and adds its variables. Names of Fyx's or Zed's
/// own (`FYX_*`, `ZED_*`) are never added, so the hook cannot forge a terminal's id or token.
pub fn apply_env_hook<S: BuildHasher>(
    env: &mut HashMap<String, String, S>,
    cwd: Option<&Path>,
    remote: bool,
    cx: &App,
) {
    let choice = take_choice(env);
    let Some(hook) = cx.try_global::<GlobalEnvHook>().map(|hook| hook.0.clone()) else {
        return;
    };
    let additions = hook.env_for(
        &EnvRequest {
            cwd,
            remote,
            account_choice: choice.as_deref(),
        },
        cx,
    );
    add(env, additions);
}

/// Removes [`ACCOUNT_CHOICE_ENV`] (returned) and any [`PARKED_ACCOUNTS_ENV`] not set by [`add`].
fn take_choice<S: BuildHasher>(env: &mut HashMap<String, String, S>) -> Option<String> {
    env.remove(PARKED_ACCOUNTS_ENV);
    env.remove(ACCOUNT_CHOICE_ENV)
}

fn add<S: BuildHasher>(env: &mut HashMap<String, String, S>, additions: EnvAdditions) {
    for (name, value) in additions.vars {
        if is_host_name(&name) {
            log::warn!("terminal env hook: refused to set {name}");
            continue;
        }
        env.insert(name, value);
    }
    if !additions.accounts.is_empty() {
        env.insert(
            PARKED_ACCOUNTS_ENV.to_owned(),
            encode_badges(&additions.accounts),
        );
    }
}

fn is_host_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.starts_with("FYX_") || upper.starts_with("ZED_")
}

const FIELD: char = '\u{1f}';
const RECORD: char = '\u{1e}';

fn encode_badges(badges: &[AccountBadge]) -> String {
    let clean = |text: &str| text.replace([FIELD, RECORD], "");
    badges
        .iter()
        .map(|badge| {
            format!(
                "{}{FIELD}{}{FIELD}{}",
                clean(&badge.tool),
                clean(&badge.name),
                clean(&badge.color)
            )
        })
        .collect::<Vec<_>>()
        .join(&RECORD.to_string())
}

pub(crate) fn decode_badges(text: &str) -> Vec<AccountBadge> {
    text.split(RECORD)
        .filter_map(|record| {
            let mut fields = record.split(FIELD);
            let badge = AccountBadge {
                tool: fields.next()?.to_owned(),
                name: fields.next()?.to_owned(),
                color: fields.next()?.to_owned(),
            };
            (fields.next().is_none() && !badge.name.is_empty()).then_some(badge)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TerminalExt;

    fn work(name: &str) -> EnvAdditions {
        EnvAdditions {
            vars: vec![
                ("CLAUDE_CONFIG_DIR".into(), "/w".into()),
                ("FYX_TERMINAL_TOKEN".into(), "forged".into()),
                ("zed_x".into(), "forged".into()),
            ],
            accounts: vec![AccountBadge {
                tool: "claude".into(),
                name: name.into(),
                color: "red".into(),
            }],
        }
    }

    #[test]
    fn additions_land_but_host_names_do_not() {
        let mut env: HashMap<String, String> = HashMap::from_iter([
            (ACCOUNT_CHOICE_ENV.to_owned(), "claude=Personal".to_owned()),
            (
                PARKED_ACCOUNTS_ENV.to_owned(),
                "claude\u{1f}Fake\u{1f}red".to_owned(),
            ),
        ]);
        assert_eq!(take_choice(&mut env).as_deref(), Some("claude=Personal"));
        assert!(
            env.is_empty(),
            "the choice and any stale badge entry are gone"
        );

        add(&mut env, work("Personal"));
        assert_eq!(env["CLAUDE_CONFIG_DIR"], "/w");
        assert!(!env.contains_key("zed_x"));

        let ext = TerminalExt::insert_env(&mut env);
        assert_ne!(env["FYX_TERMINAL_TOKEN"], "forged");
        assert!(
            !env.contains_key(PARKED_ACCOUNTS_ENV),
            "never reaches the program"
        );
        assert_eq!(ext.accounts(), work("Personal").accounts);
    }

    #[test]
    fn a_terminal_without_additions_has_no_badge() {
        let mut env: HashMap<String, String> = HashMap::from_iter([(
            PARKED_ACCOUNTS_ENV.to_owned(),
            "claude\u{1f}Fake\u{1f}red".to_owned(),
        )]);
        take_choice(&mut env);
        add(&mut env, EnvAdditions::default());
        assert!(TerminalExt::insert_env(&mut env).accounts().is_empty());
    }

    #[test]
    fn badges_round_trip_and_separators_are_stripped() {
        let badges = vec![
            AccountBadge {
                tool: "claude".into(),
                name: "Wo\u{1e}rk".into(),
                color: "red".into(),
            },
            AccountBadge {
                tool: "codex".into(),
                name: "Me".into(),
                color: "blue".into(),
            },
        ];
        let decoded = decode_badges(&encode_badges(&badges));
        assert_eq!(decoded[0].name, "Work");
        assert_eq!(decoded[1], badges[1]);
        assert!(decode_badges("").is_empty());
    }
}
