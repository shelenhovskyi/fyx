//! dev-only: drives review panes from a script so the end-to-end check runs without input devices.
//!
//! Off unless `FYX_AGENT_REVIEW_SCRIPT=<path>` is set. One command per line, `#` starts a comment:
//!
//! ```text
//! action agent::NewExternalAgentThread {"agent": "Review Stub"}
//! review              # wait for the next review pane to open
//! accept 1            # hunk numbers start at 1
//! accept-turn         # or reject-turn: every edit of the turn
//! reject 2
//! comment 2 keep it lowercase
//! submit
//! wait 3              # seconds
//! quit
//! ```
//!
//! Decisions go through the same actions the review keys dispatch (`AcceptHunk`, `RejectHunk`,
//! `SubmitReview`), focused on the pane, so the check exercises the pane rather than the model.

use std::{collections::HashSet, path::PathBuf, time::Duration};

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{Action, AnyWindowHandle, App, AsyncApp, Entity};
use workspace::MultiWorkspace;

use crate::{
    AcceptHunk, AcceptTurn, AgentReview, RejectHunk, RejectTurn, ReviewId, ReviewPane, SubmitReview,
};

const SCRIPT_VAR: &str = "FYX_AGENT_REVIEW_SCRIPT";
const POLL: Duration = Duration::from_millis(250);
const REVIEW_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, PartialEq)]
enum Step {
    Action(String, Option<serde_json::Value>),
    Review,
    Accept(usize),
    AcceptTurn,
    RejectTurn,
    Reject(usize),
    Comment(usize, String),
    Submit,
    Wait(u64),
    Quit,
}

pub(crate) fn init(cx: &mut App) {
    let Some(path) = std::env::var_os(SCRIPT_VAR).map(PathBuf::from) else {
        return;
    };
    let steps = match std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))
        .and_then(|script| parse(&script))
    {
        Ok(steps) => steps,
        Err(error) => {
            log::error!("fyx: review script: {error:#}");
            return;
        }
    };
    log::warn!(
        "fyx: review script driver active ({SCRIPT_VAR}): {} steps from {} will decide agent reviews",
        steps.len(),
        path.display()
    );
    cx.spawn(async move |cx| {
        if let Err(error) = run(steps, cx).await {
            log::error!("fyx: review script failed: {error:#}");
        }
    })
    .detach();
}

fn parse(script: &str) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    for (index, line) in script.lines().enumerate() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        let (command, rest) = line.split_once(' ').unwrap_or((line, ""));
        let rest = rest.trim();
        let hunk = |text: &str| -> Result<usize> {
            let number: usize = text
                .parse()
                .with_context(|| format!("bad hunk number {text:?}"))?;
            number.checked_sub(1).context("hunk numbers start at 1")
        };
        let step = match command {
            "action" => {
                let (name, data) = rest.split_once(' ').unwrap_or((rest, ""));
                let data = if data.trim().is_empty() {
                    None
                } else {
                    Some(serde_json::from_str(data).context("action data is not JSON")?)
                };
                Step::Action(name.to_string(), data)
            }
            "review" => Step::Review,
            "accept" => Step::Accept(hunk(rest)?),
            "accept-turn" => Step::AcceptTurn,
            "reject-turn" => Step::RejectTurn,
            "reject" => Step::Reject(hunk(rest)?),
            "comment" => {
                let (number, text) = rest.split_once(' ').unwrap_or((rest, ""));
                Step::Comment(hunk(number)?, text.trim().to_string())
            }
            "submit" => Step::Submit,
            "wait" => Step::Wait(rest.parse().context("bad wait")?),
            "quit" => Step::Quit,
            other => bail!("line {}: unknown command {other:?}", index + 1),
        };
        steps.push(step);
    }
    Ok(steps)
}

async fn run(steps: Vec<Step>, cx: &mut AsyncApp) -> Result<()> {
    let window = wait_for(cx, Duration::from_secs(60), |cx| {
        cx.windows()
            .into_iter()
            .find(|window| window.downcast::<MultiWorkspace>().is_some())
    })
    .await
    .context("no workspace window")?;
    let mut seen = HashSet::new();
    let mut current: Option<(ReviewId, Entity<ReviewPane>)> = None;
    for step in steps {
        log::info!("fyx: review script: {step:?}");
        match step {
            Step::Action(name, data) => {
                let action = cx.update(|cx| cx.build_action(&name, data))?;
                dispatch(window, action, cx)?;
            }
            Step::Review => {
                let found = wait_for(cx, REVIEW_TIMEOUT, |cx| next_pane(window, &seen, cx))
                    .await
                    .context("no review pane opened")?;
                seen.insert(found.0);
                let path = cx.update(|cx| {
                    found
                        .1
                        .read(cx)
                        .edit(cx)
                        .map(|edit| edit.abs_path().display().to_string())
                });
                log::info!("fyx: review script: reviewing {:?} {path:?}", found.0);
                current = Some(found);
            }
            Step::AcceptTurn | Step::RejectTurn => {
                let (_, pane) = current.as_ref().context("turn decision before review")?;
                focus_pane(window, pane, cx)?;
                let action = if matches!(step, Step::AcceptTurn) {
                    AcceptTurn.boxed_clone()
                } else {
                    RejectTurn.boxed_clone()
                };
                dispatch(window, action, cx)?;
            }
            Step::Accept(hunk) => {
                let (_, pane) = current.as_ref().context("accept before review")?;
                select_and_dispatch(window, pane, hunk, AcceptHunk.boxed_clone(), cx)?;
            }
            Step::Reject(hunk) => {
                let (_, pane) = current.as_ref().context("reject before review")?;
                select_and_dispatch(window, pane, hunk, RejectHunk.boxed_clone(), cx)?;
            }
            Step::Comment(hunk, text) => {
                let (_, pane) = current.as_ref().context("comment before review")?;
                window.update(cx, |_, window, cx| {
                    pane.update(cx, |pane, cx| pane.set_comment_text(hunk, text, window, cx))
                })??;
            }
            Step::Submit => {
                let (_, pane) = current.as_ref().context("submit before review")?;
                focus_pane(window, pane, cx)?;
                dispatch(window, SubmitReview.boxed_clone(), cx)?;
            }
            Step::Wait(seconds) => {
                cx.background_executor()
                    .timer(Duration::from_secs(seconds))
                    .await;
            }
            Step::Quit => {
                cx.update(|cx| cx.quit());
                return Ok(());
            }
        }
        // Let the dispatched action and its effects run before the next step.
        cx.background_executor().timer(POLL).await;
    }
    Ok(())
}

async fn wait_for<T>(
    cx: &mut AsyncApp,
    timeout: Duration,
    mut probe: impl FnMut(&mut App) -> Option<T>,
) -> Result<T> {
    let mut waited = Duration::ZERO;
    loop {
        if let Some(found) = cx.update(|cx| probe(cx)) {
            return Ok(found);
        }
        if waited >= timeout {
            return Err(anyhow!("timed out after {timeout:?}"));
        }
        cx.background_executor().timer(POLL).await;
        waited += POLL;
    }
}

fn next_pane(
    window: AnyWindowHandle,
    seen: &HashSet<ReviewId>,
    cx: &mut App,
) -> Option<(ReviewId, Entity<ReviewPane>)> {
    let review = AgentReview::global(cx)?;
    let multi_workspace = window.downcast::<MultiWorkspace>()?;
    let workspace = multi_workspace.read(cx).ok()?.workspace().clone();
    let panes = workspace
        .read(cx)
        .items_of_type::<ReviewPane>(cx)
        .collect::<Vec<_>>();
    panes.into_iter().find_map(|pane| {
        let id = pane.read(cx).current_review_id()?;
        let open = review
            .read(cx)
            .review(id)
            .is_some_and(|review| !review.is_submitted());
        (open && !seen.contains(&id)).then_some((id, pane))
    })
}

fn focus_pane(window: AnyWindowHandle, pane: &Entity<ReviewPane>, cx: &mut AsyncApp) -> Result<()> {
    window.update(cx, |_, window, cx| {
        let focus = pane.read(cx).diff_focus_handle(cx);
        window.focus(&focus, cx);
    })
}

fn select_and_dispatch(
    window: AnyWindowHandle,
    pane: &Entity<ReviewPane>,
    hunk: usize,
    action: Box<dyn Action>,
    cx: &mut AsyncApp,
) -> Result<()> {
    focus_pane(window, pane, cx)?;
    window.update(cx, |_, _, cx| {
        pane.update(cx, |pane, cx| pane.select_hunk(hunk, cx))
    })??;
    dispatch(window, action, cx)
}

fn dispatch(window: AnyWindowHandle, action: Box<dyn Action>, cx: &mut AsyncApp) -> Result<()> {
    window.update(cx, |_, window, cx| window.dispatch_action(action, cx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_script() {
        let steps = parse(
            "# check\naction agent::NewExternalAgentThread {\"agent\": \"Review Stub\"}\nreview\naccept 1\nreject 2 # no\ncomment 2 keep it\nsubmit\nwait 2\nquit\n",
        )
        .unwrap();
        assert_eq!(
            steps,
            [
                Step::Action(
                    "agent::NewExternalAgentThread".into(),
                    Some(serde_json::json!({"agent": "Review Stub"}))
                ),
                Step::Review,
                Step::Accept(0),
                Step::Reject(1),
                Step::Comment(1, "keep it".into()),
                Step::Submit,
                Step::Wait(2),
                Step::Quit,
            ]
        );
        assert!(parse("accept 0").is_err());
        assert!(parse("dance").is_err());
    }
}
