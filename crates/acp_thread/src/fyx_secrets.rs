//! fyx: hook that lets the host put secrets into the processes an agent starts (plan 7.4).
//!
//! The agent writes `secret://name` in a terminal command or in the environment it asks for. The
//! host (`agent_review`'s vault) asks the user, then puts the value into the process environment
//! only: a reference in the command becomes a reference to an environment variable of the shell.
//! The output the agent reads back shows `secret://name` wherever the value or a common encoding of
//! it appears. Without a host hook, commands run exactly as upstream.

use std::rc::Rc;

use agent_client_protocol::v2::schema as acp_v2;
use collections::HashMap;
use gpui::{App, AsyncApp, Global, Task};
use util::shell::ShellKind;

/// The scheme of a secret reference.
pub const SECRET_SCHEME: &str = "secret://";

/// A process an agent is about to start, as the host sees it before and after resolution.
pub struct AgentProcess {
    /// The thread's session, when the process belongs to one the host can name.
    pub session_id: Option<acp_v2::SessionId>,
    /// The command and its arguments, as they go to the shell.
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    /// How the shell that runs the command reads an environment variable, such as
    /// `${NAME}` or `$env:NAME`.
    pub env_reference: Rc<dyn Fn(&str) -> String>,
}

pub trait AgentSecrets: 'static {
    /// Puts the values of the secrets `process` refers to into it, after the user allowed it.
    /// Unknown or denied references stay as they are.
    fn resolve(&self, process: AgentProcess, cx: &mut App) -> Task<AgentProcess>;

    /// `text` with every value used so far, and its encodings, replaced by its reference, or
    /// `None` when it holds none.
    fn redact(&self, text: &str, cx: &App) -> Option<String>;
}

struct GlobalAgentSecrets(Rc<dyn AgentSecrets>);

impl Global for GlobalAgentSecrets {}

pub fn set_agent_secrets(secrets: Rc<dyn AgentSecrets>, cx: &mut App) {
    cx.set_global(GlobalAgentSecrets(secrets));
}

/// How `shell` expands the environment variable `name` inside or outside double quotes.
pub fn env_reference(shell: ShellKind, name: &str) -> String {
    match shell {
        ShellKind::Posix | ShellKind::Csh | ShellKind::Tcsh => format!("${{{name}}}"),
        ShellKind::Fish | ShellKind::Rc | ShellKind::Xonsh => format!("${name}"),
        ShellKind::Elvish => format!("$E:{name}"),
        ShellKind::PowerShell | ShellKind::Pwsh => format!("${{env:{name}}}"),
        ShellKind::Nushell => format!("$env.{name}"),
        ShellKind::Cmd => format!("%{name}%"),
    }
}

fn mentions_secret(command: &str, args: &[String], env: &HashMap<String, String>) -> bool {
    command.contains(SECRET_SCHEME)
        || args.iter().any(|arg| arg.contains(SECRET_SCHEME))
        || env.values().any(|value| value.contains(SECRET_SCHEME))
}

/// The command, arguments and environment to start a process with, secrets resolved.
pub(crate) async fn resolve_agent_secrets(
    session_id: Option<acp_v2::SessionId>,
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    shell: ShellKind,
    cx: &mut AsyncApp,
) -> (String, Vec<String>, HashMap<String, String>) {
    let hook = cx.update(|cx| {
        cx.try_global::<GlobalAgentSecrets>()
            .map(|hook| hook.0.clone())
    });
    let Some(hook) = hook.filter(|_| mentions_secret(&command, &args, &env)) else {
        return (command, args, env);
    };
    let process = AgentProcess {
        session_id,
        command,
        args,
        env,
        env_reference: Rc::new(move |name| env_reference(shell, name)),
    };
    let resolved = cx.update(|cx| hook.resolve(process, cx)).await;
    (resolved.command, resolved.args, resolved.env)
}

/// Output on its way to the agent, with secret values replaced by their references.
pub fn redact_agent_secrets(text: String, cx: &App) -> String {
    redact_agent_text(&text, cx).unwrap_or(text)
}

/// `text` with secret values replaced by their references, or `None` when it holds none (or no
/// host hook is installed).
pub fn redact_agent_text(text: &str, cx: &App) -> Option<String> {
    cx.try_global::<GlobalAgentSecrets>()?.0.redact(text, cx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_shell_reads_the_variable_its_own_way() {
        assert_eq!(env_reference(ShellKind::Posix, "X"), "${X}");
        assert_eq!(env_reference(ShellKind::Pwsh, "X"), "${env:X}");
        assert_eq!(env_reference(ShellKind::Cmd, "X"), "%X%");
        assert_eq!(env_reference(ShellKind::Nushell, "X"), "$env.X");
    }
}
