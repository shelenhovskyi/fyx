use std::path::{Path, PathBuf};

use fs::FakeFs;
use gpui::{BorrowAppContext as _, Entity, EntityId, TestAppContext};
use language::Buffer;
use project::Project;
use serde_json::json;
use ui_ext::process::FileRef;
use ui_ext_wasm::edit::{
    EditError, EditTarget, FileEdit, Position, TextEdit, TextRange, WorkspaceEdit,
};
use util::{path, rel_path::rel_path};

use super::*;
use crate::worktrees::{OpenWorktree, OpenWorktrees};

const WORKSPACE: u64 = 1;

async fn setup(
    trusted: bool,
    cx: &mut TestAppContext,
) -> (std::sync::Arc<FakeFs>, Entity<Project>) {
    crate::tests::init_test(cx);
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree(
        path!("/work"),
        json!({
            "a.txt": "one\ntwo\n",
            "b.txt": "x\n",
            ".env": "SECRET=1\n",
            "dir": { "c.txt": "c\n" },
        }),
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
                trusted,
            }],
        );
        register_workspace(EntityId::from(WORKSPACE), None, project.downgrade(), cx);
    });
    cx.run_until_parked();
    (fs, project)
}

fn at(line: u32, column: u32) -> Position {
    Position { line, column }
}

fn file(path: &str, edits: &[(Position, Position, &str)]) -> FileEdit {
    FileEdit {
        target: EditTarget::File(FileRef {
            worktree: None,
            path: path.into(),
        }),
        expected_version: None,
        create: false,
        edits: edits
            .iter()
            .map(|(start, end, text)| TextEdit {
                range: TextRange {
                    start: *start,
                    end: *end,
                },
                new_text: (*text).into(),
            })
            .collect(),
    }
}

fn edit(files: Vec<FileEdit>) -> WorkspaceEdit {
    WorkspaceEdit {
        label: "Rename".into(),
        files,
    }
}

async fn run(edit: WorkspaceEdit, cx: &mut TestAppContext) -> Result<Applied, EditError> {
    let result = cx
        .spawn(async move |mut cx: gpui::AsyncApp| {
            apply_edit("ext", edit, &|| false, &mut cx).await
        })
        .await;
    cx.run_until_parked();
    result
}

async fn buffer(project: &Entity<Project>, path: &str, cx: &mut TestAppContext) -> Entity<Buffer> {
    let worktree_id = cx.update(|cx| project.read(cx).worktrees(cx).next().unwrap().read(cx).id());
    project
        .update(cx, |project, cx| {
            project.open_buffer((worktree_id, rel_path(path)), cx)
        })
        .await
        .unwrap()
}

fn text(buffer: &Entity<Buffer>, cx: &mut TestAppContext) -> String {
    buffer.read_with(cx, |buffer, _| buffer.text())
}

/// Plan 4.8's done-when: one undo reverts the edit, and the typing just before it stays.
#[gpui::test]
async fn test_one_undo_reverts_the_edit_not_the_typing(cx: &mut TestAppContext) {
    let (_, project) = setup(true, cx).await;
    let a = buffer(&project, "a.txt", cx).await;
    a.update(cx, |buffer, cx| buffer.edit([(0..0, "typed ")], None, cx));
    let applied = run(
        edit(vec![file("a.txt", &[(at(1, 0), at(1, 3), "TWO")])]),
        cx,
    )
    .await
    .unwrap();
    assert_eq!(text(&a, cx), "typed one\nTWO\n");
    assert_eq!(applied.transaction.0.len(), 1);
    a.update(cx, |buffer, cx| buffer.undo(cx));
    assert_eq!(
        text(&a, cx),
        "typed one\ntwo\n",
        "one undo, the typing stays"
    );
    assert!(a.read_with(cx, |buffer, _| buffer.is_dirty()), "not saved");
}

#[gpui::test]
async fn test_several_files_and_undo(cx: &mut TestAppContext) {
    let (_, project) = setup(true, cx).await;
    let applied = run(
        edit(vec![
            file("a.txt", &[(at(0, 0), at(0, 3), "ONE")]),
            file("dir/c.txt", &[(at(0, 0), at(0, 1), "C")]),
        ]),
        cx,
    )
    .await
    .unwrap();
    let a = buffer(&project, "a.txt", cx).await;
    let c = buffer(&project, "dir/c.txt", cx).await;
    assert_eq!(text(&a, cx), "ONE\ntwo\n");
    assert_eq!(text(&c, cx), "C\n");
    cx.update(|cx| undo(&applied.transaction, cx));
    assert_eq!(text(&a, cx), "one\ntwo\n");
    assert_eq!(text(&c, cx), "c\n");
}

/// Q5: the Show/Undo card is for an edit of several files or of a file no editor shows; an edit
/// of the one file the user has open needs none.
#[gpui::test]
async fn test_card_only_for_several_or_unopened_files(cx: &mut TestAppContext) {
    let (_, project) = setup(true, cx).await;
    let (workspace, vcx) =
        cx.add_window_view(|window, cx| Workspace::test_new(project.clone(), window, cx));
    let one = |text: &str| edit(vec![file("a.txt", &[(at(0, 0), at(0, 3), text)])]);
    let card = |applied: &Applied, vcx: &mut gpui::VisualTestContext| {
        workspace.read_with(vcx, |workspace, cx| {
            needs_card(&applied.transaction, workspace, cx)
        })
    };

    let applied = run(one("ONE"), vcx).await.unwrap();
    assert!(card(&applied, vcx), "a file no editor shows");

    let a = buffer(&project, "a.txt", vcx).await;
    workspace.update_in(vcx, |workspace, window, cx| {
        let editor = cx.new(|cx| Editor::for_buffer(a, Some(project.clone()), window, cx));
        workspace.add_item_to_active_pane(Box::new(editor), None, true, window, cx);
    });
    let applied = run(one("Uno"), vcx).await.unwrap();
    assert!(!card(&applied, vcx), "the open file");

    let applied = run(
        edit(vec![
            file("a.txt", &[(at(0, 0), at(0, 3), "one")]),
            file("b.txt", &[(at(0, 0), at(0, 1), "y")]),
        ]),
        vcx,
    )
    .await
    .unwrap();
    assert!(card(&applied, vcx), "several files, one of them open");
}

/// One refused file refuses the whole edit: nothing is applied to the others.
#[gpui::test]
async fn test_a_private_file_refuses_the_whole_edit(cx: &mut TestAppContext) {
    let (_, project) = setup(true, cx).await;
    let result = run(
        edit(vec![
            file("a.txt", &[(at(0, 0), at(0, 3), "ONE")]),
            file(".env", &[(at(0, 0), at(0, 6), "LEAKED")]),
        ]),
        cx,
    )
    .await;
    assert!(
        matches!(&result, Err(EditError::NotAllowed(message)) if message.contains("private_files")),
        "{:?}",
        result.err()
    );
    let a = buffer(&project, "a.txt", cx).await;
    assert_eq!(text(&a, cx), "one\ntwo\n");
}

/// Design wit-0.4 §C.6 rule 5: an account profile's dir is off limits to extensions, also when
/// it lies inside an open worktree. Edits, new files in it and the host-side resolve of a file
/// target are refused, and the reason names the account, never the dir.
#[gpui::test]
async fn test_account_dirs_are_refused(cx: &mut TestAppContext) {
    let (fs, project) = setup(true, cx).await;
    fs.insert_tree(
        path!("/work/accounts/work"),
        json!({ ".credentials.json": "{}\n" }),
    )
    .await;
    cx.update(|cx| {
        cx.update_global::<settings::SettingsStore, _>(|store, cx| {
            store.update_user_settings(cx, |settings| {
                let accounts = settings
                    .fyx
                    .get_or_insert_default()
                    .accounts
                    .get_or_insert_default();
                accounts.tools = Some(
                    [(
                        "claude".to_string(),
                        settings::FyxAccountToolContent {
                            env: Some("CLAUDE_CONFIG_DIR".into()),
                            ..Default::default()
                        },
                    )]
                    .into_iter()
                    .collect(),
                );
                accounts.profiles = Some(vec![settings::FyxAccountProfileContent {
                    name: Some("Work".into()),
                    tool: Some("claude".into()),
                    dir: Some(path!("/work/accounts/work").into()),
                    color: None,
                }]);
            });
        });
    });
    cx.run_until_parked();
    let refused = |result: &Result<Applied, EditError>| {
        matches!(result, Err(EditError::NotAllowed(message))
            if message.contains("account dir of \"Work\"") && !message.contains(path!("/work/accounts")))
    };

    let result = run(
        edit(vec![
            file("a.txt", &[(at(0, 0), at(0, 3), "ONE")]),
            file(
                "accounts/work/.credentials.json",
                &[(at(0, 0), at(0, 2), "")],
            ),
        ]),
        cx,
    )
    .await;
    assert!(refused(&result), "{:?}", result.err());
    let mut create = file("accounts/work/new.json", &[(at(0, 0), at(0, 0), "{}")]);
    create.create = true;
    let result = run(edit(vec![create]), cx).await;
    assert!(refused(&result), "{:?}", result.err());

    let resolved = cx
        .spawn(async move |mut cx: gpui::AsyncApp| {
            resolve_file(
                "ext",
                FileRef {
                    worktree: None,
                    path: "accounts/work/.credentials.json".into(),
                },
                false,
                &mut cx,
            )
            .await
        })
        .await;
    assert!(
        matches!(&resolved, Err(EditError::NotAllowed(message)) if message.contains("\"Work\"")),
        "{:?}",
        resolved.err()
    );

    // Nothing changed: the whole edit was refused.
    let a = buffer(&project, "a.txt", cx).await;
    assert_eq!(text(&a, cx), "one\ntwo\n");
    assert_eq!(
        fs.load(Path::new(path!("/work/accounts/work/.credentials.json")))
            .await
            .unwrap(),
        "{}\n"
    );
    assert!(
        !fs.is_file(Path::new(path!("/work/accounts/work/new.json")))
            .await
    );
}

#[gpui::test]
async fn test_untrusted_worktree_is_denied(cx: &mut TestAppContext) {
    let (_, project) = setup(false, cx).await;
    let result = run(
        edit(vec![file("a.txt", &[(at(0, 0), at(0, 3), "ONE")])]),
        cx,
    )
    .await;
    assert_eq!(result.err(), Some(EditError::UntrustedWorktree));
    let a = buffer(&project, "a.txt", cx).await;
    assert_eq!(text(&a, cx), "one\ntwo\n");
}

#[gpui::test]
async fn test_versions(cx: &mut TestAppContext) {
    let (_, project) = setup(true, cx).await;
    let a = buffer(&project, "a.txt", cx).await;
    let version = a.read_with(cx, |buffer, _| document_version(buffer));
    a.update(cx, |buffer, cx| buffer.edit([(0..0, "x")], None, cx));
    let mut stale = file("a.txt", &[(at(0, 0), at(0, 1), "y")]);
    stale.expected_version = Some(version);
    let result = run(edit(vec![stale]), cx).await;
    assert!(
        matches!(result, Err(EditError::Conflict(_))),
        "{:?}",
        result.err()
    );

    let mut current = file("a.txt", &[(at(0, 0), at(0, 1), "y")]);
    current.expected_version = Some(a.read_with(cx, |buffer, _| document_version(buffer)));
    run(edit(vec![current]), cx).await.unwrap();
    assert_eq!(text(&a, cx), "yone\ntwo\n");
}

#[gpui::test]
async fn test_ranges_past_the_end(cx: &mut TestAppContext) {
    let (_, project) = setup(true, cx).await;
    let a = buffer(&project, "a.txt", cx).await;
    let mut strict = file("a.txt", &[(at(9, 0), at(9, 0), "!")]);
    strict.expected_version = Some(a.read_with(cx, |buffer, _| document_version(buffer)));
    let result = run(edit(vec![strict]), cx).await;
    assert!(
        matches!(result, Err(EditError::Invalid(_))),
        "{:?}",
        result.err()
    );
    // Without a version the range clamps to the end.
    run(edit(vec![file("a.txt", &[(at(9, 0), at(9, 0), "!")])]), cx)
        .await
        .unwrap();
    assert_eq!(text(&a, cx), "one\ntwo\n!");
}

#[gpui::test]
async fn test_missing_files_and_create(cx: &mut TestAppContext) {
    let (fs, project) = setup(true, cx).await;
    let result = run(
        edit(vec![file("new.txt", &[(at(0, 0), at(0, 0), "hi")])]),
        cx,
    )
    .await;
    assert!(
        matches!(result, Err(EditError::NotFound(_))),
        "{:?}",
        result.err()
    );

    let mut nowhere = file("missing/new.txt", &[]);
    nowhere.create = true;
    let result = run(edit(vec![nowhere]), cx).await;
    assert!(
        matches!(result, Err(EditError::NotFound(_))),
        "{:?}",
        result.err()
    );

    let mut create = file("dir/new.txt", &[(at(0, 0), at(0, 0), "hi")]);
    create.create = true;
    // The applied edit is dropped here: the host keeps the edited buffer alive while dirty.
    run(edit(vec![create]), cx).await.unwrap();
    let created = buffer(&project, "dir/new.txt", cx).await;
    assert_eq!(text(&created, cx), "hi");
    assert!(
        created.read_with(cx, |buffer, _| buffer.is_dirty()),
        "the edit is not saved"
    );

    // A failure after the file was created removes it again.
    let mut create = file("dir/other.txt", &[]);
    create.create = true;
    let mut stale = file("a.txt", &[(at(0, 0), at(0, 1), "y")]);
    stale.expected_version = Some(u64::MAX);
    let result = run(edit(vec![create, stale]), cx).await;
    assert!(
        matches!(result, Err(EditError::Conflict(_))),
        "{:?}",
        result.err()
    );
    assert!(!fs.is_file(Path::new(path!("/work/dir/other.txt"))).await);
}

#[gpui::test]
async fn test_other_targets(cx: &mut TestAppContext) {
    let (_, _project) = setup(true, cx).await;
    let mut document = file("a.txt", &[]);
    document.target = EditTarget::Document(1);
    let result = run(edit(vec![document]), cx).await;
    assert!(
        matches!(result, Err(EditError::NotFound(_))),
        "{:?}",
        result.err()
    );

    let mut unknown = file("a.txt", &[]);
    unknown.target = EditTarget::File(FileRef {
        worktree: Some(12345),
        path: "a.txt".into(),
    });
    let result = run(edit(vec![unknown]), cx).await;
    assert!(
        matches!(result, Err(EditError::NotFound(_))),
        "{:?}",
        result.err()
    );

    let result = run(edit(vec![file("dir", &[])]), cx).await;
    assert!(
        matches!(result, Err(EditError::Invalid(_))),
        "{:?}",
        result.err()
    );
}

#[gpui::test]
async fn test_a_link_out_of_the_worktree_is_refused(cx: &mut TestAppContext) {
    let (fs, project) = setup(true, cx).await;
    fs.insert_tree(path!("/outside"), json!({ "secret.txt": "s\n" }))
        .await;
    fs.create_symlink(
        Path::new(path!("/work/link.txt")),
        PathBuf::from(path!("/outside/secret.txt")),
    )
    .await
    .unwrap();
    cx.run_until_parked();
    let _ = project;
    let result = run(
        edit(vec![file("link.txt", &[(at(0, 0), at(0, 1), "S")])]),
        cx,
    )
    .await;
    assert!(
        matches!(result, Err(EditError::NotAllowed(_))),
        "{:?}",
        result.err()
    );
    assert_eq!(
        fs.load(Path::new(path!("/outside/secret.txt")))
            .await
            .unwrap(),
        "s\n"
    );
}

/// Through the channel: a dropped future applies nothing; a kept one gets the outcome.
#[gpui::test]
async fn test_through_the_service(cx: &mut TestAppContext) {
    let (_, project) = setup(true, cx).await;
    let edits = cx.update(|cx| for_extension("ext", cx));
    let dropped = edits.apply(
        "ext",
        edit(vec![file("a.txt", &[(at(0, 0), at(0, 3), "X")])]),
    );
    drop(dropped);
    cx.run_until_parked();
    let a = buffer(&project, "a.txt", cx).await;
    assert_eq!(
        text(&a, cx),
        "one\ntwo\n",
        "nothing applied for a dropped edit"
    );

    let kept = edits.apply(
        "ext",
        edit(vec![file("a.txt", &[(at(0, 0), at(0, 3), "ONE")])]),
    );
    let outcome = cx.background_spawn(kept);
    cx.run_until_parked();
    assert_eq!(outcome.await, Ok(()));
    assert_eq!(text(&a, cx), "ONE\ntwo\n");
}

/// integration-5 S1: the paths `agent:observe` hides are the ones an extension may not read:
/// `private_files` (here `.env`, Zed's default) whatever the shield mode.
#[gpui::test]
async fn test_private_paths_for_observe(cx: &mut TestAppContext) {
    let (_, _project) = setup(true, cx).await;
    cx.update(|cx| {
        assert!(is_private_path(Path::new(path!("/work/.env")), cx));
        assert!(!is_private_path(Path::new(path!("/work/a.txt")), cx));
        // 5.19 P7: fails closed outside every registered project.
        assert!(is_private_path(Path::new(path!("/elsewhere/x")), cx));
    });
}

/// 5.19 P7: before any project is registered, every path counts as private.
#[gpui::test]
fn test_private_paths_fail_closed_without_projects(cx: &mut TestAppContext) {
    crate::tests::init_test(cx);
    cx.update(|cx| assert!(is_private_path(Path::new(path!("/work/a.txt")), cx)));
}

/// integration-5 S8: a file on disk the worktree has no entry for (an ignored, unscanned
/// directory) is checked itself: a link out of the worktree is refused, also with `create`.
#[gpui::test]
async fn test_a_link_in_an_unscanned_directory_is_refused(cx: &mut TestAppContext) {
    let (fs, _project) = setup(true, cx).await;
    fs.insert_tree(path!("/outside"), json!({ "secret.txt": "s\n" }))
        .await;
    fs.insert_tree(path!("/work/ignored"), json!({})).await;
    fs.insert_file(path!("/work/.gitignore"), b"ignored/\n".to_vec())
        .await;
    fs.create_symlink(
        Path::new(path!("/work/ignored/link.txt")),
        PathBuf::from(path!("/outside/secret.txt")),
    )
    .await
    .unwrap();
    cx.run_until_parked();
    let mut create = file("ignored/link.txt", &[(at(0, 0), at(0, 1), "S")]);
    create.create = true;
    let result = run(edit(vec![create]), cx).await;
    assert!(
        matches!(result, Err(EditError::NotAllowed(_))),
        "{:?}",
        result.err()
    );
    assert_eq!(
        fs.load(Path::new(path!("/outside/secret.txt")))
            .await
            .unwrap(),
        "s\n"
    );
}

/// integration-6 (decision Q8): an extension sees full hunks only for worktree files that are
/// not private. With a real worktree's `private_files` (`.env`), the snapshot and every streamed
/// event hide the private file's path and text, from the observation through to the extension.
#[gpui::test]
async fn test_observe_redacts_private_files_end_to_end(cx: &mut TestAppContext) {
    use agent_review::observe::{
        ObservedEvent, ObservedHunk, ObservedReview, ObservedSource, ObservedTurnFile,
    };
    use futures::StreamExt as _;
    use ui_ext_wasm::observe::AgentEvent;
    let (_, _project) = setup(true, cx).await;
    let review = |id: u64, path: &str| ObservedReview {
        id,
        session: 1,
        turn: 1,
        abs_path: PathBuf::from(path),
        source: ObservedSource::Write,
        rule: None,
        resolution: agent_review::Resolution::Pending { undecided: 1 },
        submitted: false,
        operation: false,
        hunks: vec![ObservedHunk {
            old_start: 0,
            old_count: 1,
            new_start: 0,
            new_count: 1,
            old_text: "SECRET=1\n".into(),
            new_text: "SECRET=2\n".into(),
            decision: agent_review::HunkDecision::Pending,
            comment: None,
        }],
    };
    let env = path!("/work/.env");
    let a = path!("/work/a.txt");
    let (feed, reviews) = cx.update(|cx| {
        crate::agent_observe::TestFeed::new("ext", true, vec![review(1, env), review(2, a)], cx)
    });
    let (snapshot, mut events) = reviews.subscribe().await;
    assert_eq!(snapshot[0].file, None, "the private file's path is hidden");
    assert!(snapshot[0].hunks.is_empty(), "and its text");
    assert_eq!(
        snapshot[1].file.as_ref().map(|file| file.path.as_str()),
        Some("a.txt")
    );
    assert_eq!(
        snapshot[1].hunks.len(),
        1,
        "a worktree file keeps its hunks"
    );

    cx.update(|cx| {
        feed.observe(ObservedEvent::Updated(review(1, env)), cx);
        feed.observe(
            ObservedEvent::TurnEnded(
                1,
                [env, a]
                    .into_iter()
                    .map(|path| ObservedTurnFile {
                        abs_path: PathBuf::from(path),
                        resolution: agent_review::Resolution::AcceptedAll,
                        rule: None,
                    })
                    .collect(),
            ),
            cx,
        );
        feed.observe(
            ObservedEvent::UnreviewedChanges(1, vec![PathBuf::from(env), PathBuf::from(a)]),
            cx,
        );
    });
    let received: Vec<AgentEvent> = events.by_ref().take(4).collect().await;
    assert_eq!(received[0], AgentEvent::ReviewMode(true));
    let AgentEvent::Updated(updated) = &received[1] else {
        panic!("{:?}", received[1]);
    };
    assert_eq!((updated.file.clone(), updated.hunks.len()), (None, 0));
    let AgentEvent::TurnEnded(1, files) = &received[2] else {
        panic!("{:?}", received[2]);
    };
    assert_eq!(files[0].file, None);
    assert_eq!(
        files[1].file.as_ref().map(|file| file.path.as_str()),
        Some("a.txt")
    );
    let AgentEvent::UnreviewedChanges(1, paths) = &received[3] else {
        panic!("{:?}", received[3]);
    };
    assert_eq!(
        paths
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        ["a.txt"]
    );
}
