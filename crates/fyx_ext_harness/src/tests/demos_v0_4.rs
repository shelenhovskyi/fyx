//! The 0.4-only demos' `fyx-ui-0-4` builds, loaded as packages (plan 5.10): the pseudo-terminal
//! demo's shell runs headless on the harness's fake terminals, and the profile demo's
//! `[[fyx_ui.terminal_profiles]]` start fake profile terminals. `fyx_claude_code` (plan 5.15)
//! and `fyx_gemini_cli` (5.16) load with their whole 0.4 manifests, and their profiles ask for
//! an account.

use std::sync::OnceLock;

use super::*;

const PANEL: &str = "pseudo-terminal";

/// The demos that exist only for 0.4, built together: the SDK's 0.4 feature must not reach the
/// 0.3 demos, so they get a target directory of their own.
const DEMOS_V0_4: [&str; 6] = [
    "fyx_demo_pseudo_terminal",
    "fyx_demo_terminal_profiles",
    "fyx_claude_code",
    "fyx_gemini_cli",
    "fyx_demo_lint",
    "fyx_demo_bookmarks",
];

/// `extensions/<demo>` with its 0.4 component, in a folder of the test target directory, as
/// `script/fyx-build-demos --fyx-ui-0-4` lays it out: the manifest, the component, and every
/// folder but `src/` (assets and package files).
fn demo_package(demo: &str) -> PathBuf {
    static BUILT: OnceLock<std::result::Result<PathBuf, String>> = OnceLock::new();
    let wasm_dir = BUILT
        .get_or_init(|| {
            let features = DEMOS_V0_4
                .iter()
                .map(|demo| format!("{demo}/fyx-ui-0-4"))
                .collect::<Vec<_>>()
                .join(",");
            let mut args = Vec::new();
            for demo in DEMOS_V0_4 {
                args.extend(["-p", demo]);
            }
            args.extend(["--features", &features]);
            build_guest(
                &workspace_dir(),
                "fyx-ui-fixtures-v0_4",
                &args,
                "debug/fyx_demo_pseudo_terminal.wasm",
            )
            .map(|wasm| wasm.parent().expect("a profile directory").to_path_buf())
        })
        .clone()
        .unwrap_or_else(|error| panic!("{error}"));
    let wasm = wasm_dir.join(format!("{demo}.wasm"));
    let package = wasm_dir
        .ancestors()
        .nth(2)
        .expect("a target directory")
        .join(format!("{demo}-{}", std::process::id()));
    if package.exists() {
        std::fs::remove_dir_all(&package).expect("clean");
    }
    std::fs::create_dir_all(&package).expect("package dir");
    let source = workspace_dir().join("extensions").join(demo);
    std::fs::copy(
        source.join("extension.toml"),
        package.join("extension.toml"),
    )
    .expect("manifest");
    std::fs::copy(&wasm, package.join("fyx_ui.wasm"))
        .unwrap_or_else(|error| panic!("copying {wasm:?}: {error}"));
    for entry in std::fs::read_dir(&source).expect("demo dir") {
        let entry = entry.expect("entry");
        if entry.file_type().expect("type").is_dir() && entry.file_name() != "src" {
            copy_dir(&entry.path(), &package.join(entry.file_name()));
        }
    }
    package
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy");
        }
    }
}

#[test]
fn the_profiles_demo_starts_profile_terminals() {
    let package = demo_package("fyx_demo_terminal_profiles");
    let mut harness = Harness::load_package(&package).expect("loads");
    let profiles = harness.terminal_profiles();
    // Both are marked "runs any command you type": Python, and git, whose pager runs `!cmd`.
    assert_eq!(
        profiles
            .iter()
            .map(|profile| (profile.id.as_str(), profile.runs_any_code()))
            .collect::<Vec<_>>(),
        [("git-history", true), ("python-welcome", true)]
    );

    // `${package}` is the package folder; the script it names is there.
    let (program, args) = harness.profile_command_line("python-welcome").unwrap();
    let root = std::fs::canonicalize(&package)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    assert_eq!(program, "python");
    assert_eq!(
        args,
        ["-i".to_owned(), format!("{root}/scripts/welcome.py")]
    );
    assert!(Path::new(&args[1]).is_file(), "{args:?}");

    // The panel lists what the manifest runs.
    harness.mount("terminal-profiles").expect("mounts");
    let texts: Vec<String> = harness
        .nodes("terminal-profiles")
        .unwrap()
        .into_iter()
        .filter_map(|node| match node.props {
            model::Props::Text(text) => Some(text.content),
            _ => None,
        })
        .collect();
    for profile in &profiles {
        let (program, args) = harness.profile_command_line(&profile.id).unwrap();
        let shown = format!("{program} {}", args.join(" ")).replace(&root, "${package}");
        assert!(texts.contains(&profile.name), "{texts:?}");
        assert!(texts.contains(&shown), "{shown:?} in {texts:?}");
    }

    // The user starts one; it is theirs, with no account tool to pick an account for.
    let handle = harness.open_profile("git-history", None).unwrap();
    let terminal = harness.terminal(handle).unwrap();
    assert_eq!(terminal.origin, ui_ext::terminal::TerminalOrigin::Profile);
    assert_eq!(terminal.profile.as_deref(), Some("git-history"));
    assert!(harness.open_profile("git-history", Some("Work")).is_err());
    assert!(harness.open_profile("unknown", None).is_err());
    harness.close_profile(handle).unwrap();
    assert!(harness.terminal(handle).is_none());
}

/// The skeleton of plan 5.15: the manifest loads, the plugin ships in the package, and the
/// profile runs `claude --ide` with the account the user picks. The relay and review wiring is
/// replayed below (`claude_code_replays_the_compat_transcripts_on_its_listener`).
#[test]
fn claude_code_loads_with_its_profile_and_plugin() {
    let package = demo_package("fyx_claude_code");
    let mut harness = Harness::load_package(&package).expect("loads");
    for file in [
        "claude-plugin/.claude-plugin/plugin.json",
        "claude-plugin/hooks/hooks.json",
    ] {
        assert!(package.join(file).is_file(), "{file} is not in the package");
    }
    let profiles = harness.terminal_profiles();
    assert_eq!(
        profiles
            .iter()
            .map(|profile| (profile.id.as_str(), profile.account_tool.as_deref()))
            .collect::<Vec<_>>(),
        [("claude", Some("claude"))]
    );
    let (program, args) = harness.profile_command_line("claude").unwrap();
    assert_eq!(
        (program.as_str(), args),
        ("claude", vec!["--ide".to_owned()])
    );
    // The profile's terminal finds the IDE server and loads the plugin shipped in the package.
    let env: HashMap<String, String> = harness.profile_env("claude").unwrap().into_iter().collect();
    assert_eq!(
        env["CLAUDE_CODE_SSE_PORT"],
        harness.listener_port("ide").unwrap().to_string()
    );
    assert_eq!(env["ENABLE_IDE_INTEGRATION"], "true");
    let plugin = PathBuf::from(&env["CLAUDE_CODE_PLUGIN_DIRS"]);
    assert!(
        plugin.join(".claude-plugin/plugin.json").is_file(),
        "{plugin:?} is not the package's plugin"
    );

    let handle = harness.open_profile("claude", Some("Work")).unwrap();
    let terminal = harness.terminal(handle).unwrap();
    assert_eq!(terminal.origin, ui_ext::terminal::TerminalOrigin::Profile);
    assert_eq!(terminal.account.as_deref(), Some("Work"));
    harness.close_profile(handle).unwrap();

    harness.mount("claude-code").expect("mounts");
    let texts: Vec<String> = harness
        .nodes("claude-code")
        .unwrap()
        .into_iter()
        .filter_map(|node| match node.props {
            model::Props::Text(text) => Some(text.content),
            _ => None,
        })
        .collect();
    assert!(
        texts.contains(&"No Claude session is running.".to_owned()),
        "{texts:?}"
    );
}

/// Plan 5.14's listeners: the component registers its IDE tools on `ide` while loading, the
/// fake host speaks MCP for it, and the component answers `tools/call`s.
#[test]
fn claude_code_answers_on_its_ide_listener() {
    let mut harness = Harness::load_package(demo_package("fyx_claude_code")).expect("loads");
    assert!(harness.listener_open("ide"));
    assert!(harness.listener_port("ide").is_some());
    let tools: Vec<String> = harness
        .registered_tools("ide")
        .into_iter()
        .map(|tool| tool.name)
        .collect();
    assert_eq!(
        tools,
        [
            "openFile",
            "openDiff",
            "close_tab",
            "closeAllDiffTabs",
            "getDiagnostics"
        ]
    );

    let connection = harness.connect("ide").unwrap();
    let mut exchange = |message: serde_json::Value| -> Vec<serde_json::Value> {
        harness
            .client_send(connection, &message.to_string())
            .unwrap();
        harness.deliver_io().unwrap();
        harness
            .client_received(connection)
            .unwrap()
            .iter()
            .map(|text| serde_json::from_str(text).unwrap())
            .collect()
    };
    let initialized = exchange(serde_json::json!({
        "jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": { "protocolVersion": "2025-11-25" },
    }));
    assert_eq!(initialized[0]["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        initialized[0]["result"]["serverInfo"]["name"],
        "fyx-claude-code"
    );
    let closed = exchange(serde_json::json!({
        "jsonrpc": "2.0", "id": "a", "method": "tools/call",
        "params": { "name": "closeAllDiffTabs", "arguments": {} },
    }));
    assert_eq!(
        closed,
        [serde_json::json!({
            "jsonrpc": "2.0", "id": "a",
            "result": { "content": [{ "type": "text", "text": "CLOSED_ALL_DIFF_TABS" }] },
        })]
    );
    // No path maps to a worktree until the SDK has `context`.
    let refused = exchange(serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": { "name": "openFile", "arguments": { "filePath": "/work/app/a.rs" } },
    }));
    assert_eq!(refused[0]["error"]["code"], -32602, "{refused:?}");
    let unknown = exchange(serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": { "name": "nope" },
    }));
    assert_eq!(unknown[0]["error"]["code"], -32602, "{unknown:?}");
    assert!(harness.open_mcp_calls().is_empty());

    harness.set_listener_granted("ide", false);
    assert!(!harness.listener_open("ide"));
    assert!(harness.client_send(connection, "{}").is_err());
    assert!(harness.connect("ide").is_err());
}

/// The compat transcripts (`extensions/fyx_claude_code/src/compat`), replayed through the
/// harness against the built component, as far as listeners reach: every recorded message from
/// Claude goes to the fake `ide` listener; the host's `initialize` and `tools/list` answers and
/// the extension's answers to the calls that need no path must equal the recorded ones, and
/// every call is answered. Each hook goes through the fake relay from the profile terminal and
/// is answered with exit 0; a hook or review decision with `expect` must make those turns and
/// leave the last expected status on the terminal's tab.
#[test]
fn claude_code_replays_the_compat_transcripts_on_its_listener() {
    use serde_json::Value;
    use std::collections::VecDeque;
    const TRANSCRIPTS: [(&str, &str); 4] = [
        (
            "turn-with-diff",
            include_str!("../../../../extensions/fyx_claude_code/src/compat/turn-with-diff.jsonl"),
        ),
        (
            "live-interactive",
            include_str!(
                "../../../../extensions/fyx_claude_code/src/compat/live-interactive.jsonl"
            ),
        ),
        (
            "live-interactive-windows",
            include_str!(
                "../../../../extensions/fyx_claude_code/src/compat/live-interactive-windows.jsonl"
            ),
        ),
        (
            "live-print-hooks",
            include_str!(
                "../../../../extensions/fyx_claude_code/src/compat/live-print-hooks.jsonl"
            ),
        ),
    ];
    const COMPARED: [&str; 4] = [
        "openDiff",
        "close_tab",
        "closeAllDiffTabs",
        "getDiagnostics",
    ];
    let package = demo_package("fyx_claude_code");
    let mut compared = 0;
    let mut notified = 0;
    let mut hooks = 0;
    for (name, text) in TRANSCRIPTS {
        let header: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        let root = header["root"].as_str().expect("a root");
        let mut harness = Harness::load_package(&package).expect("loads");
        harness.set_worktrees(&[(1, root)]);
        let connection = harness.connect("ide").unwrap();
        let terminal = harness.open_profile("claude", None).unwrap();
        let mut tools = HashMap::new();
        let mut answers: HashMap<String, Value> = HashMap::new();
        let mut notes: VecDeque<Value> = VecDeque::new();
        // Each `openDiff` call's proposal, by JSON-RPC id.
        let mut proposal_of: HashMap<String, u64> = HashMap::new();
        let receive = |harness: &mut Harness,
                       answers: &mut HashMap<String, Value>,
                       notes: &mut VecDeque<Value>| {
            for text in harness.client_received(connection).unwrap() {
                let answer: Value = serde_json::from_str(&text).unwrap();
                match answer.get("id") {
                    Some(id) => {
                        answers.insert(id.to_string(), answer);
                    }
                    None => notes.push_back(answer),
                }
            }
        };
        for (number, line) in text.lines().enumerate().skip(1) {
            if line.trim().is_empty() {
                continue;
            }
            let at = format!("{name}.jsonl:{}", number + 1);
            let event: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("{at}: {e}"));
            if event["t"] == "editor" {
                let file = ui_ext::process::FileRef {
                    worktree: Some(1),
                    path: event["path"].as_str().unwrap().to_owned(),
                };
                let selection = &event["selection"];
                let position = |at: &Value| ui_ext::edit::Position {
                    line: at[0].as_u64().unwrap() as u32,
                    column: at[1].as_u64().unwrap() as u32,
                };
                let range = ui_ext::edit::TextRange {
                    start: position(&selection["start"]),
                    end: position(&selection["end"]),
                };
                harness.set_editor_context(ui_ext::context::EditorContext {
                    active: Some(file.clone()),
                    open: vec![file],
                    selection: Some((range, selection["text"].as_str().map(str::to_owned))),
                    trusted: Some(true),
                });
                harness.deliver_io().unwrap_or_else(|e| panic!("{at}: {e}"));
                receive(&mut harness, &mut answers, &mut notes);
                continue;
            }
            if event["t"] == "diagnostics" {
                use ui_ext::context::{Diagnostic, DiagnosticSeverity, FileDiagnostics};
                let position = |at: &Value| ui_ext::edit::Position {
                    line: at[0].as_u64().unwrap() as u32,
                    column: at[1].as_u64().unwrap() as u32,
                };
                let files = event["files"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|file| FileDiagnostics {
                        path: ui_ext::process::FileRef {
                            worktree: Some(1),
                            path: file["path"].as_str().unwrap().to_owned(),
                        },
                        diagnostics: file["diagnostics"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|entry| Diagnostic {
                                range: ui_ext::edit::TextRange {
                                    start: position(&entry["range"]["start"]),
                                    end: position(&entry["range"]["end"]),
                                },
                                severity: match entry["severity"].as_str() {
                                    Some("Error") => DiagnosticSeverity::Error,
                                    Some("Warning") => DiagnosticSeverity::Warning,
                                    Some("Information") => DiagnosticSeverity::Information,
                                    Some("Hint") => DiagnosticSeverity::Hint,
                                    other => panic!("{at}: severity {other:?}"),
                                },
                                message: entry["message"].as_str().unwrap().to_owned(),
                                source: entry["source"].as_str().map(str::to_owned),
                                code: None,
                            })
                            .collect(),
                    })
                    .collect();
                harness.set_diagnostics(files);
                harness.deliver_io().unwrap_or_else(|e| panic!("{at}: {e}"));
                receive(&mut harness, &mut answers, &mut notes);
                continue;
            }
            if event["t"] == "hook" {
                let turns = harness.review_turns().len();
                let id = harness
                    .relay_request(
                        event["channel"].as_str().unwrap(),
                        Some(terminal),
                        &[],
                        &event["stdin"].to_string(),
                    )
                    .unwrap_or_else(|e| panic!("{at}: {e}"));
                harness.deliver_io().unwrap_or_else(|e| panic!("{at}: {e}"));
                receive(&mut harness, &mut answers, &mut notes);
                assert_eq!(
                    harness.relay_answer(id),
                    Some(ui_ext::integration::RelayAnswer::default()),
                    "{at}: every hook lets Claude go on"
                );
                if let Some(expect) = event["expect"].as_array() {
                    check_hook_effects(&harness, terminal, turns, expect, &at);
                    hooks += 1;
                }
                continue;
            }
            if event["t"] == "review" {
                use ui_ext::review::Decision;
                let turns = harness.review_turns().len();
                let call = event["call"].to_string();
                let proposal = proposal_of[&call];
                let final_content = event["final_content"].as_str().unwrap_or_default();
                let decided = match event["decision"].as_str() {
                    Some("accepted") => {
                        harness.decide_review(proposal, Decision::Accepted, final_content)
                    }
                    Some("partial") => {
                        harness.decide_review(proposal, Decision::Partial, final_content)
                    }
                    Some("rejected") => harness.decide_review(proposal, Decision::Rejected, ""),
                    Some("closed") => harness.fail_review(proposal, "the file went away"),
                    other => panic!("{at}: decision {other:?}"),
                };
                assert!(decided, "{at}: proposal {proposal} is not open");
                harness.deliver_io().unwrap_or_else(|e| panic!("{at}: {e}"));
                receive(&mut harness, &mut answers, &mut notes);
                if let Some(expect) = event["expect"].as_array() {
                    check_hook_effects(&harness, terminal, turns, expect, &at);
                    hooks += 1;
                }
                continue;
            }
            if event["t"] != "mcp" {
                continue;
            }
            let message = &event["msg"];
            if event["dir"] == "in" {
                if message["method"] == "tools/call" {
                    tools.insert(
                        message["id"].to_string(),
                        message["params"]["name"]
                            .as_str()
                            .unwrap_or_default()
                            .to_owned(),
                    );
                }
                let before: Vec<u64> = harness.proposals().iter().map(|(id, _)| *id).collect();
                harness
                    .client_send(connection, &message.to_string())
                    .unwrap_or_else(|e| panic!("{at}: {e}"));
                harness.deliver_io().unwrap_or_else(|e| panic!("{at}: {e}"));
                receive(&mut harness, &mut answers, &mut notes);
                if message["params"]["name"] == "openDiff" {
                    let (proposal, _) = harness
                        .proposals()
                        .into_iter()
                        .find(|(id, _)| !before.contains(id))
                        .unwrap_or_else(|| panic!("{at}: openDiff proposed nothing"));
                    proposal_of.insert(message["id"].to_string(), proposal);
                }
                continue;
            }
            let Some(id) = message.get("id").map(Value::to_string) else {
                // `at_mentioned` needs a gesture the harness does not have yet.
                if message["method"] == "selection_changed" {
                    let ours = notes
                        .pop_front()
                        .unwrap_or_else(|| panic!("{at}: no selection_changed"));
                    assert_eq!(&ours, message, "{at}");
                    notified += 1;
                }
                continue;
            };
            let tool = tools.get(&id).map(String::as_str);
            let ours = answers
                .get(&id)
                .unwrap_or_else(|| panic!("{at}: no answer to {id}"));
            if message["result"]["serverInfo"].is_object() {
                assert_eq!(
                    ours["result"]["protocolVersion"], message["result"]["protocolVersion"],
                    "{at}"
                );
            } else if let Some(recorded) = message["result"]["tools"].as_array() {
                let names = |tools: &[Value]| -> Vec<String> {
                    tools
                        .iter()
                        .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
                        .collect()
                };
                let listed = ours["result"]["tools"].as_array().expect("tools");
                assert_eq!(names(listed), names(recorded), "{at}");
            } else if tool.is_some_and(|tool| COMPARED.contains(&tool)) {
                assert_eq!(ours, message, "{at}");
                compared += 1;
            }
        }
        for id in tools.keys() {
            assert!(
                answers.contains_key(id),
                "{name}: call {id} was never answered"
            );
        }
        assert!(harness.open_mcp_calls().is_empty(), "{name}");
        assert!(harness.proposals().is_empty(), "{name}");
        assert!(harness.waiting_relay_requests().is_empty(), "{name}");
        assert!(notes.is_empty(), "{name}: {notes:?} not in the transcript");
    }
    assert!(compared > 0, "no answer was compared");
    assert!(notified > 0, "no selection_changed was compared");
    assert!(hooks > 0, "no hook's effects were compared");
}

/// The turns since the `turns`-th and the terminal's status, against a transcript's `expect` in
/// `compat::describe`'s form: every `begin-turn` and `end-turn` in order, and the last `status`.
fn check_hook_effects(
    harness: &Harness,
    terminal: u64,
    turns: usize,
    expect: &[serde_json::Value],
    at: &str,
) {
    use serde_json::{Value, json};
    let made: Vec<Value> = harness.review_turns()[turns..]
        .iter()
        .map(|turn| match turn {
            ReviewTurn::Begin(session) => json!({ "begin-turn": session }),
            ReviewTurn::End(session, changed) => json!({
                "end-turn": {
                    "session": session,
                    "changed": changed.iter().map(|file| file.path.clone()).collect::<Vec<_>>(),
                }
            }),
        })
        .collect();
    let expected: Vec<Value> = expect
        .iter()
        .filter(|effect| effect.get("status").is_none())
        .cloned()
        .collect();
    assert_eq!(made, expected, "{at}: turns");
    if let Some(status) = expect.iter().rev().find_map(|effect| effect.get("status")) {
        let shown = harness
            .terminal(terminal)
            .expect("the profile terminal")
            .status
            .as_ref()
            .map_or(Value::Null, |shown| {
                json!({
                    "state": format!("{:?}", shown.state).to_lowercase(),
                    "label": shown.label,
                })
            });
        assert_eq!(&shown, status, "{at}: status");
    }
}

/// Plan 5.16: the manifest loads, and the profile runs `gemini` with the account the user
/// picks. The companion runs on the fake `ide` listener
/// (`gemini_cli_replays_the_compat_transcripts_on_its_listener`).
#[test]
fn gemini_cli_loads_with_its_profile() {
    let mut harness = Harness::load_package(demo_package("fyx_gemini_cli")).expect("loads");
    let profiles = harness.terminal_profiles();
    assert_eq!(
        profiles
            .iter()
            .map(|profile| (profile.id.as_str(), profile.account_tool.as_deref()))
            .collect::<Vec<_>>(),
        [("gemini", Some("gemini"))]
    );
    let (program, args) = harness.profile_command_line("gemini").unwrap();
    assert_eq!((program.as_str(), args.len()), ("gemini", 0));
    // The profile's terminal finds the IDE server and loads the hooks shipped in the package.
    let env: HashMap<String, String> = harness.profile_env("gemini").unwrap().into_iter().collect();
    assert_eq!(env.len(), 2, "{env:?}");
    assert_eq!(
        env["GEMINI_CLI_IDE_SERVER_PORT"],
        harness.listener_port("ide").unwrap().to_string()
    );
    let defaults = PathBuf::from(&env["GEMINI_CLI_SYSTEM_DEFAULTS_PATH"]);
    let settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&defaults).expect("the package's defaults"))
            .unwrap();
    // Every channel has its hook in the file (src/hooks.rs checks the commands).
    let hooks = settings["hooks"].as_object().expect("hooks");
    let channels: Vec<String> = harness
        .relay_channels()
        .iter()
        .map(|channel| channel.name.clone())
        .collect();
    assert_eq!(
        channels,
        [
            "session-start",
            "before-agent",
            "before-tool",
            "after-tool",
            "notification",
            "after-agent",
            "session-end",
        ]
    );
    for channel in &channels {
        assert!(
            hooks.values().any(|hook| hook
                .to_string()
                .contains(&format!("--channel fyx-gemini-cli/{channel}"))),
            "{channel} has no hook"
        );
    }

    let handle = harness.open_profile("gemini", Some("Personal")).unwrap();
    assert_eq!(
        harness.terminal(handle).unwrap().account.as_deref(),
        Some("Personal")
    );
    // A hook from the profile terminal: a turn begins, the tab shows it, and Gemini goes on.
    let id = harness
        .relay_request(
            "fyx-gemini-cli/before-agent",
            Some(handle),
            &[],
            r#"{"hook_event_name":"BeforeAgent","session_id":"g1","cwd":"/work","prompt":"fix"}"#,
        )
        .unwrap();
    harness.deliver_io().unwrap();
    assert_eq!(
        harness.relay_answer(id),
        Some(ui_ext::integration::RelayAnswer::default())
    );
    assert_eq!(harness.review_turns(), [ReviewTurn::Begin("g1".to_owned())]);
    let status = harness.terminal(handle).unwrap().status.clone();
    assert_eq!(
        status.map(|status| status.label),
        Some("Thinking".to_owned())
    );
    harness.close_profile(handle).unwrap();

    harness.mount("gemini-cli").expect("mounts");
    let texts: Vec<String> = harness
        .nodes("gemini-cli")
        .unwrap()
        .into_iter()
        .filter_map(|node| match node.props {
            model::Props::Text(text) => Some(text.content),
            _ => None,
        })
        .collect();
    assert!(
        texts.contains(&"No edit is waiting for review.".to_owned()),
        "{texts:?}"
    );
}

/// The discovery template passes the host's checks and has the shape the CLI reads: `port` a
/// number (a whole `${port}`), `workspacePath` the joined roots, `ideInfo` an object.
#[test]
fn gemini_cli_discovery_file_has_the_cli_shape() {
    let manifest: toml::Value = toml::from_str(
        &std::fs::read_to_string(demo_package("fyx_gemini_cli").join("extension.toml")).unwrap(),
    )
    .unwrap();
    let table = &manifest["fyx_ui"];
    let listeners: Vec<ui_ext::integration::ListenerSpec> =
        table["listeners"].clone().try_into().unwrap();
    let discovery_files: Vec<ui_ext::integration::DiscoveryFileSpec> =
        table["discovery_files"].clone().try_into().unwrap();
    ui_ext::integration::IntegrationTables {
        listeners: &listeners,
        discovery_files: &discovery_files,
        ..Default::default()
    }
    .check()
    .expect("the host accepts the template");
    use ui_ext::integration::DiscoveryValue as V;
    let [file] = discovery_files.as_slice() else {
        panic!("{discovery_files:?}");
    };
    assert_eq!(file.name, "gemini-ide-server-${pid}-${port}.json");
    let text = |value: &str| V::Text(value.into());
    assert_eq!(file.content["port"], text("${port}"));
    assert_eq!(file.content["workspacePath"], text("${worktrees.joined}"));
    assert_eq!(file.content["authToken"], text("${token}"));
    let V::Table(ide_info) = &file.content["ideInfo"] else {
        panic!("ideInfo is not a table");
    };
    assert_eq!(ide_info["name"], text("fyx"));
    assert_eq!(ide_info["displayName"], text("Fyx"));
}

/// Plan 5.16: `fyx_gemini_cli`'s compat transcripts (Gemini CLI 0.69.0), replayed through the
/// built 0.4 component on the fake `ide` listener. The host answers `initialize` and
/// `tools/list`; the component answers `openDiff` and `closeDiff` through the review fake, and
/// its notifications (`ide/contextUpdate`, `ide/diffAccepted`, `ide/diffRejected`) must equal
/// the recorded ones, in order. A context update's `timestamp`s are ranks in the component
/// (the editor feed has an order, not times), so only their order is compared.
#[test]
fn gemini_cli_replays_the_compat_transcripts_on_its_listener() {
    use serde_json::Value;
    use std::collections::VecDeque;
    const TRANSCRIPTS: [(&str, &str); 1] = [(
        "two-diffs",
        include_str!("../../../../extensions/fyx_gemini_cli/src/compat/two-diffs.jsonl"),
    )];
    fn without_timestamps(mut message: Value) -> Value {
        if let Some(files) = message["params"]["workspaceState"]["openFiles"].as_array_mut() {
            for file in files {
                file.as_object_mut().unwrap().remove("timestamp");
            }
        }
        message
    }
    let package = demo_package("fyx_gemini_cli");
    let (mut compared, mut notified) = (0, 0);
    for (name, text) in TRANSCRIPTS {
        let header: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(header["gemini_cli"], "0.69.0", "{name}");
        let root = header["root"].as_str().expect("a root");
        let mut harness = Harness::load_package(&package).expect("loads");
        assert!(harness.listener_open("ide"));
        harness.set_worktrees(&[(1, root)]);
        // The feed's first context (nothing open) goes out before the CLI connects.
        harness.deliver_io().unwrap();
        // The transcript's `conn` ids, as the harness's connections.
        let mut connections: HashMap<u64, u64> = HashMap::new();
        let mut answers: HashMap<String, Value> = HashMap::new();
        let mut notes: HashMap<u64, VecDeque<Value>> = HashMap::new();
        let mut calls = Vec::new();
        let mut proposal_of: HashMap<String, u64> = HashMap::new();
        let receive = |harness: &mut Harness,
                       connections: &HashMap<u64, u64>,
                       answers: &mut HashMap<String, Value>,
                       notes: &mut HashMap<u64, VecDeque<Value>>| {
            for (&conn, &connection) in connections {
                for text in harness.client_received(connection).unwrap() {
                    let message: Value = serde_json::from_str(&text).unwrap();
                    match message.get("id") {
                        Some(id) => {
                            answers.insert(id.to_string(), message);
                        }
                        None => notes.entry(conn).or_default().push_back(message),
                    }
                }
            }
        };
        for (number, line) in text.lines().enumerate().skip(1) {
            if line.trim().is_empty() {
                continue;
            }
            let at = format!("{name}.jsonl:{}", number + 1);
            let event: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("{at}: {e}"));
            let conn = event["conn"].as_u64().unwrap_or(1);
            connections
                .entry(conn)
                .or_insert_with(|| harness.connect("ide").unwrap());
            match event["t"].as_str() {
                Some("editor") => {
                    let mut files: Vec<(String, u64)> = event["files"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|file| {
                            (
                                file[0].as_str().unwrap().to_owned(),
                                file[1].as_u64().unwrap(),
                            )
                        })
                        .collect();
                    files.sort_by_key(|(_, focused_at)| std::cmp::Reverse(*focused_at));
                    let open: Vec<ui_ext::process::FileRef> = files
                        .into_iter()
                        .map(|(path, _)| ui_ext::process::FileRef {
                            worktree: Some(1),
                            path,
                        })
                        .collect();
                    let selection = event.get("selection").map(|selection| {
                        let caret = ui_ext::edit::Position {
                            line: selection["line"].as_u64().unwrap() as u32,
                            column: selection["column"].as_u64().unwrap() as u32,
                        };
                        (
                            ui_ext::edit::TextRange {
                                start: caret,
                                end: caret,
                            },
                            selection["text"].as_str().map(str::to_owned),
                        )
                    });
                    harness.set_editor_context(ui_ext::context::EditorContext {
                        active: open.first().cloned(),
                        open,
                        selection,
                        trusted: event["trusted"].as_bool(),
                    });
                }
                Some("review") => {
                    use ui_ext::review::Decision;
                    let proposal = proposal_of[&event["call"].to_string()];
                    let final_content = event["final_content"].as_str().unwrap_or_default();
                    let decided = match event["decision"].as_str() {
                        Some("accepted") => {
                            harness.decide_review(proposal, Decision::Accepted, final_content)
                        }
                        Some("partial") => {
                            harness.decide_review(proposal, Decision::Partial, final_content)
                        }
                        Some("rejected") => harness.decide_review(proposal, Decision::Rejected, ""),
                        Some("closed") => harness.fail_review(proposal, "the file went away"),
                        other => panic!("{at}: decision {other:?}"),
                    };
                    assert!(decided, "{at}: proposal {proposal} is not open");
                }
                Some("mcp") if event["dir"] == "in" => {
                    let message = &event["msg"];
                    if message["method"] == "tools/call" {
                        calls.push(message["id"].to_string());
                    }
                    let before: Vec<u64> = harness.proposals().iter().map(|(id, _)| *id).collect();
                    harness
                        .client_send(connections[&conn], &message.to_string())
                        .unwrap_or_else(|e| panic!("{at}: {e}"));
                    harness.deliver_io().unwrap_or_else(|e| panic!("{at}: {e}"));
                    if message["params"]["name"] == "openDiff" {
                        let (proposal, _) = harness
                            .proposals()
                            .into_iter()
                            .find(|(id, _)| !before.contains(id))
                            .unwrap_or_else(|| panic!("{at}: openDiff proposed nothing"));
                        proposal_of.insert(message["id"].to_string(), proposal);
                    }
                }
                Some("mcp") => {
                    let message = &event["msg"];
                    receive(&mut harness, &connections, &mut answers, &mut notes);
                    match message.get("id").map(Value::to_string) {
                        None => {
                            let ours = notes
                                .get_mut(&conn)
                                .and_then(VecDeque::pop_front)
                                .unwrap_or_else(|| panic!("{at}: nothing sent for {message}"));
                            assert_eq!(
                                without_timestamps(ours),
                                without_timestamps(message.clone()),
                                "{at}"
                            );
                            notified += 1;
                        }
                        Some(id) => {
                            let ours = answers
                                .get(&id)
                                .unwrap_or_else(|| panic!("{at}: no answer to {id}"));
                            if message["result"]["serverInfo"].is_object() {
                                assert_eq!(
                                    ours["result"]["protocolVersion"],
                                    message["result"]["protocolVersion"],
                                    "{at}"
                                );
                            } else if let Some(recorded) = message["result"]["tools"].as_array() {
                                let names = |tools: &[Value]| -> Vec<String> {
                                    tools
                                        .iter()
                                        .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
                                        .collect()
                                };
                                let listed = ours["result"]["tools"].as_array().expect("tools");
                                assert_eq!(names(listed), names(recorded), "{at}");
                            } else {
                                assert_eq!(ours, message, "{at}");
                                compared += 1;
                            }
                        }
                    }
                    continue;
                }
                other => panic!("{at}: event {other:?}"),
            }
            harness.deliver_io().unwrap_or_else(|e| panic!("{at}: {e}"));
        }
        receive(&mut harness, &connections, &mut answers, &mut notes);
        for id in &calls {
            assert!(
                answers.contains_key(id),
                "{name}: call {id} was never answered"
            );
        }
        assert!(harness.open_mcp_calls().is_empty(), "{name}");
        assert!(harness.proposals().is_empty(), "{name}");
        assert!(
            notes.values().all(VecDeque::is_empty),
            "{name}: {notes:?} not in the transcript"
        );
    }
    assert!(compared > 0, "no answer was compared");
    assert!(notified > 0, "no notification was compared");
}

#[test]
fn profiles_need_their_capabilities() {
    let wasm = std::fs::read(demo_package("fyx_demo_terminal_profiles").join("fyx_ui.wasm"))
        .expect("component");
    let spec = |args: &[&str]| ui_ext::terminal::TerminalProfileSpec {
        id: "py".into(),
        name: "Py".into(),
        icon: None,
        program: "python".into(),
        path: None,
        args: args.iter().map(ToString::to_string).collect(),
        cwd: ui_ext::terminal::ProfileCwd::Home,
        account_tool: None,
        resume_flag: None,
    };
    let load = |capabilities: &[&str]| {
        Harness::load_with_capabilities(&wasm, capabilities).expect("loads")
    };
    let error = load(&["ui:panel"])
        .with_terminal_profiles(vec![spec(&[])])
        .err()
        .expect("refused");
    assert!(error.to_string().contains("terminal:profiles"), "{error}");
    let error = load(&["ui:panel", "terminal:profiles"])
        .with_terminal_profiles(vec![spec(&["${package}/x.py"])])
        .err()
        .expect("refused");
    assert!(error.to_string().contains("package:files"), "{error}");
    let harness = load(&["ui:panel", "terminal:profiles"])
        .with_terminal_profiles(vec![spec(&["${package}/x.py"]), spec(&[])])
        .err()
        .expect("an id used twice");
    assert!(harness.to_string().contains("used twice"), "{harness}");
    let harness = load(&["ui:panel", "terminal:profiles", "package:files"])
        .with_terminal_profiles(vec![spec(&["-i", "${package}/x.py"])])
        .expect("accepted");
    assert_eq!(
        harness.profile_command_line("py").unwrap().1,
        ["-i", "<package>/x.py"]
    );
}

fn status_line(harness: &Harness) -> String {
    harness
        .nodes(PANEL)
        .expect("mounted")
        .into_iter()
        .find_map(|node| match node.props {
            model::Props::Text(text) if text.content.starts_with("WASM shell") => {
                Some(text.content)
            }
            _ => None,
        })
        .expect("a status line")
}

#[test]
fn the_shell_runs_in_a_panel_terminal() {
    let mut harness =
        Harness::load_package(demo_package("fyx_demo_pseudo_terminal")).expect("loads");
    harness.mount(PANEL).expect("mounts");
    let [handle] = harness.terminal_nodes(PANEL)[..] else {
        panic!("one terminal node: {:?}", harness.terminal_nodes(PANEL));
    };
    let terminal = harness.terminal(handle).expect("open");
    assert!(terminal.in_panel);
    assert!(
        terminal.plain_output().starts_with("Fyx WASM shell"),
        "{:?}",
        terminal.plain_output()
    );

    harness.type_into_terminal(handle, b"echo hi\r").unwrap();
    harness.resize_terminal(handle, 80, 24).unwrap();
    harness.resize_terminal(handle, 100, 30).unwrap();
    harness.deliver_io().expect("delivered");
    let output = harness.terminal(handle).unwrap().plain_output();
    assert!(output.ends_with("echo hi\nhi\nwasm$ "), "{output:?}");
    assert_eq!(status_line(&harness), "WASM shell, 100×30");

    harness.type_into_terminal(handle, b"size\r").unwrap();
    harness.deliver_io().expect("delivered");
    let output = harness.terminal(handle).unwrap().plain_output();
    assert!(output.contains("100 columns, 30 rows"), "{output:?}");

    harness.click(PANEL, "Busy").expect("busy");
    let status = harness.terminal(handle).unwrap().status.clone();
    assert_eq!(
        status.map(|status| (status.state, status.label)),
        Some((ui_ext::terminal::StatusState::Working, "Busy".into()))
    );
    harness.click(PANEL, "Idle").expect("idle");
    assert_eq!(harness.terminal(handle).unwrap().status, None);

    // A remount shows the same terminal; it opens once.
    harness.mount(PANEL).expect("remounts");
    assert_eq!(harness.terminal_nodes(PANEL), [handle]);
    assert_eq!(harness.terminals().len(), 1);
}

#[test]
fn without_terminal_pseudo_the_panel_says_why() {
    let package = demo_package("fyx_demo_pseudo_terminal");
    let wasm = std::fs::read(package.join("fyx_ui.wasm")).expect("component");
    let mut harness = Harness::load_with_capabilities(&wasm, &["ui:panel"]).expect("loads");
    harness.mount(PANEL).expect("mounts");
    assert!(harness.terminal_nodes(PANEL).is_empty());
    assert!(
        status_line_or_error(&harness).contains("\"terminal:pseudo\" is not in the manifest"),
        "{:?}",
        harness.nodes(PANEL)
    );
}

fn status_line_or_error(harness: &Harness) -> String {
    harness
        .nodes(PANEL)
        .expect("mounted")
        .into_iter()
        .find_map(|node| match node.props {
            model::Props::Text(text) if text.content.starts_with("No terminal") => {
                Some(text.content)
            }
            _ => None,
        })
        .unwrap_or_default()
}

/// Plan 6.1 (design.md §D): a 0.4 package's `[[fyx_ui.settings]]` load with it, a user's
/// invalid value gives the extension the default, and a valid one is a change of that key only.
#[test]
fn a_package_declares_settings_and_the_user_sets_them() {
    let package = demo_package("fyx_demo_pseudo_terminal");
    let manifest = package.join("extension.toml");
    let mut text = std::fs::read_to_string(&manifest).unwrap();
    text.push_str(
        r#"
[[fyx_ui.settings]]
key = "scrollback"
title = "Scrollback lines"
type = "integer"
default = 1000
min = 100
max = 10000

[[fyx_ui.settings]]
key = "shell"
title = "Shell"
type = "choice"
default = "bash"
choices = [{ value = "bash", title = "Bash" }, { value = "pwsh", title = "PowerShell" }]
"#,
    );
    std::fs::write(&manifest, &text).unwrap();
    let mut harness = Harness::load_package(&package).expect("loads");
    let keys: Vec<&str> = harness
        .setting_specs()
        .iter()
        .map(|spec| spec.key.as_str())
        .collect();
    assert_eq!(keys, ["scrollback", "shell"]);
    assert_eq!(
        harness.setting("scrollback").unwrap().value,
        settings::SettingValue::Integer(1000)
    );

    let changed = harness
        .set_user_settings(serde_json::json!({ "scrollback": 50, "shell": "pwsh" }))
        .unwrap();
    assert_eq!(changed, ["shell"]);
    assert!(harness.setting("scrollback").unwrap().is_default);
    assert!(
        harness
            .setting_error("scrollback")
            .unwrap()
            .contains("outside")
    );
    assert_eq!(
        harness.setting("shell").unwrap().value,
        settings::SettingValue::Choice("pwsh".into())
    );
    assert_eq!(harness.take_settings_changed(), [vec!["shell".to_owned()]]);
    assert!(harness.setting("missing").is_none());
    // `deliver_io` hands the next change to `handle-io` as `settings-changed`.
    let changed = harness
        .set_user_settings(serde_json::json!({ "scrollback": 500 }))
        .unwrap();
    assert_eq!(changed, ["scrollback", "shell"]);
    assert!(!harness.deliver_io().expect("delivers").is_empty());
    assert!(harness.take_settings_changed().is_empty());
    // The demo read each changed key back through the SDK's `settings::get`.
    let logged: Vec<&str> = harness
        .logs()
        .iter()
        .map(|(_, message)| message.as_str())
        .filter(|message| message.starts_with("setting "))
        .collect();
    assert_eq!(
        logged,
        [
            "setting scrollback = Integer(500)",
            "setting shell = Choice(\"bash\")"
        ]
    );

    // A broken declaration refuses the package.
    std::fs::write(&manifest, text.replace("default = 1000", "default = 5")).unwrap();
    let error = format!(
        "{:#}",
        Harness::load_package(&package).err().expect("refused")
    );
    assert!(error.contains("outside 100..=10000"), "{error}");
}
fn editor_selection(path: &str, line: u32, text: Option<&str>) -> ui_ext::context::EditorContext {
    let file = ui_ext::process::FileRef {
        worktree: Some(1),
        path: path.to_owned(),
    };
    let at = |line| ui_ext::edit::Position { line, column: 0 };
    ui_ext::context::EditorContext {
        active: Some(file.clone()),
        open: vec![file],
        selection: Some((
            ui_ext::edit::TextRange {
                start: at(line),
                end: at(line + 3),
            },
            text.map(str::to_owned),
        )),
        trusted: Some(true),
    }
}

/// Plan 6.6 (design.md §E.4): the lint demo publishes the `TODO`s of a selection as warnings
/// under no source of its own, a private file publishes nothing, and Clear clears every file.
#[test]
fn the_lint_demo_publishes_the_todos_of_a_selection() {
    let mut harness = Harness::load_package(demo_package("fyx_demo_lint")).expect("loads");
    harness.set_worktrees(&[(1, "/work")]);
    harness.set_private_file(1, ".env");
    harness.mount("lint").expect("mounts");
    harness.deliver_io().expect("watching starts");

    harness.set_editor_context(editor_selection(
        "src/main.rs",
        4,
        Some("// TODO: name it\nlet x = 1;\n    TODO\n"),
    ));
    harness.deliver_io().expect("deliver");
    let published = harness.published_diagnostics();
    assert_eq!(published.len(), 1, "{published:?}");
    assert_eq!(published[0].path.path, "src/main.rs");
    let entries: Vec<_> = published[0]
        .diagnostics
        .iter()
        .map(|entry| {
            (
                entry.range.start.line,
                entry.range.start.column,
                entry.severity,
                entry.message.as_str(),
                entry.source.as_deref(),
            )
        })
        .collect();
    use ui_ext::context::DiagnosticSeverity::Warning;
    assert_eq!(
        entries,
        [
            (4, 3, Warning, "TODO: name it", None),
            (6, 4, Warning, "TODO", None)
        ]
    );
    let list = harness
        .find("lint", |props| matches!(props, model::Props::List(_)))
        .expect("a list");
    assert_eq!(harness.rows("lint", list, 0, 10).expect("rows").len(), 2);

    // A private file has no selection text: nothing is published for it.
    harness.set_editor_context(editor_selection(".env", 0, None));
    harness.deliver_io().expect("deliver");
    assert_eq!(harness.published_diagnostics().len(), 1);

    // A selection without a TODO clears its file.
    harness.set_editor_context(editor_selection("src/main.rs", 0, Some("fn main() {}")));
    harness.deliver_io().expect("deliver");
    assert!(harness.published_diagnostics().is_empty());

    harness.set_editor_context(editor_selection("a.rs", 0, Some("TODO")));
    harness.deliver_io().expect("deliver");
    assert_eq!(harness.published_diagnostics().len(), 1);
    harness.click("lint", "Clear").expect("clear");
    assert!(harness.published_diagnostics().is_empty());
}

/// Plan 6.7 (design.md §F.5): the bookmarks demo badges a bookmarked file, shows a two-level
/// tree whose folder rows fold on `row-toggled`, and removes the badge with the bookmark.
#[test]
fn the_bookmarks_demo_is_a_tree_with_badges() {
    let mut harness = Harness::load_package(demo_package("fyx_demo_bookmarks")).expect("loads");
    harness.set_worktrees(&[(1, "/work")]);
    harness.mount("bookmarks").expect("mounts");
    harness.deliver_io().expect("watching starts");
    for path in ["src/main.rs", "src/lib.rs", "README.md"] {
        harness.set_editor_context(editor_selection(path, 0, None));
        harness.deliver_io().expect("deliver");
        let name = path.rsplit('/').next().expect("a name");
        harness
            .click("bookmarks", &format!("Bookmark {name}"))
            .expect("bookmark");
    }
    let badges: Vec<_> = harness
        .file_decorations()
        .into_iter()
        .map(|decoration| (decoration.path.path, decoration.badge))
        .collect();
    assert_eq!(
        badges,
        [
            ("src/main.rs".to_owned(), Some("\u{2605}".to_owned())),
            ("src/lib.rs".to_owned(), Some("\u{2605}".to_owned())),
            ("README.md".to_owned(), Some("\u{2605}".to_owned())),
        ]
    );

    let list = harness
        .find("bookmarks", |props| matches!(props, model::Props::List(_)))
        .expect("a list");
    let shape = |harness: &mut Harness| {
        harness
            .tree_rows("bookmarks", list, 0, 20)
            .expect("rows")
            .into_iter()
            .map(|row| (row.row.indent, row.expansion))
            .collect::<Vec<_>>()
    };
    use crate::Expansion::{Collapsed, Expanded, Leaf};
    assert_eq!(
        shape(&mut harness),
        [
            (0, Expanded),
            (1, Leaf),
            (0, Expanded),
            (1, Leaf),
            (1, Leaf)
        ]
    );

    // Collapsing `src/` (row 2) asks for its rows again.
    let outcome = harness
        .toggle_row("bookmarks", list, 2, false)
        .expect("toggle");
    assert!(
        outcome.invalidated.iter().any(|op| op.target == list),
        "{outcome:?}"
    );
    assert_eq!(
        shape(&mut harness),
        [(0, Expanded), (1, Leaf), (0, Collapsed)]
    );
    harness
        .toggle_row("bookmarks", list, 2, true)
        .expect("toggle");
    assert_eq!(shape(&mut harness).len(), 5);

    // README.md is still the active file: its button removes the bookmark and the badge.
    harness
        .click("bookmarks", "Remove README.md")
        .expect("remove");
    assert_eq!(harness.file_decorations().len(), 2);
    assert_eq!(shape(&mut harness), [(0, Expanded), (1, Leaf), (1, Leaf)]);
}
