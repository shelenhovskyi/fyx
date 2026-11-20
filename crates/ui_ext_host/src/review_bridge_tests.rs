use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use agent_review::{AgentReview, HunkDecision};
use fs::{FakeFs, Fs as _};
use gpui::{Entity, EntityId, TestAppContext};
use project::Project;
use serde_json::json;
use ui_ext::{
    process::FileRef,
    review::{Decision, Proposal, Proposed, ReviewOutcome},
};
use util::path;

use super::*;
use crate::worktrees::{OpenWorktree, OpenWorktrees};
use ui_ext::review::{CommentState, ReviewComments};

const WORKSPACE: u64 = 1;

type Outcomes = Arc<Mutex<Vec<(u64, Result<ReviewOutcome, String>)>>>;

async fn setup(review_mode: bool, cx: &mut TestAppContext) -> (Arc<FakeFs>, Entity<Project>) {
    crate::tests::init_test(cx);
    cx.update(|cx| agent_review::init(None, cx));
    let review = cx.update(|cx| AgentReview::global(cx)).unwrap();
    review.update(cx, |review, cx| review.set_enabled(review_mode, cx));
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree(
        path!("/work"),
        json!({ "a.txt": "one\ntwo\nthree\n", ".env": "SECRET=1\n" }),
    )
    .await;
    let project = Project::test(fs.clone(), [Path::new(path!("/work"))], cx).await;
    cx.update(|cx| {
        let worktree_id = project.read(cx).worktrees(cx).next().unwrap().read(cx).id();
        OpenWorktrees::global(cx).set(
            EntityId::from(WORKSPACE),
            vec![OpenWorktree {
                key: (WORKSPACE, worktree_id.to_proto()),
                root: PathBuf::from(path!("/work")),
                trusted: true,
            }],
        );
        crate::workspace_edit::register_workspace(
            EntityId::from(WORKSPACE),
            None,
            project.downgrade(),
            cx,
        );
    });
    cx.run_until_parked();
    (fs, project)
}

fn proposal(path: &str, change: Proposed, apply: bool) -> Proposal {
    Proposal {
        path: FileRef {
            worktree: None,
            path: path.into(),
        },
        change,
        title: "Fix".into(),
        session: "s1".into(),
        apply,
    }
}

fn sink(outcomes: &Outcomes) -> ReviewSink {
    let outcomes = outcomes.clone();
    Arc::new(move |id, outcome| outcomes.lock().unwrap().push((id, outcome)))
}

async fn propose(
    id: u64,
    proposal: Proposal,
    outcomes: &Outcomes,
    open: &OpenReviews,
    cx: &mut TestAppContext,
) -> Result<(), String> {
    let (sink, open) = (sink(outcomes), open.clone());
    let result = cx
        .spawn(async move |mut cx: gpui::AsyncApp| {
            open_review("ext", "ext", id, proposal, sink, &open, &mut cx).await
        })
        .await;
    cx.run_until_parked();
    result
}

fn host(cx: &mut TestAppContext) -> ExtensionReviews {
    ExtensionReviews {
        key: "ext".into(),
        main: cx.update(MainThread::global),
        open: OpenReviews::default(),
        watch: NEXT_WATCH.fetch_add(1, Ordering::Relaxed),
        watching: Mutex::new(false),
    }
}

fn review(cx: &mut TestAppContext) -> Entity<AgentReview> {
    cx.update(|cx| AgentReview::global(cx)).unwrap()
}

fn decide(decision: HunkDecision, cx: &mut TestAppContext) {
    let review = review(cx);
    review.update(cx, |review, cx| {
        let id = review.reviews()[0].id();
        review.decide_all(id, decision, cx).unwrap();
        review.submit(id, cx).unwrap();
    });
    cx.run_until_parked();
}

async fn buffer_text(project: &Entity<Project>, cx: &mut TestAppContext) -> String {
    let buffer = project
        .update(cx, |project, cx| {
            project.open_local_buffer(path!("/work/a.txt"), cx)
        })
        .await
        .unwrap();
    buffer.read_with(cx, |buffer, _| buffer.text())
}

/// An accepted diff with `apply` lands in the buffer as one undo step and is not saved; the
/// extension hears `accepted` with the text.
#[gpui::test]
async fn test_accepted_diff_applies_to_the_buffer(cx: &mut TestAppContext) {
    let (fs, project) = setup(true, cx).await;
    let (outcomes, open) = (Outcomes::default(), OpenReviews::default());
    let diff = "@@ -2,1 +2,1 @@\n-two\n+TWO\n".to_string();
    propose(
        7,
        proposal("a.txt", Proposed::UnifiedDiff(diff), true),
        &outcomes,
        &open,
        cx,
    )
    .await
    .unwrap();
    assert_eq!(
        review(cx).read_with(cx, |review, _| review.reviews().len()),
        1
    );
    assert!(
        outcomes.lock().unwrap().is_empty(),
        "the user decides first"
    );
    decide(HunkDecision::Accepted, cx);
    let outcome = outcomes.lock().unwrap().pop().unwrap();
    assert_eq!(
        outcome,
        (
            7,
            Ok(ReviewOutcome {
                decision: Decision::Accepted,
                final_content: "one\nTWO\nthree\n".into(),
            })
        )
    );
    assert_eq!(buffer_text(&project, cx).await, "one\nTWO\nthree\n");
    let on_disk = fs.load(Path::new(path!("/work/a.txt"))).await.unwrap();
    assert_eq!(on_disk, "one\ntwo\nthree\n", "never saved");
    assert!(open.lock().unwrap().is_empty());
}

/// A rejection touches nothing and says `rejected` with the old text; without `apply` an
/// accepted proposal leaves the buffer alone too.
#[gpui::test]
async fn test_rejected_and_unapplied(cx: &mut TestAppContext) {
    let (_, project) = setup(true, cx).await;
    let (outcomes, open) = (Outcomes::default(), OpenReviews::default());
    let new = Proposed::NewContent("one\n2\nthree\n".into());
    propose(
        1,
        proposal("a.txt", new.clone(), true),
        &outcomes,
        &open,
        cx,
    )
    .await
    .unwrap();
    decide(HunkDecision::Rejected, cx);
    let (_, outcome) = outcomes.lock().unwrap().pop().unwrap();
    assert_eq!(outcome.unwrap().decision, Decision::Rejected);
    assert_eq!(buffer_text(&project, cx).await, "one\ntwo\nthree\n");

    propose(2, proposal("a.txt", new, false), &outcomes, &open, cx)
        .await
        .unwrap();
    decide(HunkDecision::Accepted, cx);
    let (id, outcome) = outcomes.lock().unwrap().pop().unwrap();
    let outcome = outcome.unwrap();
    assert_eq!((id, outcome.decision), (2, Decision::Accepted));
    assert_eq!(outcome.final_content, "one\n2\nthree\n");
    assert_eq!(
        buffer_text(&project, cx).await,
        "one\ntwo\nthree\n",
        "apply is false"
    );
}

/// Withdrawing and closing close the review without an outcome; withdraw returns the proposal.
#[gpui::test]
async fn test_withdraw_and_close(cx: &mut TestAppContext) {
    let (_, _project) = setup(true, cx).await;
    let host = host(cx);
    let outcomes = Outcomes::default();
    let new = Proposed::NewContent("1\n".into());
    propose(
        1,
        proposal("a.txt", new.clone(), true),
        &outcomes,
        &host.open,
        cx,
    )
    .await
    .unwrap();
    propose(2, proposal("a.txt", new, true), &outcomes, &host.open, cx)
        .await
        .unwrap();
    assert_eq!(
        review(cx).read_with(cx, |review, _| review.reviews().len()),
        2
    );
    // `ReviewHost::withdraw` waits for the main thread: here the job runs inline.
    let withdrawn = cx.update(|cx| withdraw(&host.open, 1, cx));
    assert_eq!(
        withdrawn,
        Ok("one\ntwo\nthree\n".into()),
        "nothing accepted yet"
    );
    assert!(
        cx.update(|cx| withdraw(&host.open, 1, cx)).is_err(),
        "only once"
    );
    cx.run_until_parked();
    assert_eq!(
        review(cx).read_with(cx, |review, _| review.reviews().len()),
        1
    );
    host.close("ext", &[2, 99]);
    cx.run_until_parked();
    assert_eq!(
        review(cx).read_with(cx, |review, _| review.reviews().len()),
        0
    );
    assert!(outcomes.lock().unwrap().is_empty(), "no outcome for either");
}

/// Comments sent on an extension's proposals reach its instance as `review-comments`: those sent
/// before it watches when it starts, later ones on `CommentsChanged`; a gone instance stops.
#[gpui::test]
async fn test_review_comments_reach_the_instance(cx: &mut TestAppContext) {
    let (_, _project) = setup(true, cx).await;
    let host = host(cx);
    let outcomes = Outcomes::default();

    let comment = |index: usize, text: &str, cx: &mut TestAppContext| {
        review(cx).update(cx, |review, cx| {
            let id = review.reviews()[index].id();
            review.set_comment(id, 0, Some(text.into()), cx).unwrap();
            review
                .send_comments(&session_key("ext", "s1").into(), cx)
                .unwrap();
        });
        cx.run_until_parked();
    };
    let new = Proposed::NewContent("1\ntwo\nthree\n".into());
    propose(
        1,
        proposal("a.txt", new.clone(), false),
        &outcomes,
        &host.open,
        cx,
    )
    .await
    .unwrap();
    comment(0, "why a digit?", cx);

    let received = Arc::new(Mutex::new(Vec::<ReviewComments>::new()));
    let sink = received.clone();
    host.watch_comments(
        "ext",
        Arc::new(move |comments| sink.lock().unwrap().push(comments)),
    );
    cx.run_until_parked();
    {
        let received = received.lock().unwrap();
        assert_eq!(received.len(), 1, "the waiting batch, at once");
        assert_eq!(received[0].session, "s1");
        let [comment] = received[0].comments.as_slice() else {
            panic!("one comment: {:?}", received[0].comments);
        };
        assert_eq!(comment.path.path, "a.txt");
        assert!(comment.path.worktree.is_some(), "the extension's own id");
        assert_eq!(comment.lines, Some((1, 1)));
        assert_eq!(comment.state, CommentState::Pending);
        assert_eq!(comment.text, "why a digit?");
        assert!(received[0].prompt.contains("why a digit?"));
    }

    propose(2, proposal("a.txt", new, false), &outcomes, &host.open, cx)
        .await
        .unwrap();
    comment(1, "and here?", cx);
    assert_eq!(received.lock().unwrap().len(), 2, "on CommentsChanged");

    drop(host);
    cx.run_until_parked();
    assert!(
        cx.update(|cx| cx.global::<CommentsRouter>().watchers().is_empty()),
        "a dropped load stops watching"
    );
}

/// Always reviewed: with review mode off, and with `auto_accept` matching every file (W5's
/// `propose`, coordinator decision).
#[gpui::test]
async fn test_always_reviewed(cx: &mut TestAppContext) {
    let (_, _project) = setup(false, cx).await;
    review(cx).update(cx, |review, cx| {
        review.set_user_auto_accept(&["**".to_string()], cx)
    });
    let (outcomes, open) = (Outcomes::default(), OpenReviews::default());
    let new = Proposed::NewContent("x\n".into());
    propose(1, proposal("a.txt", new, true), &outcomes, &open, cx)
        .await
        .unwrap();
    assert_eq!(
        review(cx).read_with(cx, |review, _| review.reviews().len()),
        1,
        "a review opened"
    );
    assert!(outcomes.lock().unwrap().is_empty(), "nothing decided");
}

/// Refused before a review opens: a diff that does not apply, a private file.
#[gpui::test]
async fn test_refusals(cx: &mut TestAppContext) {
    let (_, _project) = setup(true, cx).await;
    let (outcomes, open) = (Outcomes::default(), OpenReviews::default());
    let new = || Proposed::NewContent("x\n".into());
    let diff = Proposed::UnifiedDiff("@@ -1,1 +1,1 @@\n-uno\n+1\n".into());
    let stale = propose(2, proposal("a.txt", diff, true), &outcomes, &open, cx).await;
    assert!(stale.unwrap_err().contains("the diff does not apply"));
    let private = propose(3, proposal(".env", new(), true), &outcomes, &open, cx).await;
    assert!(private.is_err(), "private_files");
    assert_eq!(
        review(cx).read_with(cx, |review, _| review.reviews().len()),
        0
    );
    assert!(open.lock().unwrap().is_empty());
}

#[test]
fn test_changed_range() {
    assert_eq!(changed_range("abc", "abc"), (3..3, 3..3));
    assert_eq!(changed_range("one\ntwo\n", "one\nTWO\n"), (4..7, 4..7));
    assert_eq!(changed_range("a", "ab"), (1..1, 1..2));
    // "é" and "è" share their first byte: the range starts at the character.
    assert_eq!(changed_range("xé", "xè"), (1..3, 1..3));
    assert_eq!(changed_range("aa", "a"), (1..2, 1..1));
}
