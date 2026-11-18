//! `fyx:ui` guest for the host behaviour the demos do not exercise. On surface `probe`:
//! `click(1)` logs, pushes `patches [remove(1)]` through the host import and answers `unchanged`;
//! `click(4)` pushes an insert of flex node 10 and answers with an insert of text node 11 under it,
//! so the reply depends on the push; `click(5)` pushes `remove(1)` and then fails; `click(6)`
//! pushes without end, past the host's per-call cap;
//! `click(7)` allocates 300 MiB, past the host's memory cap; `click(8)` pushes 40 updates of 200
//! patches each, past the host's cap on one call's pushed patches;
//! `click(9)` tries to open a TCP connection and fails with the error it got;
//! `click(10)` pushes five 4 MiB markdown texts, past the host's cap on one call's text, and
//! `click(11)` logs one 1 MiB message, under it;
//! `mount("slow")` sleeps for a second, inside the first-mount budget but not a later one's;
//! `click(2)` spins forever and `click(3)` sleeps for a second (blocked in `wasi:io/poll`), both to
//! hit the host's time budget. The `fyx:ui-api-version` section comes
//! from `register_extension!`; emitting it here too would concatenate two copies into 12 bytes.

use ui_ext_sdk::*;

const SURFACE_ID: &str = "probe";
const PUSH: NodeId = 1;
const SPIN: NodeId = 2;
const SLEEP: NodeId = 3;
const PUSH_THEN_PATCH: NodeId = 4;
const PUSH_THEN_FAIL: NodeId = 5;
const PUSH_FLOOD: NodeId = 6;
const ALLOCATE: NodeId = 7;
const PUSH_BULK: NodeId = 8;
const CONNECT: NodeId = 9;
const PUSH_TEXT: NodeId = 10;
const LOG_LONG: NodeId = 11;

struct Probe;

impl Extension for Probe {
    fn new() -> Self {
        Probe
    }

    fn surfaces(&self) -> Vec<Surface> {
        Vec::new()
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        if surface_id != "slow" {
            return Err(format!("unknown surface {surface_id}"));
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
        Ok(vec![Node {
            id: 1,
            parent: None,
            props: TextProps::new("slow").into(),
        }])
    }

    fn handle_event(&mut self, surface_id: &str, event: UiEvent) -> Result<Update, String> {
        if surface_id != SURFACE_ID {
            return Err(format!("unknown surface {surface_id}"));
        }
        match event {
            UiEvent::Click(PUSH) => {
                log(LogLevel::Info, "pushing an update");
                push_update(SURFACE_ID, &Update::Patches(vec![Patch::Remove(PUSH)]));
                Ok(Update::Unchanged)
            }
            UiEvent::Click(SPIN) => loop {
                std::hint::spin_loop();
            },
            UiEvent::Click(PUSH_THEN_PATCH) => {
                let parent = Node {
                    id: 10,
                    parent: None,
                    props: FlexProps::column().into(),
                };
                push_update(
                    SURFACE_ID,
                    &Update::Patches(vec![Patch::Insert(InsertOp {
                        node: parent,
                        before: None,
                    })]),
                );
                Ok(Update::Patches(vec![Patch::Insert(InsertOp {
                    node: Node {
                        id: 11,
                        parent: Some(10),
                        props: TextProps::new("child").into(),
                    },
                    before: None,
                })]))
            }
            UiEvent::Click(PUSH_THEN_FAIL) => {
                push_update(SURFACE_ID, &Update::Patches(vec![Patch::Remove(PUSH)]));
                Err("failed after pushing".into())
            }
            UiEvent::Click(PUSH_FLOOD) => loop {
                push_update(SURFACE_ID, &Update::Unchanged);
            },
            UiEvent::Click(ALLOCATE) => {
                let big = vec![1u8; 300 << 20];
                std::hint::black_box(&big);
                Ok(Update::Unchanged)
            }
            UiEvent::Click(PUSH_BULK) => {
                for _ in 0..40 {
                    let patches = (0..200).map(Patch::Remove).collect();
                    push_update(SURFACE_ID, &Update::Patches(patches));
                }
                Ok(Update::Unchanged)
            }
            UiEvent::Click(CONNECT) => match std::net::TcpStream::connect("127.0.0.1:9") {
                Ok(_) => Err("connected: the sandbox let a socket through".into()),
                Err(error) => Err(format!("connect failed: {error}")),
            },
            UiEvent::Click(PUSH_TEXT) => {
                let text = "x".repeat(4 << 20);
                for _ in 0..5 {
                    push_update(
                        SURFACE_ID,
                        &Update::Patches(vec![Patch::SetProps((1, Props::Markdown(text.clone())))]),
                    );
                }
                Ok(Update::Unchanged)
            }
            UiEvent::Click(LOG_LONG) => {
                log(LogLevel::Debug, &"y".repeat(1 << 20));
                Ok(Update::Unchanged)
            }
            UiEvent::Click(SLEEP) => {
                std::thread::sleep(std::time::Duration::from_secs(1));
                Ok(Update::Unchanged)
            }
            _ => Ok(Update::Unchanged),
        }
    }
}

// The host runs fyx:ui 0.1; the SDK's default world is 0.2.
register_extension!(Probe, v0_1);
