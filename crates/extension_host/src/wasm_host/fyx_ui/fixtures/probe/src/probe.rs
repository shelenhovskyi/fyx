//! `fyx:ui` guest for the host behaviour the demos do not exercise. On surface `probe`:
//! `click(1)` logs, pushes `patches [remove(1)]` through the host import and answers `unchanged`;
//! `click(2)` spins forever, to hit the host's time budget. It also carries the
//! `fyx:ui-api-version` section explicitly.

use ui_ext_sdk::*;

#[used]
#[unsafe(link_section = "fyx:ui-api-version")]
pub static FYX_UI_API_VERSION: [u8; 6] = [0, 0, 0, 1, 0, 0];

const SURFACE_ID: &str = "probe";
const PUSH: NodeId = 1;
const SPIN: NodeId = 2;

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
            _ => Ok(Update::Unchanged),
        }
    }
}

register_extension!(Probe);
