//! Writes the generated reference pages into `docs/fyx/api/`, or with `--check` only reports
//! the pages that are out of date and the stale names in `docs/fyx/sdk-reference.md`.

use std::process::ExitCode;

use anyhow::{Context as _, Result};
use fyx_api_docs::{HAND_WRITTEN_REFERENCE, RUST_CRATES, WIT_DIRS};

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<bool> {
    let check_only = std::env::args().any(|argument| argument == "--check");
    let root = fyx_api_docs::repository_root();
    let mut current = true;
    for page in fyx_api_docs::generate(&root)? {
        let path = root.join(&page.path);
        let committed = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        if committed == page.content {
            continue;
        }
        if check_only {
            eprintln!("out of date: {}", page.path.display());
            current = false;
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).with_context(|| format!("creating {parent:?}"))?;
            }
            std::fs::write(&path, &page.content).with_context(|| format!("writing {path:?}"))?;
            eprintln!("wrote {}", page.path.display());
        }
    }

    let mut rust_items = Vec::new();
    for (_, source) in RUST_CRATES {
        rust_items.extend(fyx_api_docs::rust_items(&root.join(source))?);
    }
    let mut wit_names = std::collections::BTreeSet::new();
    for dir in WIT_DIRS {
        wit_names.extend(fyx_api_docs::wit_names(&root.join(dir))?);
    }
    let reference = std::fs::read_to_string(root.join(HAND_WRITTEN_REFERENCE))
        .with_context(|| format!("reading {HAND_WRITTEN_REFERENCE}"))?;
    let stale = fyx_api_docs::stale_references(&reference, &rust_items, &wit_names);
    for name in &stale {
        eprintln!("{HAND_WRITTEN_REFERENCE} names `{name}`, which the SDK and the WIT do not have");
    }
    Ok(current && stale.is_empty())
}
