//! An opt-in cache of compiled components, for test suites that load the same components in many
//! processes (nextest runs each test in its own): one compile per component and machine instead
//! of one per test. Off unless [`cache_compiled_components`] names a directory.
//!
//! A cached file holds the component's wasm bytes next to wasmtime's serialized code, and is used
//! only when those bytes equal the ones being loaded, so a hash collision cannot hand back another
//! component's code. wasmtime still checks that the code fits this engine and wasmtime version;
//! a file that does not is compiled again and replaced.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, OnceLock, PoisonError};

use anyhow::{Context as _, Result};
use wasmtime::Engine;
use wasmtime::component::Component;

static DIR: OnceLock<PathBuf> = OnceLock::new();

/// Keeps every component the harness compiles in `dir`, and reuses them in this and later
/// processes. Only for directories nothing but the harness writes, such as one inside a cargo
/// target directory: the cached code is run as it is. The first call wins; later ones are
/// ignored.
pub fn cache_compiled_components(dir: impl Into<PathBuf>) {
    let _ = DIR.set(dir.into());
}

/// `Component::from_binary`, through the cache when it is on.
pub(crate) fn compile(engine: &Engine, wasm: &[u8]) -> Result<Component> {
    let Some(dir) = DIR.get() else {
        return from_binary(engine, wasm);
    };
    // Loaded components of this process, by their bytes.
    static LOADED: LazyLock<Mutex<HashMap<Vec<u8>, Component>>> = LazyLock::new(Default::default);
    if let Some(component) = LOADED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(wasm)
    {
        return Ok(component.clone());
    }
    let file = dir.join(file_name(engine, wasm));
    let component = match read(engine, &file, wasm) {
        Some(component) => component,
        None => {
            let component = from_binary(engine, wasm)?;
            // A failed write only costs the next process a compile.
            let _ = write(&component, &file, wasm);
            component
        }
    };
    LOADED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(wasm.to_vec(), component.clone());
    Ok(component)
}

fn from_binary(engine: &Engine, wasm: &[u8]) -> Result<Component> {
    Component::from_binary(engine, wasm)
        .map_err(anyhow::Error::from)
        .context("compiling the component")
}

fn file_name(engine: &Engine, wasm: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    engine.precompile_compatibility_hash().hash(&mut hasher);
    wasm.hash(&mut hasher);
    format!("{:016x}-{}.cwasm", hasher.finish(), wasm.len())
}

/// The cached component of `file`: `[wasm length: u64 LE][wasm][serialized code]`.
fn read(engine: &Engine, file: &Path, wasm: &[u8]) -> Option<Component> {
    // Read into memory rather than `deserialize_file`, which maps the file: on Windows a mapped
    // file cannot be replaced while another test process still has it open.
    let bytes = std::fs::read(file).ok()?;
    let (len, rest) = bytes.split_first_chunk::<8>()?;
    let len = usize::try_from(u64::from_le_bytes(*len)).ok()?;
    if rest.get(..len)? != wasm {
        return None;
    }
    // SAFETY: the file was written by `write` below from code this harness compiled, in a
    // directory the caller of `cache_compiled_components` guarantees only the harness writes.
    // wasmtime still checks the header against this engine.
    unsafe { Component::deserialize(engine, &rest[len..]) }.ok()
}

/// Writes through a temporary file of this process and a rename, so a reader never sees a torn
/// file. A rename that loses to another process's is fine: both wrote the same component.
fn write(component: &Component, file: &Path, wasm: &[u8]) -> Result<()> {
    let code = component.serialize().map_err(anyhow::Error::from)?;
    let dir = file.parent().context("no parent directory")?;
    std::fs::create_dir_all(dir)?;
    let mut bytes = Vec::with_capacity(8 + wasm.len() + code.len());
    bytes.extend_from_slice(&(wasm.len() as u64).to_le_bytes());
    bytes.extend_from_slice(wasm);
    bytes.extend_from_slice(&code);
    let temp = file.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temp, bytes)?;
    if let Err(error) = std::fs::rename(&temp, file) {
        let _ = std::fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}
