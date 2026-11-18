//! Loads a `fyx:ui@0.1.0` component the way `crates/extension_host/src/wasm_host/fyx_ui.rs` does
//! (same wasmtime version, `Config`, WASI p2 linker, version section check, time budgets) and replays
//! the counter demo's behaviour contract against it, printing size and timings.
//!
//! Usage: `fyx-ui-host-check <component.wasm> [--runs N]`

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use wasmtime::component::{Component, HasData, Linker, ResourceTable};
use wasmtime::{CacheStore, Engine, Store, UpdateDeadline};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../../../crates/ui_ext/wit",
        world: "ui-extension",
        imports: {
            default: trappable,
        },
        exports: {
            default: async,
        },
        additional_derives: [PartialEq],
    });
}

use bindings::fyx::ui::{host as wit_host, types as wit};

const API_VERSION_SECTION: &str = "fyx:ui-api-version";
const UI_EPOCH_INTERVAL: Duration = Duration::from_millis(10);
const UI_CALL_BUDGET: Duration = Duration::from_millis(200);
const UI_INIT_BUDGET: Duration = Duration::from_secs(2);
const ROW_NODE_ID_BASE: u32 = 0x8000_0000;

struct UiState {
    table: ResourceTable,
    ctx: WasiCtx,
    pushes: Vec<(String, wit::Update)>,
    logs: Vec<(wit_host::LogLevel, String)>,
    call_started: Instant,
    call_budget: Duration,
}

impl UiState {
    fn begin_call(&mut self, budget: Duration) {
        self.call_started = Instant::now();
        self.call_budget = budget;
        self.pushes.clear();
    }
}

impl HasData for UiState {
    type Data<'a> = &'a mut UiState;
}

impl WasiView for UiState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

impl wit::Host for UiState {}

impl wit_host::Host for UiState {
    fn push_update(&mut self, surface_id: String, update: wit::Update) -> wasmtime::Result<()> {
        self.pushes.push((surface_id, update));
        Ok(())
    }

    fn log(&mut self, level: wit_host::LogLevel, message: String) -> wasmtime::Result<()> {
        self.logs.push((level, message));
        Ok(())
    }
}

/// Same parsing as `fyx_ui::parse_api_version`, including the duplicate-section rejection.
fn parse_api_version(wasm_bytes: &[u8]) -> Result<Option<(u16, u16, u16)>> {
    let mut version = None;
    for payload in wasmparser::Parser::new(0).parse_all(wasm_bytes) {
        if let wasmparser::Payload::CustomSection(section) = payload.context("parsing component")?
            && section.name() == API_VERSION_SECTION
        {
            let data = section.data();
            ensure!(
                data.len() == 6,
                "{API_VERSION_SECTION} has {} bytes, the host only accepts 6",
                data.len()
            );
            version = Some((
                u16::from_be_bytes([data[0], data[1]]),
                u16::from_be_bytes([data[2], data[3]]),
                u16::from_be_bytes([data[4], data[5]]),
            ));
        }
    }
    Ok(version)
}

/// In memory, like `extension_host`'s `IncrementalCompilationCache` (which caps all extensions
/// together at 32 MB); records how many bytes a component would take of it.
#[derive(Debug, Default)]
struct MemoryCache {
    entries: Mutex<HashMap<Vec<u8>, Vec<u8>>>,
}

impl MemoryCache {
    fn bytes(&self) -> usize {
        self.entries
            .lock()
            .map(|entries| {
                entries
                    .iter()
                    .map(|(key, value)| key.len() + value.len())
                    .sum()
            })
            .unwrap_or_default()
    }
}

impl CacheStore for MemoryCache {
    fn get(&self, key: &[u8]) -> Option<Cow<'_, [u8]>> {
        let entries = self.entries.lock().ok()?;
        entries.get(key).map(|value| Cow::Owned(value.clone()))
    }

    fn insert(&self, key: &[u8], value: Vec<u8>) -> bool {
        match self.entries.lock() {
            Ok(mut entries) => {
                entries.insert(key.to_vec(), value);
                true
            }
            Err(_) => false,
        }
    }
}

fn build_engine(stop_epoch: Arc<AtomicBool>, cache: Arc<MemoryCache>) -> Result<Engine> {
    let mut config = wasmtime::Config::new();
    config.wasm_component_model(true);
    config.epoch_interruption(true);
    config
        .enable_incremental_compilation(cache)
        .map_err(anyhow::Error::from)?;
    let engine = Engine::new(&config).map_err(anyhow::Error::from)?;
    let weak_engine = engine.weak();
    std::thread::Builder::new()
        .name("fyx-ui-epoch".into())
        .spawn(move || {
            while !stop_epoch.load(Ordering::Relaxed) {
                std::thread::sleep(UI_EPOCH_INTERVAL);
                let Some(engine) = weak_engine.upgrade() else {
                    break;
                };
                engine.increment_epoch();
            }
        })
        .context("spawning the epoch thread")?;
    Ok(engine)
}

fn new_store(engine: &Engine) -> Store<UiState> {
    let mut wasi = WasiCtxBuilder::new();
    wasi.inherit_stderr();
    let mut store = Store::new(
        engine,
        UiState {
            table: ResourceTable::new(),
            ctx: wasi.build(),
            pushes: Vec::new(),
            logs: Vec::new(),
            call_started: Instant::now(),
            call_budget: UI_INIT_BUDGET,
        },
    );
    store.epoch_deadline_callback(|context| {
        let state = context.data();
        if state.call_started.elapsed() > state.call_budget {
            Ok(UpdateDeadline::Interrupt)
        } else {
            Ok(UpdateDeadline::Yield(1))
        }
    });
    store.set_epoch_deadline(1);
    store
}

/// Resident memory of this process, from `/proc/self/statm` (Linux only).
fn resident_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let resident_pages = statm.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(resident_pages * 4096)
}

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort();
    samples.get(samples.len() / 2).copied().unwrap_or_default()
}

fn ms(duration: Duration) -> String {
    format!("{:.2} ms", duration.as_secs_f64() * 1000.0)
}

struct Instance {
    extension: bindings::UiExtension,
    store: Store<UiState>,
}

impl Instance {
    async fn surfaces(&mut self) -> Result<Vec<wit::Surface>> {
        self.store.data_mut().begin_call(UI_INIT_BUDGET);
        self.extension
            .call_surfaces(&mut self.store)
            .await
            .map_err(anyhow::Error::from)
    }

    async fn mount(&mut self, surface_id: &str) -> Result<Result<Vec<wit::Node>, String>> {
        self.store.data_mut().begin_call(UI_CALL_BUDGET);
        self.extension
            .call_mount(&mut self.store, surface_id)
            .await
            .map_err(anyhow::Error::from)
    }

    async fn event(
        &mut self,
        surface_id: &str,
        event: wit::UiEvent,
    ) -> Result<Result<wit::Update, String>> {
        self.store.data_mut().begin_call(UI_CALL_BUDGET);
        self.extension
            .call_handle_event(&mut self.store, surface_id, &event)
            .await
            .map_err(anyhow::Error::from)
    }

    async fn click(&mut self, id: u32) -> Result<wit::Update> {
        self.event("counter", wit::UiEvent::Click(id))
            .await?
            .map_err(|error| anyhow!("click({id}) failed: {error}"))
    }

    async fn rows(&mut self, list: u32, start: u32, count: u32) -> Result<Vec<wit::ListRow>> {
        self.store.data_mut().begin_call(UI_CALL_BUDGET);
        self.extension
            .call_list_rows(&mut self.store, "counter", list, start, count)
            .await
            .map_err(anyhow::Error::from)
    }
}

async fn instantiate(
    engine: &Engine,
    component: &Component,
    linker: &Linker<UiState>,
) -> Result<Instance> {
    let mut store = new_store(engine);
    let extension = tokio::time::timeout(
        UI_INIT_BUDGET,
        bindings::UiExtension::instantiate_async(&mut store, component, linker),
    )
    .await
    .context("instantiation ran past the 2 s budget")?
    .map_err(anyhow::Error::from)?;
    Ok(Instance { extension, store })
}

fn set_props_ids(update: &wit::Update) -> Result<Vec<u32>> {
    let wit::Update::Patches(patches) = update else {
        bail!("expected patches, got {update:?}");
    };
    patches
        .iter()
        .map(|patch| match patch {
            wit::Patch::SetProps((id, _)) => Ok(*id),
            patch => Err(anyhow!("expected only set-props, got {patch:?}")),
        })
        .collect()
}

fn text(content: &str, bold: bool) -> wit::Props {
    wit::Props::Text(wit::TextProps {
        content: content.into(),
        color: None,
        bold,
        monospace: false,
    })
}

/// The contract of `extensions/fyx_demo_counter` (its crate doc and `behaves_as_documented`).
async fn check_counter_contract(instance: &mut Instance) -> Result<()> {
    let surfaces = instance.surfaces().await?;
    ensure!(
        surfaces.len() == 1,
        "expected one surface, got {surfaces:?}"
    );
    let surface = &surfaces[0];
    ensure!(surface.id == "counter" && surface.title == "Counter");
    let wit::SurfaceKind::Panel(panel) = &surface.kind else {
        bail!("expected a panel, got {:?}", surface.kind);
    };
    ensure!(
        matches!(panel.position, wit::DockPosition::Right)
            && panel.icon == "hash"
            && panel.default_size == 320
    );

    let nodes = instance
        .mount("counter")
        .await?
        .map_err(|error| anyhow!(error))?;
    let shape = nodes
        .iter()
        .map(|node| (node.id, node.parent))
        .collect::<Vec<_>>();
    ensure!(
        shape
            == [
                (1, None),
                (2, Some(1)),
                (3, Some(1)),
                (4, Some(3)),
                (5, Some(3)),
                (6, Some(3)),
                (7, Some(1)),
            ],
        "first mount shape {shape:?}"
    );
    ensure!(nodes[1].props == text("Count: 0", true));
    ensure!(
        nodes[6].props
            == wit::Props::List(wit::ListProps {
                row_count: 0,
                row_height: 24
            })
    );

    ensure!(instance.click(4).await? == wit::Update::Unchanged);
    ensure!(set_props_ids(&instance.click(5).await?)? == [2, 4, 6, 7]);
    ensure!(set_props_ids(&instance.click(5).await?)? == [2, 7]);

    let rows = instance.rows(7, 1, 10).await?;
    ensure!(rows.len() == 1 && rows[0].index == 1, "rows {rows:?}");
    let row_shape = rows[0]
        .nodes
        .iter()
        .map(|node| (node.id, node.parent))
        .collect::<Vec<_>>();
    ensure!(
        row_shape
            == [
                (ROW_NODE_ID_BASE, None),
                (ROW_NODE_ID_BASE + 1, Some(ROW_NODE_ID_BASE))
            ],
        "row shape {row_shape:?}"
    );
    ensure!(
        rows[0].nodes[1].props == text("Row 2 of 2", false),
        "row text {:?}",
        rows[0].nodes[1].props
    );

    ensure!(set_props_ids(&instance.click(6).await?)? == [2, 4, 6, 7]);
    ensure!(instance.click(6).await? == wit::Update::Unchanged);
    ensure!(instance.rows(7, 0, 10).await?.is_empty());
    ensure!(
        instance
            .event("other", wit::UiEvent::Click(5))
            .await?
            .is_err()
    );
    ensure!(instance.mount("other").await?.is_err());

    instance.click(5).await?;
    let remounted = instance
        .mount("counter")
        .await?
        .map_err(|error| anyhow!(error))?;
    let ids = remounted.iter().map(|node| node.id).collect::<Vec<_>>();
    ensure!(
        ids == [8, 9, 10, 11, 12, 13, 14],
        "second mount ids {ids:?}"
    );
    ensure!(remounted[1].props == text("Count: 1", true));
    ensure!(instance.click(5).await? == wit::Update::Unchanged);
    ensure!(set_props_ids(&instance.click(12).await?)? == [9, 14]);
    ensure!(instance.rows(14, 0, 10).await?.len() == 2);
    ensure!(instance.rows(7, 0, 10).await?.is_empty());
    ensure!(instance.store.data().pushes.is_empty(), "unexpected pushes");
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(
        args.next()
            .context("usage: fyx-ui-host-check <component.wasm> [--runs N]")?,
    );
    let runs = match (args.next().as_deref(), args.next()) {
        (Some("--runs"), Some(runs)) => runs.parse::<usize>().context("--runs")?.max(1),
        _ => 5,
    };

    let wasm_bytes = std::fs::read(&path).with_context(|| format!("reading {path:?}"))?;
    println!("component: {}", path.display());
    println!(
        "size: {} bytes ({:.2} MiB)",
        wasm_bytes.len(),
        wasm_bytes.len() as f64 / (1024.0 * 1024.0)
    );
    match parse_api_version(&wasm_bytes)? {
        Some((major, minor, patch)) => {
            println!("{API_VERSION_SECTION}: {major}.{minor}.{patch}");
            ensure!(
                (major, minor, patch) == (0, 1, 0),
                "the host supports exactly 0.1.0"
            );
        }
        None => println!("{API_VERSION_SECTION}: missing (host falls back to import versions)"),
    }

    let stop_epoch = Arc::new(AtomicBool::new(false));
    let cache = Arc::new(MemoryCache::default());
    let engine = build_engine(stop_epoch.clone(), cache.clone())?;

    let compile_started = Instant::now();
    let component = Component::new(&engine, &wasm_bytes).map_err(anyhow::Error::from)?;
    println!(
        "compile (cranelift, cold cache, as on every Fyx start): {}",
        ms(compile_started.elapsed())
    );
    println!(
        "incremental cache entries: {:.2} MiB (Fyx caps all extensions at 32 MiB)",
        cache.bytes() as f64 / (1024.0 * 1024.0)
    );
    let recompile_started = Instant::now();
    Component::new(&engine, &wasm_bytes).map_err(anyhow::Error::from)?;
    println!(
        "compile again (warm cache, e.g. reloading the extension): {}",
        ms(recompile_started.elapsed())
    );

    let imports = component
        .component_type()
        .imports(&engine)
        .map(|(name, _)| name.to_string())
        .collect::<Vec<_>>();
    println!("imports ({}):", imports.len());
    for import in &imports {
        println!("  {import}");
    }

    let mut linker = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker).map_err(anyhow::Error::from)?;
    bindings::UiExtension::add_to_linker::<_, UiState>(&mut linker, |state| state)
        .map_err(anyhow::Error::from)?;

    let mut instantiate_times = Vec::new();
    let mut surfaces_times = Vec::new();
    let mut mount_times = Vec::new();
    let mut click_times = Vec::new();
    for _ in 0..runs {
        let started = Instant::now();
        let mut instance = instantiate(&engine, &component, &linker).await?;
        instantiate_times.push(started.elapsed());

        let started = Instant::now();
        instance.surfaces().await?;
        surfaces_times.push(started.elapsed());

        let started = Instant::now();
        instance
            .mount("counter")
            .await?
            .map_err(|error| anyhow!(error))?;
        mount_times.push(started.elapsed());

        let started = Instant::now();
        instance.click(5).await?;
        click_times.push(started.elapsed());
    }
    println!("median of {runs} fresh instances:");
    println!("  instantiate: {}", ms(median(instantiate_times)));
    println!(
        "  surfaces (first call, creates the extension): {}",
        ms(median(surfaces_times))
    );
    println!("  first mount: {}", ms(median(mount_times)));
    println!("  first click: {}", ms(median(click_times)));

    const LIVE_INSTANCES: u64 = 16;
    if let Some(before) = resident_bytes() {
        let mut live = Vec::new();
        for _ in 0..LIVE_INSTANCES {
            let mut instance = instantiate(&engine, &component, &linker).await?;
            instance.surfaces().await?;
            instance
                .mount("counter")
                .await?
                .map_err(|error| anyhow!(error))?;
            live.push(instance);
        }
        if let Some(after) = resident_bytes() {
            println!(
                "resident memory per mounted instance: {:.2} MiB (mean of {LIVE_INSTANCES})",
                after.saturating_sub(before) as f64 / (LIVE_INSTANCES as f64 * 1024.0 * 1024.0)
            );
        }
    }

    let mut instance = instantiate(&engine, &component, &linker).await?;
    check_counter_contract(&mut instance).await?;
    println!("counter contract: ok");
    for (level, message) in &instance.store.data().logs {
        println!("  guest log [{level:?}]: {message}");
    }

    stop_epoch.store(true, Ordering::Relaxed);
    Ok(())
}
