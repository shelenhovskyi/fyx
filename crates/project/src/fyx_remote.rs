//! fyx: plan 5.17. The `ssh` command `terminal_host`'s remote tunnel runs for an SSH project:
//! remote forwards to Fyx's listeners and a script that writes the discovery files on the host.

use std::collections::HashMap;

use anyhow::Result;
use gpui::{App, Entity, EntityId};
use remote::{Interactive, RemoteClient, RemoteConnectionOptions};

use crate::Project;

/// A command to run locally with piped stdin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TunnelCommand {
    /// The `RemoteClient` entity: the tunnel's key and what remote shells are matched by.
    pub remote: EntityId,
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// `project`'s SSH connection to a Unix-like host: its `RemoteClient` entity and the host's name
/// (`user@host:port`). `None` for local projects and other connections (WSL, Docker, Windows
/// hosts).
pub fn ssh_host(project: &Project, cx: &App) -> Option<(EntityId, String)> {
    let remote_client = project.remote_client()?;
    let client = ssh_posix(&remote_client, cx)?;
    Some((
        remote_client.entity_id(),
        client.connection_options().display_name(),
    ))
}

fn ssh_posix<'a>(remote_client: &'a Entity<RemoteClient>, cx: &'a App) -> Option<&'a RemoteClient> {
    let client = remote_client.read(cx);
    (matches!(client.connection_options(), RemoteConnectionOptions::Ssh(_))
        && client.path_style().is_posix())
    .then_some(client)
}

/// For an SSH connection to a Unix-like host: `ssh -o ExitOnForwardFailure=yes -o BatchMode=yes
/// -R 127.0.0.1:<port>:127.0.0.1:<port>… <dest> sh -c <script>`. `ssh` exits before `script`
/// runs unless every forward is set up, and never asks for a password. `None` for other
/// connections (WSL, Docker, Windows hosts).
pub fn ssh_tunnel_command(
    project: &Project,
    script: &str,
    ports: &[u16],
    cx: &App,
) -> Option<Result<TunnelCommand>> {
    let remote_client = project.remote_client()?;
    let client = ssh_posix(&remote_client, cx)?;
    let command = match client.build_command(
        Some("sh".to_owned()),
        &["-c".to_owned(), script.to_owned()],
        &HashMap::default(),
        None,
        None,
        Interactive::No,
    ) {
        Ok(command) => command,
        Err(error) => return Some(Err(error)),
    };
    let mut args = Vec::with_capacity(command.args.len() + 4 + 2 * ports.len());
    for option in ["ExitOnForwardFailure=yes", "BatchMode=yes"] {
        args.extend(["-o".to_owned(), option.to_owned()]);
    }
    for port in ports {
        args.extend([
            "-R".to_owned(),
            format!("127.0.0.1:{port}:127.0.0.1:{port}"),
        ]);
    }
    args.extend(command.args);
    Some(Ok(TunnelCommand {
        remote: remote_client.entity_id(),
        program: command.program,
        args,
        env: command.env.into_iter().collect(),
    }))
}
