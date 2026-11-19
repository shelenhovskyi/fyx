//! Guest components for tests, built for `wasm32-wasip2` by the tests themselves (feature
//! `test-support`). One place for `ui_ext_wasm`'s and `ui_ext_host`'s tests: the fixture workspace
//! in `crates/ui_ext_wasm/fixtures` (one lockfile for every fixture) and the demo extensions of
//! the Fyx workspace. Everything is built `--locked` into `<target>/fyx-ui-fixtures`, next to the
//! test binary's own target directory, once per test process and component; under nextest (a
//! process per test), once per test run and component.

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{LazyLock, Mutex},
};

use std::collections::HashMap;

/// A component of the fixture workspace (`probe`, `status_item`), by package name: built in
/// release mode, which keeps them small.
pub fn fixture_wasm(package: &str) -> Vec<u8> {
    build(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures"),
        package,
        "release",
    )
}

/// A demo extension of the Fyx workspace (`fyx_demo_counter`, `fyx_demo_gallery`), by package
/// name, in debug mode as `script/fyx-build-demos` builds it.
pub fn demo_wasm(package: &str) -> Vec<u8> {
    build(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."),
        package,
        "debug",
    )
}

fn build(workspace: &Path, package: &str, profile: &str) -> Vec<u8> {
    static BUILT: LazyLock<Mutex<HashMap<String, Result<PathBuf, String>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    // Held while building, so concurrent tests of one process build each component once.
    let mut built = BUILT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = built
        .entry(format!("{}:{package}:{profile}", workspace.display()))
        .or_insert_with(|| build_uncached(workspace, package, profile))
        .clone()
        .unwrap_or_else(|error| panic!("{error}"));
    std::fs::read(&path).unwrap_or_else(|error| panic!("reading {path:?}: {error}"))
}

/// Builds `package` and returns its `.wasm`. Under nextest, which runs every test in a process
/// of its own, the first process to take the component's lock file runs `cargo build` and stamps
/// it with `NEXTEST_RUN_ID`, and the others find the stamp: the sources do not change during a
/// run. Plain `cargo test` builds once per process, as before.
fn build_uncached(workspace: &Path, package: &str, profile: &str) -> Result<PathBuf, String> {
    use std::hash::{DefaultHasher, Hash as _, Hasher as _};

    // <target>/debug/deps/<test binary>
    let target_dir = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .ancestors()
        .nth(3)
        .ok_or("test binary is not inside a cargo target directory")?
        .join("fyx-ui-fixtures");
    let built = target_dir
        .join("wasm32-wasip2")
        .join(profile)
        .join(format!("{package}.wasm"));
    let Ok(run_id) = std::env::var("NEXTEST_RUN_ID") else {
        run_cargo_build(workspace, &target_dir, package, profile)?;
        return Ok(built);
    };
    let mut hasher = DefaultHasher::new();
    (workspace, package, profile).hash(&mut hasher);
    let stamp = target_dir.join(format!("fyx-built-{:016x}", hasher.finish()));
    std::fs::create_dir_all(&target_dir)
        .map_err(|error| format!("creating {target_dir:?}: {error}"))?;
    let lock = std::fs::File::create(stamp.with_extension("lock"))
        .map_err(|error| format!("creating the lock for {stamp:?}: {error}"))?;
    // Held until this process has built the component, or seen that another one did.
    lock.lock()
        .map_err(|error| format!("locking {stamp:?}: {error}"))?;
    if std::fs::read_to_string(&stamp).is_ok_and(|stamped| stamped == run_id) && built.exists() {
        return Ok(built);
    }
    run_cargo_build(workspace, &target_dir, package, profile)?;
    std::fs::write(&stamp, &run_id).map_err(|error| format!("writing {stamp:?}: {error}"))?;
    Ok(built)
}

fn run_cargo_build(
    workspace: &Path,
    target_dir: &Path,
    package: &str,
    profile: &str,
) -> Result<(), String> {
    let mut args = vec![
        "build",
        "--locked",
        "--target",
        "wasm32-wasip2",
        "-p",
        package,
    ];
    if profile == "release" {
        args.push("--release");
    }
    // Blocking is fine: this runs once per component and test process, before executor work.
    #[allow(clippy::disallowed_methods)]
    let status = Command::new(option_env!("CARGO").unwrap_or("cargo"))
        .args(&args)
        .arg("--target-dir")
        .arg(target_dir)
        .current_dir(workspace)
        .status()
        .map_err(|error| format!("failed to run cargo: {error}"))?;
    if !status.success() {
        return Err(format!(
            "cargo {args:?} in {} failed: {status}",
            workspace.display()
        ));
    }
    Ok(())
}
