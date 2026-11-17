//! Interim consent for installed UI extensions (integration-4 finding 3, until the permission
//! broker of plan 1.5).
//!
//! An extension in the installed-extensions directory is loaded only when the user has enabled
//! it in its current form: the same capabilities and, for a regular install, the same component
//! (SHA-256), or, for a dev install (a link), the same folder the link points to. A dev
//! extension's component changes on every build, so its folder is what the user agreed to.
//! Anything else (a folder dropped in by hand or by Zed's "Install Dev Extension", an update that
//! changes the component, an `extension.toml` edited to ask for more) waits for an "Enable" click.
//! `FYX_UI_DEV_EXTENSIONS` entries are not checked: setting the variable is the opt-in.
//!
//! Records live in `<data dir>/fyx-ui-consent.json`.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result};
use fs::Fs;
use gpui::BackgroundExecutor;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// What the user agreed to for one extension.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentRecord {
    /// Sorted and deduplicated.
    pub capabilities: Vec<String>,
    pub grant: Grant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Grant {
    /// A regular install: this exact component.
    Component { sha256: String },
    /// A dev install: whatever is built in this folder.
    DevFolder { path: PathBuf },
}

impl ConsentRecord {
    /// The record for the extension in `extension_dir` as it is now. The component (up to
    /// 32 MiB) is hashed on `executor`, off the thread that awaits this.
    pub async fn current(
        extension_dir: &Path,
        capabilities: &[String],
        component: &Path,
        is_dev: bool,
        fs: &dyn Fs,
        executor: &BackgroundExecutor,
    ) -> Result<Self> {
        let mut capabilities = capabilities.to_vec();
        capabilities.sort();
        capabilities.dedup();
        let grant = if is_dev {
            Grant::DevFolder {
                path: fs
                    .canonicalize(extension_dir)
                    .await
                    .with_context(|| format!("resolving {}", extension_dir.display()))?,
            }
        } else {
            let bytes = fs
                .load_bytes(component)
                .await
                .with_context(|| format!("reading {}", component.display()))?;
            Grant::Component {
                sha256: executor
                    .spawn(async move { format!("{:x}", Sha256::digest(&bytes)) })
                    .await,
            }
        };
        Ok(Self {
            capabilities,
            grant,
        })
    }
}

#[derive(Default, Serialize, Deserialize)]
struct ConsentFile {
    extensions: BTreeMap<String, ConsentRecord>,
}

/// The consent records, kept in memory and, when it has a path, on disk.
#[derive(Default)]
pub struct ConsentStore {
    path: Option<PathBuf>,
    records: BTreeMap<String, ConsentRecord>,
    loaded: bool,
}

impl ConsentStore {
    pub fn at(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            records: BTreeMap::new(),
            loaded: false,
        }
    }

    pub fn default_path() -> PathBuf {
        paths::data_dir().join("fyx-ui-consent.json")
    }

    /// The file still to read, if any: read it with [`read_records`] and pass the result to
    /// [`ConsentStore::set_loaded`].
    pub fn path_to_load(&self) -> Option<PathBuf> {
        if self.loaded {
            return None;
        }
        self.path.clone()
    }

    pub fn set_loaded(&mut self, records: BTreeMap<String, ConsentRecord>) {
        if !self.loaded {
            self.loaded = true;
            // Grants made before the file was read stay.
            for (id, record) in records {
                self.records.entry(id).or_insert(record);
            }
        }
    }

    pub fn allows(&self, extension_id: &str, record: &ConsentRecord) -> bool {
        self.records.get(extension_id) == Some(record)
    }

    /// Records consent and returns the file contents to write, if the store has a file.
    pub fn grant(
        &mut self,
        extension_id: &str,
        record: ConsentRecord,
    ) -> Option<(PathBuf, String)> {
        self.records.insert(extension_id.to_string(), record);
        let path = self.path.clone()?;
        let file = ConsentFile {
            extensions: self.records.clone(),
        };
        match serde_json::to_string_pretty(&file) {
            Ok(text) => Some((path, text)),
            Err(error) => {
                log::error!("fyx:ui: cannot save consent: {error}");
                None
            }
        }
    }
}

/// The records in the consent file. A missing file means no consent yet; a broken one is logged
/// and treated the same, so the user is asked again rather than an extension let through.
pub async fn read_records(path: &Path, fs: &dyn Fs) -> BTreeMap<String, ConsentRecord> {
    let Ok(text) = fs.load(path).await else {
        return BTreeMap::new();
    };
    match serde_json::from_str::<ConsentFile>(&text) {
        Ok(file) => file.extensions,
        Err(error) => {
            log::error!("fyx:ui: ignoring {}: {error}", path.display());
            BTreeMap::new()
        }
    }
}

/// Writes what [`ConsentStore::grant`] returned.
pub async fn save(path: PathBuf, text: String, fs: &dyn Fs) {
    if let Some(dir) = path.parent()
        && let Err(error) = fs.create_dir(dir).await
    {
        log::error!(
            "fyx:ui: cannot save consent to {}: {error:#}",
            path.display()
        );
        return;
    }
    if let Err(error) = fs.atomic_write(path.clone(), text).await {
        log::error!(
            "fyx:ui: cannot save consent to {}: {error:#}",
            path.display()
        );
    }
}
