# Fyx CI

Fyx has two GitHub Actions workflows of its own. All files are named `.github/workflows/fyx_*.yml`
so they never collide with upstream's files on a rebase. The 46 workflows inherited from Zed are
still in `.github/workflows/`, unchanged and disabled (see below).

The authoritative merge gate is still `script/gate.sh`, run by the gatekeeper on the owner's
Windows PC before anything is merged into `integration`. CI runs the same script after the merge,
on a clean machine, so a gate that only passes on a warm local tree shows up red.

## `fyx_gate.yml`: the gate on a clean Windows runner

**Runs on:** every push to `integration` or `fyx/main`, and by hand (`workflow_dispatch`).
**Runner:** `windows-latest` (GitHub-hosted). **Concurrency:** one run per branch; a newer push
cancels the older run on the same branch.

Steps:

1. Enables `core.longpaths` for git (some git dependencies have very long paths).
2. Checks out the full history, blobless, because `gate.sh` diffs against the tag in
   `FYX_UPSTREAM` to find the changed crates.
3. Frees disk by deleting SDKs the build never uses (Android, Haskell, CodeQL, Go, Ruby, PyPy, Java,
   SQL Server, MongoDB, ...), then puts `CARGO_TARGET_DIR` on the drive with the most free space.
   The step logs free space before and after.
4. Installs the MSVC Spectre-mitigated libraries with the Visual Studio installer if the image lacks
   them for its default MSVC version. The `msvc_spectre_libs` build script fails without them.
5. Makes sure CMake is on `PATH` (installs it with Chocolatey if not).
6. Installs the toolchain pinned in `rust-toolchain.toml` (`rustup toolchain install` with no
   arguments reads the file), `cargo-nextest`, and sccache.
7. Runs `bash script/gate.sh` with `CARGO_INCREMENTAL=0`. sccache is the `rustc` wrapper (set in
   `.cargo/config.toml`) and stores its cache in the GitHub Actions cache
   (`SCCACHE_GHA_ENABLED=true`), so later runs reuse compiled crates.
8. Always prints free disk, the size of the target directory and sccache statistics, so you can
   see how close a run came to filling the disk.

`gate.sh` checks, lints and tests every crate changed since the upstream tag, which today includes
upstream crates with `fyx:` hooks (`zed`, `workspace`, `project`, `agent_ui`, `acp_thread`,
`extension_host`, ...), not only the Fyx crates. `FULL=1` (the full `zed` binary build) is not run
in CI.

<!-- TRIAL-RESULTS -->

### Reading a red run

The gate pipes cargo's output through `tail`, so the step log shows the last lines of the failing
command. For the whole output, run `bash script/gate.sh` locally or re-run the failing `cargo`
command with the same `-p` list (printed as the first line, `gate: <crates>`).

## `fyx_rebase_check.yml`: weekly upstream rebase check

**Runs on:** Mondays at 06:17 UTC, and by hand. A manual run takes an optional `tag` input to try a
specific upstream tag instead of the newest one. **Runner:** `ubuntu-latest`.
**Permissions:** `contents: read`, `issues: write`. The checkout keeps no credentials, so the job
cannot push.

Steps:

1. Checks out `integration` with full history.
2. Lists the tags of `zed-industries/zed` and picks the newest stable one (`vX.Y.Z`, no `-pre`
   suffix). It fetches only that tag, into `refs/zed-upstream/<tag>`, so the fork's own copies of
   upstream tags are never overwritten.
3. In a throwaway local branch, runs `git rebase --onto <tag> $(cat FYX_UPSTREAM)`. That replays
   the Fyx stack (`FYX_UPSTREAM..integration`) onto the new tag, the same way the integrator would.
4. If the rebase conflicts, it records where the rebase stopped and its conflicted files, aborts,
   and runs `git merge-tree --merge-base=<FYX_UPSTREAM>` to list every file where the whole stack
   conflicts with the new tag (the rebase alone stops at the first conflict).
5. Issues:
   - On conflict, it opens an issue titled **"Rebase onto `<tag>` conflicts"** with both file lists
     and a link to the run. If such an issue is already open, it updates its title and body instead,
     and comments when the tag changed. There is at most one such issue.
   - When the rebase is clean, or `integration` is already on the newest tag, it closes the open
     issue with a comment, if there is one.

The rebased branch exists only on the runner and is never pushed. Doing the actual rebase,
updating `FYX_UPSTREAM` and re-running the gate stays a manual integrator task.

The schedule only fires for the copy of the workflow on the default branch (`fyx/main`), and
manual runs also need the file to be on `fyx/main`.

## Secrets

None. Both workflows use only the automatic `GITHUB_TOKEN`, with the permissions listed above. The
sccache step uses the Actions cache credentials that every job gets. Nothing needs to be added in
**Settings → Secrets and variables → Actions**.

## Zed's inherited workflows

The 46 upstream workflows stay in `.github/workflows/` and are disabled on this repository through
the GitHub API. Do not delete or edit them: they belong to upstream, and changing them would
conflict on every rebase. Most of them would not work here anyway: they use Zed's self-hosted and
Namespace runners, Zed's secrets, and Zed's infrastructure.

To re-enable one, for example `run_tests.yml`:

- In the browser: **Actions** tab → pick the workflow in the left sidebar → **Enable workflow**.
- With the GitHub CLI:

  ```bash
  gh workflow enable run_tests.yml --repo shelenhovskyi/fyx
  ```

- With the REST API: `PUT /repos/shelenhovskyi/fyx/actions/workflows/run_tests.yml/enable`.

To disable it again, use **Disable workflow** in the same menu, `gh workflow disable`, or
`PUT .../disable`. Check the workflow's `runs-on:` and `secrets.*` first: a workflow that needs a
runner label the repository does not have waits in the queue until it times out.

If you need a modified copy of an upstream workflow, add it as a new `fyx_*.yml` file instead of
editing the original.
