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
const DEMOS_V0_4: [&str; 4] = [
    "fyx_demo_pseudo_terminal",
    "fyx_demo_terminal_profiles",
    "fyx_claude_code",
    "fyx_gemini_cli",
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
/// profile runs `claude --ide` with the account the user picks. Its relay and review wiring is
/// tested natively (`extensions/fyx_claude_code`) until the harness has them (5.14).
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
/// every call is answered. Hooks, review decisions, editor events and the calls with paths wait
/// for the SDK's relay, review and context (the rest of 5.14).
#[test]
fn claude_code_replays_the_compat_transcripts_on_its_listener() {
    use serde_json::Value;
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
    const PATHLESS: [&str; 2] = ["close_tab", "closeAllDiffTabs"];
    let package = demo_package("fyx_claude_code");
    let mut compared = 0;
    for (name, text) in TRANSCRIPTS {
        let mut harness = Harness::load_package(&package).expect("loads");
        let connection = harness.connect("ide").unwrap();
        let mut tools = HashMap::new();
        let mut answers: HashMap<String, Value> = HashMap::new();
        for (number, line) in text.lines().enumerate().skip(1) {
            if line.trim().is_empty() {
                continue;
            }
            let at = format!("{name}.jsonl:{}", number + 1);
            let event: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("{at}: {e}"));
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
                harness
                    .client_send(connection, &message.to_string())
                    .unwrap_or_else(|e| panic!("{at}: {e}"));
                harness.deliver_io().unwrap_or_else(|e| panic!("{at}: {e}"));
                for text in harness.client_received(connection).unwrap() {
                    let answer: Value = serde_json::from_str(&text).unwrap();
                    if let Some(id) = answer.get("id") {
                        answers.insert(id.to_string(), answer);
                    }
                }
                continue;
            }
            // `selection_changed` and `at_mentioned` come from `context`.
            let Some(id) = message.get("id").map(Value::to_string) else {
                continue;
            };
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
            } else if tools
                .get(&id)
                .is_some_and(|tool| PATHLESS.contains(&tool.as_str()))
            {
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
    }
    assert!(compared > 0, "no answer was compared");
}

/// The skeleton of plan 5.16: the manifest loads, and the profile runs `gemini` with the
/// account the user picks. The companion protocol is tested natively until 5.14.
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
    let handle = harness.open_profile("gemini", Some("Personal")).unwrap();
    assert_eq!(
        harness.terminal(handle).unwrap().account.as_deref(),
        Some("Personal")
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
