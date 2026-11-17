//! `fyx:ui` guest for the host behaviour the demos do not exercise. On surface `probe`:
//! `click(1)` logs, pushes `patches [remove(1)]` through the host import and answers `unchanged`;
//! `click(2)` spins forever and `click(3)` sleeps for a second (blocked in `wasi:io/poll`), both to
//! hit the host's time budget. The `fyx:ui-api-version` section comes
//! from `register_extension!`; emitting it here too would concatenate two copies into 12 bytes.

use ui_ext_sdk::*;

const SURFACE_ID: &str = "probe";
const PUSH: NodeId = 1;
const SPIN: NodeId = 2;
const SLEEP: NodeId = 3;

struct Probe;

impl Extension for Probe {
    fn new() -> Self {
        Probe
    }

    fn surfaces(&self) -> Vec<Surface> {
        Vec::new()
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        Err(format!("unknown surface {surface_id}"))
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
            UiEvent::Click(SLEEP) => {
                std::thread::sleep(std::time::Duration::from_secs(1));
                Ok(Update::Unchanged)
            }
            _ => Ok(Update::Unchanged),
        }
    }
}

register_extension!(Probe);
