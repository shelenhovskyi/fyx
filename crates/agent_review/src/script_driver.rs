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
//! prompt status       # send a user message in the reviewed session (as typed in the panel)
//! operation delete a.txt          # the built-in agent's file operations, through the review;
//! operation move a.txt b.txt      # paths are relative to the reviewed file's directory
//! wait 3              # seconds
//! quit
//! ```
//!
//! Decisions go through the same actions the review keys dispatch (`AcceptHunk`, `RejectHunk`,
//! `SubmitReview`), focused on the pane, so the check exercises the pane rather than the model.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    time::Duration,
};

use acp_thread::fyx_review::AgentFileOperation;

use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{Action, AnyWindowHandle, App, AsyncApp, Entity};
use workspace::MultiWorkspace;

use crate::{
    AcceptHunk, AgentReview, HunkDecision, OpenReviews, RejectHunk, ReviewId, ReviewPane,
    SubmitReview,
};

const SCRIPT_VAR: &str = "FYX_AGENT_REVIEW_SCRIPT";
const POLL: Duration = Duration::from_millis(250);
const REVIEW_TIMEOUT: Duration = Duration::from_secs(60);
/// The actions a script may dispatch: starting an agent thread and opening things. Everything else,
/// settings included, stays out of reach.
const ALLOWED_ACTIONS: &[&str] = &["agent::NewExternalAgentThread", "workspace::Open"];

#[derive(Debug, PartialEq)]
enum Step {
    Action(String, Option<serde_json::Value>),
    Review,
    Accept(usize),
    AcceptTurn,
    RejectTurn,
    Reject(usize),
    Comment(usize, String),
    Prompt(String),
    Operation(String, Vec<String>),
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
            "prompt" => Step::Prompt(rest.to_string()),
            "operation" => {
                let mut words = rest.split_whitespace().map(str::to_string);
                let kind = words.next().context("operation needs a kind")?;
                let paths = words.collect::<Vec<_>>();
                let expected = if kind == "delete" { 1 } else { 2 };
                anyhow::ensure!(
                    matches!(kind.as_str(), "delete" | "move" | "copy") && paths.len() == expected,
                    "line {}: operation delete <path> | move <from> <to> | copy <from> <to>",
                    index + 1
                );
                Step::Operation(kind, paths)
            }
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
    let mut last_session = None;
    let mut last_directory: Option<PathBuf> = None;
    let mut current: Option<(ReviewId, Entity<ReviewPane>)> = None;
    for step in steps {
        log::info!("fyx: review script: {step:?}");
        match step {
            Step::Action(name, data) => {
                anyhow::ensure!(
                    action_allowed(&name),
                    "action {name} is not allowed in a review script"
                );
                let action = cx.update(|cx| cx.build_action(&name, data))?;
                dispatch(window, action, cx)?;
            }
            Step::Review => {
                // A new review only shows a toast; the script clicks its Review, as the user would.
                let found = wait_for(cx, REVIEW_TIMEOUT, |cx| {
                    next_pane(window, &seen, cx).or_else(|| {
                        window
                            .update(cx, |_, window, cx| {
                                window.dispatch_action(OpenReviews.boxed_clone(), cx)
                            })
                            .ok();
                        None
                    })
                })
                .await
                .context("no review opened")?;
                seen.insert(found.0);
                cx.update(|cx| {
                    if let Some(review) = AgentReview::global(cx)
                        && let Some(reviewed) = review.read(cx).review(found.0)
                    {
                        last_session = Some(reviewed.session_id().clone());
                        last_directory = reviewed.edit().abs_path().parent().map(Path::to_path_buf);
                    }
                });
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
                // As if the user confirmed the pane's "accept/reject all N waiting files?".
                let (_, pane) = current.as_ref().context("turn decision before review")?;
                let decision = if matches!(step, Step::AcceptTurn) {
                    HunkDecision::Accepted
                } else {
                    HunkDecision::Rejected
                };
                cx.update(|cx| -> Result<()> {
                    let turn = pane.read(cx).turn();
                    let review = AgentReview::global(cx).context("no review store")?;
                    review.update(cx, |review, cx| review.decide_turn(turn, decision, cx))
                })?;
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
            Step::Prompt(text) => {
                let (review_id, _) = current.as_ref().context("prompt before review")?;
                let review_id = *review_id;
                cx.update(|cx| -> Result<()> {
                    let review = AgentReview::global(cx).context("no review store")?;
                    let session_id = review
                        .read(cx)
                        .review(review_id)
                        .map(|review| review.session_id().clone())
                        .or_else(|| last_session.clone())
                        .context("the reviewed session is gone")?;
                    let thread = review
                        .read(cx)
                        .thread_for_session(&session_id)
                        .context("the reviewed thread is gone")?;
                    let submission = thread.update(cx, |thread, cx| {
                        thread.send(
                            vec![agent_client_protocol::v2::schema::ContentBlock::Text(
                                agent_client_protocol::v2::schema::TextContent::new(text),
                            )],
                            cx,
                        )
                    });
                    cx.spawn(async move |_| {
                        if let Err(error) = submission.response.await {
                            log::error!("fyx: review script: prompt failed: {error:#}");
                        }
                    })
                    .detach();
                    Ok(())
                })?;
            }
            Step::Operation(kind, paths) => {
                let (review_id, _) = current.as_ref().context("operation before review")?;
                let review_id = *review_id;
                cx.update(|cx| -> Result<()> {
                    let review = AgentReview::global(cx).context("no review store")?;
                    let (session_id, directory) = {
                        let review = review.read(cx);
                        let reviewed = review.review(review_id);
                        let session_id = reviewed
                            .map(|review| review.session_id().clone())
                            .or_else(|| last_session.clone())
                            .context("the reviewed session is gone")?;
                        let directory = reviewed
                            .and_then(|review| {
                                review.edit().abs_path().parent().map(Path::to_path_buf)
                            })
                            .or_else(|| last_directory.clone())
                            .context("no directory to resolve the paths in")?;
                        (session_id, directory)
                    };
                    let path = |index: usize| directory.join(&paths[index]);
                    let operation = match kind.as_str() {
                        "delete" => AgentFileOperation::Delete {
                            path: path(0),
                            text: std::fs::read_to_string(path(0)).ok(),
                        },
                        "move" => AgentFileOperation::Move {
                            from: path(0),
                            to: path(1),
                        },
                        _ => AgentFileOperation::Copy {
                            from: path(0),
                            to: path(1),
                        },
                    };
                    let description = operation.describe();
                    let decided = review.update(cx, |review, cx| {
                        review.review_operation(session_id, operation, cx)
                    });
                    cx.spawn(async move |_| match decided.await {
                        Ok(()) => log::info!("fyx: review script: allowed: {description}"),
                        Err(error) => {
                            log::info!("fyx: review script: stopped: {description}: {error:#}")
                        }
                    })
                    .detach();
                    Ok(())
                })?;
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

fn action_allowed(name: &str) -> bool {
    ALLOWED_ACTIONS.contains(&name)
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
        assert_eq!(
            parse(
                "prompt status
operation delete a.txt
operation copy a.txt b.txt"
            )
            .unwrap(),
            [
                Step::Prompt("status".into()),
                Step::Operation("delete".into(), vec!["a.txt".into()]),
                Step::Operation("copy".into(), vec!["a.txt".into(), "b.txt".into()]),
            ]
        );
        assert!(parse("operation move a.txt").is_err());
        assert!(parse("operation chmod a.txt").is_err());
        let script = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/test-agent/e2e-full.script"
        ))
        .unwrap();
        parse(&script).expect("the checked-in end-to-end script parses");
        assert!(action_allowed("agent::NewExternalAgentThread"));
        assert!(!action_allowed("agent_review::ToggleReviewMode"));
        assert!(!action_allowed("zed::OpenSettings"));
        assert!(parse("dance").is_err());
    }
}
