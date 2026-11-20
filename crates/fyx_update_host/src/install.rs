//! Swapping a verified build in (plan 6.2, `docs/fyx/platform/self-update.md`): unpacking into the
//! staged folder beside the installed app, running `fyx_update`'s rename plans, the Windows helper
//! that runs them after Fyx exits, and the record that rolls back a version that does not start.
//!
//! Every step is a rename or a removal on one volume. When a rename fails, the ones before it are
//! undone in reverse, so the installed app is never left missing.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use async_compression::futures::bufread::GzipDecoder;
use futures::io::{BufReader, Cursor};
use fyx_update::{ArtifactKind, Layout, Step};
use semver::Version;
use serde::{Deserialize, Serialize};

/// The record of an update that has been swapped in but has not started yet.
const PENDING_FILE: &str = "pending.json";
/// Starts of the new version that may end before it counts as started; the next start rolls back.
pub const MAX_FAILED_STARTS: u32 = 2;

/// The installed app for the running executable `exe`: the `.app` bundle on macOS
/// (`Fyx.app/Contents/MacOS/fyx`), the executable's folder elsewhere.
pub fn installed_location(exe: &Path, os: &str) -> Option<PathBuf> {
    if os == "macos" {
        let bundle = exe.parent()?.parent()?.parent()?;
        let is_bundle = bundle
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"));
        return is_bundle.then(|| bundle.to_path_buf());
    }
    exe.parent().map(Path::to_path_buf)
}

/// Unpacks `archive` into `layout.staged`, replacing an earlier staged copy. An archive whose
/// only top-level entry is a folder (`Fyx.app/`, `fyx/`) stages that folder. Entries that would
/// leave the folder are skipped (zip) or refused (tar).
pub async fn stage(archive: Vec<u8>, kind: ArtifactKind, layout: &Layout) -> Result<()> {
    let unpack_dir = layout.staged.with_extension("unpacking");
    remove_if_present(&unpack_dir)?;
    remove_if_present(&layout.staged)?;
    std::fs::create_dir_all(&unpack_dir)
        .with_context(|| format!("creating {}", unpack_dir.display()))?;
    let unpacked = match kind {
        ArtifactKind::WindowsZip => util::archive::extract_zip(&unpack_dir, Cursor::new(archive))
            .await
            .context("unpacking the zip"),
        ArtifactKind::MacAppTarGz | ArtifactKind::LinuxTarGz => {
            async_tar::Archive::new(GzipDecoder::new(BufReader::new(Cursor::new(archive))))
                .unpack(&unpack_dir)
                .await
                .context("unpacking the tar.gz")
        }
    };
    if let Err(error) = unpacked {
        remove_if_present(&unpack_dir)?;
        return Err(error);
    }
    let root = single_folder(&unpack_dir)?.unwrap_or_else(|| unpack_dir.clone());
    std::fs::rename(&root, &layout.staged)
        .with_context(|| format!("moving the unpacked build to {}", layout.staged.display()))?;
    remove_if_present(&unpack_dir)
}

/// The only entry of `dir`, if it is a folder.
fn single_folder(dir: &Path) -> Result<Option<PathBuf>> {
    let mut entries = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<std::io::Result<Vec<_>>>()
        .with_context(|| format!("reading {}", dir.display()))?;
    if entries.len() != 1 {
        return Ok(None);
    }
    let entry = entries.remove(0);
    let is_dir = entry
        .file_type()
        .with_context(|| format!("reading {}", entry.path().display()))?
        .is_dir();
    Ok(is_dir.then(|| entry.path()))
}

fn remove_if_present(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => Err(error),
    }
    .with_context(|| format!("removing {}", path.display()))
}

/// Runs `steps` in order. If a rename fails, the renames already done are undone in reverse and
/// the error is returned. Removals are not undone: they only delete leftovers of older updates.
pub fn run_steps(steps: &[Step]) -> Result<()> {
    let mut done: Vec<(&Path, &Path)> = Vec::new();
    for step in steps {
        let result = match step {
            Step::RemoveIfPresent(path) => remove_if_present(path),
            Step::Rename { from, to } => std::fs::rename(from, to)
                .with_context(|| format!("renaming {} to {}", from.display(), to.display()))
                .map(|()| done.push((from, to))),
        };
        if let Err(error) = result {
            for (from, to) in done.into_iter().rev() {
                if let Err(undo) = std::fs::rename(to, from) {
                    log::error!(
                        "fyx update: could not undo {} -> {}: {undo}",
                        from.display(),
                        to.display()
                    );
                }
            }
            return Err(error);
        }
    }
    Ok(())
}

/// A PowerShell script that waits for process `pid` to exit, runs `steps` like [`run_steps`]
/// (retrying each rename while Windows still holds the files), writes `ok` or the error to `log`,
/// and starts `relaunch` if given. The paths are single-quoted literals.
pub fn windows_helper_script(
    pid: u32,
    steps: &[Step],
    relaunch: Option<&Path>,
    log: &Path,
) -> String {
    let mut script = format!(
        "$ErrorActionPreference = 'Stop'\n\
         $log = {}\n\
         Wait-Process -Id {pid} -Timeout 120 -ErrorAction SilentlyContinue\n\
         $done = New-Object System.Collections.ArrayList\n\
         function Move-Retrying($from, $to) {{\n\
         \x20   for ($try = 1; ; $try++) {{\n\
         \x20       try {{ Move-Item -LiteralPath $from -Destination $to; break }}\n\
         \x20       catch {{ if ($try -ge 20) {{ throw }}; Start-Sleep -Milliseconds 500 }}\n\
         \x20   }}\n\
         \x20   [void]$done.Add(@($from, $to))\n\
         }}\n\
         try {{\n",
        quote(log)
    );
    for step in steps {
        match step {
            Step::RemoveIfPresent(path) => script.push_str(&format!(
                "    if (Test-Path -LiteralPath {path}) {{ Remove-Item -LiteralPath {path} -Recurse -Force }}\n",
                path = quote(path)
            )),
            Step::Rename { from, to } => script.push_str(&format!(
                "    Move-Retrying {} {}\n",
                quote(from),
                quote(to)
            )),
        }
    }
    script.push_str(
        "    Set-Content -LiteralPath $log -Value 'ok'\n\
         } catch {\n\
         \x20   $failure = \"$_\"\n\
         \x20   for ($i = $done.Count - 1; $i -ge 0; $i--) {\n\
         \x20       try { Move-Item -LiteralPath $done[$i][1] -Destination $done[$i][0] } catch {}\n\
         \x20   }\n\
         \x20   Set-Content -LiteralPath $log -Value \"failed: $failure\"\n\
         }\n",
    );
    if let Some(relaunch) = relaunch {
        script.push_str(&format!("Start-Process -FilePath {}\n", quote(relaunch)));
    }
    script
}

/// A PowerShell single-quoted string literal.
fn quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

/// An update swapped in and waiting to prove it starts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pending {
    pub version: Version,
    pub installed: PathBuf,
    /// Starts of `version` that did not reach "started" yet.
    pub failed_starts: u32,
}

/// What to do about a pending update when Fyx starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartDecision {
    /// No update is pending.
    Nothing,
    /// The new version is running: count this start, and call [`mark_started`] once it is up.
    Watch(Pending),
    /// The new version failed to start too often: roll back to the previous one.
    RollBack(Pending),
    /// Another version is running than the one swapped in: the swap did not happen.
    NotInstalled(Pending),
}

pub fn read_pending(updates_dir: &Path) -> Option<Pending> {
    let bytes = std::fs::read(updates_dir.join(PENDING_FILE)).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn write_pending(updates_dir: &Path, pending: &Pending) -> Result<()> {
    std::fs::create_dir_all(updates_dir)
        .with_context(|| format!("creating {}", updates_dir.display()))?;
    let bytes = serde_json::to_vec_pretty(pending).context("encoding the pending update")?;
    std::fs::write(updates_dir.join(PENDING_FILE), bytes).context("saving the pending update")
}

/// The new version has started: nothing is pending any more.
pub fn mark_started(updates_dir: &Path) -> Result<()> {
    remove_if_present(&updates_dir.join(PENDING_FILE))
}

/// Decides at start, and records this start of the new version.
pub fn on_start(updates_dir: &Path, running: &Version) -> Result<StartDecision> {
    let Some(mut pending) = read_pending(updates_dir) else {
        return Ok(StartDecision::Nothing);
    };
    if pending.version != *running {
        mark_started(updates_dir)?;
        return Ok(StartDecision::NotInstalled(pending));
    }
    if pending.failed_starts >= MAX_FAILED_STARTS {
        mark_started(updates_dir)?;
        return Ok(StartDecision::RollBack(pending));
    }
    pending.failed_starts += 1;
    write_pending(updates_dir, &pending)?;
    Ok(StartDecision::Watch(pending))
}

/// Refuses a layout whose folders are missing, before anything is renamed.
pub fn check_ready(layout: &Layout, staged_needed: bool) -> Result<()> {
    if !layout.installed.is_dir() {
        bail!("{} is not an installed Fyx", layout.installed.display());
    }
    if staged_needed && !layout.staged.is_dir() {
        bail!("no unpacked update at {}", layout.staged.display());
    }
    if !staged_needed && !layout.previous.is_dir() {
        bail!("no previous version at {}", layout.previous.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(root: &Path, files: &[(&str, &str)]) {
        for (path, text) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("dirs");
            std::fs::write(path, text).expect("write");
        }
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).expect("read")
    }

    fn layout(root: &Path) -> Layout {
        let layout = Layout::beside(&root.join("Fyx"));
        tree(&layout.installed, &[("fyx.exe", "1.0.0")]);
        tree(&layout.staged, &[("fyx.exe", "1.0.1")]);
        layout
    }

    #[test]
    fn install_then_roll_back() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = layout(dir.path());
        check_ready(&layout, true).expect("ready");
        run_steps(&fyx_update::install_plan(&layout)).expect("install");
        assert_eq!(read(&layout.installed.join("fyx.exe")), "1.0.1");
        assert_eq!(read(&layout.previous.join("fyx.exe")), "1.0.0");
        assert!(!layout.staged.exists());

        check_ready(&layout, false).expect("ready to roll back");
        run_steps(&fyx_update::rollback_plan(&layout)).expect("roll back");
        assert_eq!(read(&layout.installed.join("fyx.exe")), "1.0.0");
        assert_eq!(read(&layout.staged.join("fyx.exe")), "1.0.1");
    }

    #[test]
    fn a_failed_rename_undoes_the_ones_before() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = layout(dir.path());
        std::fs::remove_dir_all(&layout.staged).expect("remove");
        assert!(run_steps(&fyx_update::install_plan(&layout)).is_err());
        assert_eq!(
            read(&layout.installed.join("fyx.exe")),
            "1.0.0",
            "the installed app is back where it was"
        );
        assert!(!layout.previous.exists());
    }

    #[test]
    fn the_installed_app_of_an_executable() {
        assert_eq!(
            installed_location(
                Path::new("/Applications/Fyx.app/Contents/MacOS/fyx"),
                "macos"
            ),
            Some(PathBuf::from("/Applications/Fyx.app"))
        );
        assert_eq!(
            installed_location(Path::new("/usr/local/bin/fyx"), "macos"),
            None
        );
        assert_eq!(
            installed_location(Path::new(r"C:\Programs\Fyx\fyx.exe"), "windows"),
            Some(PathBuf::from(r"C:\Programs\Fyx"))
        );
    }

    #[test]
    fn a_version_that_does_not_start_twice_is_rolled_back() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let new = Version::new(1, 0, 1);
        let pending = Pending {
            version: new.clone(),
            installed: dir.path().join("Fyx"),
            failed_starts: 0,
        };
        assert_eq!(
            on_start(dir.path(), &new).expect("start"),
            StartDecision::Nothing
        );
        write_pending(dir.path(), &pending).expect("write");
        assert!(matches!(
            on_start(dir.path(), &new),
            Ok(StartDecision::Watch(_))
        ));
        assert!(matches!(
            on_start(dir.path(), &new),
            Ok(StartDecision::Watch(_))
        ));
        assert!(matches!(
            on_start(dir.path(), &new),
            Ok(StartDecision::RollBack(_))
        ));
        assert_eq!(
            on_start(dir.path(), &new).expect("start"),
            StartDecision::Nothing
        );

        write_pending(dir.path(), &pending).expect("write");
        assert!(matches!(
            on_start(dir.path(), &new),
            Ok(StartDecision::Watch(_))
        ));
        mark_started(dir.path()).expect("started");
        assert_eq!(
            on_start(dir.path(), &new).expect("start"),
            StartDecision::Nothing
        );

        write_pending(dir.path(), &pending).expect("write");
        assert!(matches!(
            on_start(dir.path(), &Version::new(1, 0, 0)),
            Ok(StartDecision::NotInstalled(_))
        ));
    }

    #[test]
    fn staging_a_zip_takes_its_single_folder() {
        smol::block_on(async {
            let dir = tempfile::tempdir().expect("a temp dir");
            let layout = Layout::beside(&dir.path().join("Fyx"));
            let archive = zip(&[("fyx/fyx.exe", "1.0.1"), ("fyx/assets/a.txt", "a")]).await;
            stage(archive, ArtifactKind::WindowsZip, &layout)
                .await
                .expect("staged");
            assert_eq!(read(&layout.staged.join("fyx.exe")), "1.0.1");
            assert_eq!(read(&layout.staged.join("assets").join("a.txt")), "a");
            assert!(!layout.staged.with_extension("unpacking").exists());
        });
    }

    #[test]
    fn staging_a_zip_skips_entries_that_leave_the_folder() {
        smol::block_on(async {
            let dir = tempfile::tempdir().expect("a temp dir");
            let layout = Layout::beside(&dir.path().join("inner").join("Fyx"));
            let archive = zip(&[("fyx.exe", "1.0.1"), ("../escaped.txt", "x")]).await;
            stage(archive, ArtifactKind::WindowsZip, &layout)
                .await
                .expect("staged");
            assert_eq!(read(&layout.staged.join("fyx.exe")), "1.0.1");
            assert!(!dir.path().join("inner").join("escaped.txt").exists());
            assert!(!dir.path().join("escaped.txt").exists());
        });
    }

    async fn zip(files: &[(&str, &str)]) -> Vec<u8> {
        use async_zip::{Compression, ZipEntryBuilder, base::write::ZipFileWriter};
        {
            let mut writer = ZipFileWriter::new(Vec::new());
            for (name, text) in files {
                writer
                    .write_entry_whole(
                        ZipEntryBuilder::new((*name).to_string().into(), Compression::Stored),
                        text.as_bytes(),
                    )
                    .await
                    .expect("a zip entry");
            }
            writer.close().await.expect("a zip")
        }
    }

    #[cfg(windows)]
    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "a test waits for the helper on purpose"
    )]
    fn the_windows_helper_installs_after_the_process_exits_and_relaunches_nothing_else() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = layout(dir.path());
        let log = dir.path().join("helper.log");
        // A stand-in for Fyx: a process that exits on its own after a moment.
        let mut waiting = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-Command", "Start-Sleep -Milliseconds 800"])
            .spawn()
            .expect("a process to wait for");
        let script =
            windows_helper_script(waiting.id(), &fyx_update::install_plan(&layout), None, &log);
        let script_path = dir.path().join("helper.ps1");
        std::fs::write(&script_path, script).expect("write the script");
        let status = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script_path)
            .status()
            .expect("run the helper");
        assert!(status.success());
        waiting.wait().expect("the stand-in exited");
        assert_eq!(read(&log).trim(), "ok");
        assert_eq!(read(&layout.installed.join("fyx.exe")), "1.0.1");
        assert_eq!(read(&layout.previous.join("fyx.exe")), "1.0.0");
    }

    #[cfg(windows)]
    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "a test waits for the helper on purpose"
    )]
    fn the_windows_helper_undoes_a_failed_install() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let layout = layout(dir.path());
        std::fs::remove_dir_all(&layout.staged).expect("remove");
        let log = dir.path().join("helper.log");
        let script = windows_helper_script(
            std::process::id().wrapping_add(1_000_000),
            &fyx_update::install_plan(&layout),
            None,
            &log,
        );
        let script_path = dir.path().join("it's a helper.ps1");
        std::fs::write(&script_path, script).expect("write the script");
        let status = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script_path)
            .status()
            .expect("run the helper");
        assert!(status.success());
        assert!(read(&log).starts_with("failed:"), "{}", read(&log));
        assert_eq!(read(&layout.installed.join("fyx.exe")), "1.0.0");
    }
}
