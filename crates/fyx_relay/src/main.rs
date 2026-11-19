//! `fyx-relay --channel <extension-id>/<name> [args…]`: a command hook's way into the running Fyx
//! (design 0.4 §B.9). Prints the extension's answer and exits with its code; when Fyx or the
//! extension does not answer, exits with the channel's `on_failure` (default 0) and prints nothing.

use std::io::Write as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    let answer = fyx_relay::run_from_env(std::env::args().skip(1), std::io::stdin().lock());
    std::io::stdout().write_all(answer.stdout.as_bytes()).ok();
    std::io::stderr().write_all(answer.stderr.as_bytes()).ok();
    ExitCode::from(answer.exit)
}
