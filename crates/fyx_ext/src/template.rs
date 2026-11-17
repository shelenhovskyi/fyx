//! `fyx-ext new`: a standalone Rust extension crate with its own `[workspace]`, an
//! `extension.toml` with a `[fyx_ui]` table, a minimal panel with a unit test, the pinned
//! toolchain, `.gitignore` and a README. Ported from `script/fyx-new-extension`.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

/// The Fyx checkout this `fyx-ext` was built from, the default home of the SDK.
pub fn default_fyx_checkout() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const RUST_TOOLCHAIN: &str = include_str!("../../../rust-toolchain.toml");

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SdkSource {
    /// A path dependency on `<checkout>/crates/ui_ext_sdk`.
    Checkout(PathBuf),
    Git {
        url: String,
        reference: GitReference,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitReference {
    Branch(String),
    Rev(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewExtension {
    /// `my_panel` or `my-panel`.
    pub name: String,
    /// The crate goes to `<parent>/<crate name>`.
    pub parent: PathBuf,
    pub sdk: SdkSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Names {
    pub crate_name: String,
    pub id: String,
    pub title: String,
}

pub fn names(name: &str) -> Result<Names> {
    anyhow::ensure!(
        is_valid_name(name),
        "name must be lower-case letters and digits separated by single '-' or '_', starting with a letter: {name}"
    );
    let crate_name = name.replace('-', "_");
    let id = name.replace('_', "-");
    let spaced = id.replace('-', " ");
    let mut chars = spaced.chars();
    let title = chars
        .next()
        .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default();
    Ok(Names {
        crate_name,
        id,
        title,
    })
}

fn is_valid_name(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        && name.split(['-', '_']).all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

fn toolchain_channel() -> Result<&'static str> {
    RUST_TOOLCHAIN
        .lines()
        .find_map(|line| {
            let value = line
                .trim()
                .strip_prefix("channel")?
                .trim()
                .strip_prefix('=')?;
            Some(value.trim().trim_matches('"'))
        })
        .context("no channel in Fyx's rust-toolchain.toml")
}

/// A path as Cargo on Windows accepts it in TOML: forward slashes, no `\\?\` prefix.
fn toml_path(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    text.strip_prefix("//?/").unwrap_or(&text).to_string()
}

/// Writes the crate and returns its directory.
pub fn create(new: &NewExtension) -> Result<PathBuf> {
    let names = names(&new.name)?;
    let crate_dir = new.parent.join(&names.crate_name);
    if crate_dir.exists()
        && std::fs::read_dir(&crate_dir)
            .with_context(|| format!("reading {crate_dir:?}"))?
            .next()
            .is_some()
    {
        anyhow::bail!("{} already exists and is not empty", crate_dir.display());
    }

    let sdk_note = match &new.sdk {
        SdkSource::Checkout(_) => {
            "# The SDK is not on crates.io yet, and it inherits settings from the Fyx workspace, so this\n# path must point into a full Fyx checkout."
        }
        SdkSource::Git { .. } => {
            "# The SDK is not on crates.io yet. Cargo clones the Fyx repository to build it."
        }
    };
    let sdk_dependency = dependency("ui_ext_sdk", &new.sdk)?;
    let harness_dependency = dependency("fyx_ext_harness", &new.sdk)?;

    let fill = |template: &str| {
        template
            .replace("__CRATE__", &names.crate_name)
            .replace("__ID__", &names.id)
            .replace("__TITLE__", &names.title)
            .replace("__SDK_NOTE__", sdk_note)
            .replace("__SDK_DEPENDENCY__", &sdk_dependency)
            .replace("__HARNESS_DEPENDENCY__", &harness_dependency)
    };
    let toolchain = RUST_TOOLCHAIN_TEMPLATE.replace("__TOOLCHAIN__", toolchain_channel()?);

    for dir in ["src", "tests"] {
        std::fs::create_dir_all(crate_dir.join(dir))
            .with_context(|| format!("creating {crate_dir:?}"))?;
    }
    for (path, content) in [
        ("Cargo.toml".to_string(), fill(CARGO_TOML)),
        ("extension.toml".to_string(), fill(EXTENSION_TOML)),
        ("rust-toolchain.toml".to_string(), toolchain),
        (".gitignore".to_string(), GITIGNORE.to_string()),
        (format!("src/{}.rs", names.crate_name), fill(SOURCE)),
        ("tests/headless.rs".to_string(), fill(HEADLESS_TEST)),
        ("README.md".to_string(), fill(README)),
    ] {
        let path = crate_dir.join(path);
        std::fs::write(&path, content).with_context(|| format!("writing {path:?}"))?;
    }
    Ok(crate_dir)
}

/// `<crate> = { ... }` for a crate of the Fyx repository, from the same source as the SDK.
fn dependency(crate_name: &str, source: &SdkSource) -> Result<String> {
    Ok(match source {
        SdkSource::Checkout(checkout) => {
            let path = checkout.join("crates").join(crate_name);
            anyhow::ensure!(
                path.join("Cargo.toml").is_file(),
                "{} is not a Fyx checkout (no crates/{crate_name}/Cargo.toml)",
                checkout.display()
            );
            let path = path
                .canonicalize()
                .with_context(|| format!("resolving {path:?}"))?;
            format!("{crate_name} = {{ path = \"{}\" }}", toml_path(&path))
        }
        SdkSource::Git { url, reference } => {
            let reference = match reference {
                GitReference::Branch(branch) => format!("branch = \"{branch}\""),
                GitReference::Rev(rev) => format!("rev = \"{rev}\""),
            };
            format!("{crate_name} = {{ git = \"{url}\", {reference} }}")
        }
    })
}

const CARGO_TOML: &str = r#"[package]
name = "__CRATE__"
version = "0.1.0"
edition = "2024"
publish = false

# A workspace of its own, so the crate also builds when it sits inside another workspace (such as
# a Fyx checkout) without being one of its members.
[workspace]

[lib]
path = "src/__CRATE__.rs"
crate-type = ["cdylib"]
doctest = false

[dependencies]
__SDK_NOTE__
__SDK_DEPENDENCY__

[dev-dependencies]
# Runs the built component in wasmtime for tests/headless.rs; `fyx-ext test` builds it first.
__HARNESS_DEPENDENCY__

# The harness compiles components with wasmtime, which is much faster optimized.
[profile.dev.package]
wasmtime = { opt-level = 3 }
wasmtime-environ = { opt-level = 3 }
wasmtime-internal-cranelift = { opt-level = 3 }
cranelift-codegen = { opt-level = 3 }
"#;

const EXTENSION_TOML: &str = r#"id = "__ID__"
name = "__TITLE__"
description = "A Fyx UI panel."
version = "0.1.0"
schema_version = 1
authors = []

# Read by Fyx's extension host; upstream Zed ignores this table.
[fyx_ui]
api_version = "0.1.0"
component = "fyx_ui.wasm"
capabilities = ["ui:panel"]
"#;

const RUST_TOOLCHAIN_TEMPLATE: &str = r#"# The toolchain the Fyx SDK is built and tested with.
[toolchain]
channel = "__TOOLCHAIN__"
profile = "minimal"
components = ["rustfmt", "clippy"]
targets = ["wasm32-wasip2"]
"#;

const HEADLESS_TEST: &str = r#"//! Runs the built component the way Fyx does, without the editor. `fyx-ext test` builds it and
//! sets `FYX_EXT_COMPONENT`; a plain `cargo test` skips these tests.

use fyx_ext_harness::Harness;

fn harness() -> Option<Harness> {
    match Harness::from_env() {
        Ok(harness) => Some(harness),
        Err(error) if std::env::var_os(fyx_ext_harness::COMPONENT_VARIABLE).is_none() => {
            eprintln!("skipped, run `fyx-ext test`: {error}");
            None
        }
        Err(error) => panic!("{error:#}"),
    }
}

#[test]
fn clicking_updates_the_label() {
    let Some(mut harness) = harness() else {
        return;
    };
    harness.mount("__ID__").expect("mount");
    assert!(harness.find_text("__ID__", "Clicked 0 times").is_some());

    harness.click("__ID__", "Click me").expect("click");
    assert!(harness.find_text("__ID__", "Clicked 1 times").is_some());
}
"#;

const GITIGNORE: &str = "/target\n/fyx_ui.wasm\n/*.fyxext\n";

const SOURCE: &str = r#"use ui_ext_sdk::*;

const SURFACE_ID: &str = "__ID__";

#[derive(Default)]
struct Panel {
    clicks: u32,
    tree: SurfaceTree,
    label: NodeId,
    button: NodeId,
}

impl Panel {
    fn label(&self) -> TextProps {
        TextProps::new(format!("Clicked {} times", self.clicks)).color(ColorToken::TextMuted)
    }
}

fn check_surface(surface_id: &str) -> Result<(), String> {
    if surface_id == SURFACE_ID {
        Ok(())
    } else {
        Err(format!("unknown surface {surface_id:?}"))
    }
}

impl Extension for Panel {
    fn new() -> Self {
        Self::default()
    }

    fn surfaces(&self) -> Vec<Surface> {
        vec![Surface::panel(
            SURFACE_ID,
            "__TITLE__",
            DockPosition::Right,
            "sparkle",
            280,
        )]
    }

    fn mount(&mut self, surface_id: &str) -> Result<Vec<Node>, String> {
        check_surface(surface_id)?;
        let label = self.label();
        let mut ids = (0, 0);
        let nodes = self.tree.mount(|tree| {
            tree.flex(FlexProps::column().gap(8).padding(8), |tree| {
                tree.text(TextProps::new("__TITLE__").bold());
                ids.0 = tree.text(label);
                ids.1 = tree.button(ButtonProps::new("Click me").icon("plus"));
            });
        })?;
        // A remount builds new nodes with new ids, so they are stored on every mount.
        (self.label, self.button) = ids;
        Ok(nodes)
    }

    fn handle_event(&mut self, surface_id: &str, event: UiEvent) -> Result<Update, String> {
        check_surface(surface_id)?;
        match event {
            UiEvent::Click(id) if id == self.button => {
                // A panic traps the call, and a trapped call disables the extension.
                self.clicks = self.clicks.saturating_add(1);
                self.tree.set_props(self.label, self.label())?;
                Ok(self.tree.take_update())
            }
            _ => Ok(Update::Unchanged),
        }
    }
}

register_extension!(Panel);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn click_patches_the_label() {
        let mut panel = Panel::new();
        let nodes = panel.mount(SURFACE_ID).expect("mount");
        assert_eq!(nodes.len(), 4);

        let update = panel
            .handle_event(SURFACE_ID, UiEvent::Click(panel.button))
            .expect("click");
        let label = Props::Text(TextProps::new("Clicked 1 times").color(ColorToken::TextMuted));
        assert_eq!(
            update,
            Update::Patches(vec![Patch::SetProps((panel.label, label))])
        );

        let update = panel
            .handle_event(SURFACE_ID, UiEvent::Click(panel.label))
            .expect("click on text");
        assert_eq!(update, Update::Unchanged);
    }
}
"#;

const README: &str = r#"# __TITLE__ (`__ID__`)

A Fyx UI extension with one right-dock panel, created by `fyx-ext new`. The guide is
`docs/fyx/extension-authoring.md` in the Fyx repository.

`rust-toolchain.toml` pins the toolchain the SDK is tested with and adds the `wasm32-wasip2`
target, so rustup installs both on the first build.

## Run it in Fyx

```bash
fyx-ext dev --fyx /path/to/fyx
```

This builds the component, checks it, starts Fyx with the extension loaded from this folder, and
rebuilds when a source file changes. Open the right dock and click the sparkle button.

## Other commands

| Command | What it does |
|---|---|
| `fyx-ext build [--release]` | builds for `wasm32-wasip2` and copies the component to `fyx_ui.wasm` |
| `fyx-ext check` | checks `extension.toml` and the component the way Fyx does when it loads them |
| `fyx-ext test` | builds, mounts every surface headless, then runs `cargo test` with the component, so `tests/headless.rs` runs too |
| `fyx-ext package` | writes `__ID__-<version>.fyxext` after a release build and a check |
| `cargo test` | runs the unit tests on your machine, not in WASM; the headless tests skip |

If the crate sits inside a Fyx checkout, that checkout's `.cargo/config.toml` asks for `sccache`.
Without sccache installed, build with `RUSTC_WRAPPER=` set to empty.
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Report;

    #[test]
    fn names_follow_the_crate_and_id_conventions() {
        let names = names("my_panel-2").expect("valid");
        assert_eq!(names.crate_name, "my_panel_2");
        assert_eq!(names.id, "my-panel-2");
        assert_eq!(names.title, "My panel 2");
        for bad in ["", "My", "2x", "a__b", "a-", "a.b"] {
            assert!(super::names(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn creates_a_crate_whose_manifest_passes() {
        let dir = tempfile::tempdir().expect("temp dir");
        let crate_dir = create(&NewExtension {
            name: "hello-panel".into(),
            parent: dir.path().to_path_buf(),
            sdk: SdkSource::Checkout(default_fyx_checkout()),
        })
        .expect("create");
        assert_eq!(crate_dir, dir.path().join("hello_panel"));
        for file in [
            "Cargo.toml",
            "extension.toml",
            "rust-toolchain.toml",
            ".gitignore",
            "src/hello_panel.rs",
            "tests/headless.rs",
            "README.md",
        ] {
            assert!(crate_dir.join(file).is_file(), "{file}");
        }

        let mut report = Report::default();
        let manifest = crate::manifest::read(&crate_dir, &mut report).expect("manifest");
        assert_eq!(manifest.id, "hello-panel");
        assert!(report.is_empty(), "{report:?}");

        let cargo_toml = std::fs::read_to_string(crate_dir.join("Cargo.toml")).expect("read");
        assert!(cargo_toml.contains("name = \"hello_panel\""));
        assert!(cargo_toml.contains("crates/ui_ext_sdk\" }"), "{cargo_toml}");
        assert!(
            cargo_toml.contains("crates/fyx_ext_harness\" }"),
            "{cargo_toml}"
        );
        assert!(!cargo_toml.contains('\\'), "{cargo_toml}");
        let source = std::fs::read_to_string(crate_dir.join("src/hello_panel.rs")).expect("read");
        assert!(source.contains("const SURFACE_ID: &str = \"hello-panel\";"));
        assert!(!source.contains("__"), "every placeholder is filled");
        let toolchain =
            std::fs::read_to_string(crate_dir.join("rust-toolchain.toml")).expect("read");
        assert!(!toolchain.contains("__TOOLCHAIN__"));

        let again = create(&NewExtension {
            name: "hello_panel".into(),
            parent: dir.path().to_path_buf(),
            sdk: SdkSource::Checkout(default_fyx_checkout()),
        });
        assert!(again.expect_err("exists").to_string().contains("not empty"));
    }

    #[test]
    fn git_sdk_dependency() {
        let dir = tempfile::tempdir().expect("temp dir");
        let crate_dir = create(&NewExtension {
            name: "g".into(),
            parent: dir.path().to_path_buf(),
            sdk: SdkSource::Git {
                url: "https://example.com/fyx.git".into(),
                reference: GitReference::Rev("abc123".into()),
            },
        })
        .expect("create");
        let cargo_toml = std::fs::read_to_string(crate_dir.join("Cargo.toml")).expect("read");
        assert!(
            cargo_toml.contains(
                "ui_ext_sdk = { git = \"https://example.com/fyx.git\", rev = \"abc123\" }"
            ),
            "{cargo_toml}"
        );
        assert!(
            cargo_toml.contains(
                "fyx_ext_harness = { git = \"https://example.com/fyx.git\", rev = \"abc123\" }"
            ),
            "{cargo_toml}"
        );
    }
}
