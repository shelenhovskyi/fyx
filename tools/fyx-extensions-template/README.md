# fyx-extensions template

The files of the registry repository `shelenhovskyi/fyx-extensions`, as
`docs/fyx/platform/registry-repo.md` designs it (platform plan tasks 3.1 and 3.2). The real
repository is not created yet: that is the owner's step. Until then this folder is the template,
and `scripts/dry-run.sh` runs the pipeline locally.

**No real key exists anywhere in it.** The dry run generates throwaway test keys and deletes them.

## What is here

| Path | What |
|---|---|
| `extensions/fyx-demo-counter.toml` | the sample T0 entry: Fyx's counter demo, built from the Fyx repository |
| `blocklist/blocklist.toml` | the block list source (empty) |
| `keys/` | where the owner's public keyset goes; see `keys/README.md` |
| `policy/registry.toml` | the registry owner and the download base URL |
| `policy/capability-tiers.json` | capability → tier, checked against Fyx's client by a test in `crates/fyx_registry` |
| `policy/reserved-ids.txt` | ids only the registry owner may register (`fyx-*`, …) |
| `policy/builder.toml` | the pinned builder image (digest still a placeholder) |
| `builder/Dockerfile` | the builder image: Rust, `wasm32-wasip2`, `fyx-ext`, `fyx-registry`, `cargo-deny` |
| `scripts/registry.py` | the entry rules, the release fields, the policy decision, the PR comment, the block list; standard library only |
| `scripts/test_registry.py` | its tests |
| `scripts/dry-run.sh` | the pipeline end to end, locally, with test keys |
| `.github/CODEOWNERS` | everything that decides trust is the owner's; `extensions/` is not |
| `.github/workflows/pr.yml` | untrusted: entry, build in the pinned image (`fyx-ext check`/`test`/`package`), keyset check for `keys/` PRs |
| `.github/workflows/review-trigger.yml` | re-runs the decision on a submitted review |
| `.github/workflows/review.yml` | the agent review (fails closed: "no reviewer configured"), the policy decision, statuses, the comment, `needs-owner`, auto-merge |
| `.github/workflows/publish.yml` | rebuild, then sign the release, `index add`, sign the snapshot (`release-signing`) |
| `.github/workflows/blocklist.yml` | sign the block list (`blocklist-signing`) |
| `.github/workflows/refresh.yml` | re-sign the snapshot and the block list every 12 h, plus the expiry watchdog |
| `.github/workflows/dry-run.yml` | `scripts/dry-run.sh` in CI, with test keys made in the run |

## Dry run

From the Fyx checkout, after `script/fyx-build-demos`:

```bash
tools/fyx-extensions-template/scripts/dry-run.sh
```

It runs the policy tests, makes four test keys (release, snapshot, snapshot rotation with
`blocklist-revocation`, block list) and checks the keyset's role separation. Then:

- **pr.yml:** validates the sample entry, packages the demo, derives the release (tier T0).
- **review.yml:**
  - without a reviewer, a contributor's T0 PR waits for the owner (`needs-owner`);
  - with a passing verdict, it auto-merges;
  - the sample (a reserved `fyx-*` id) passes when the owner opens it.
- **publish.yml:** signs the release, adds it to the index, and signs the snapshot.
- **blocklist.yml:** signs the block list.
- **The client check:** `fyx-registry verify client` runs Fyx's registry client over the result.
  The fresh release waits for the 24 h age gate. 25 h later it installs, and the package
  verifies.
- **refresh.yml:** re-signs the snapshot.

`--fyx-ext <bin>` adds `fyx-ext check`, `test` and `package` as in CI; `--keep <dir>` keeps the
published files (never the keys).

## Where this differs from registry-repo.md

- **Review trigger.** `review.yml` runs on `workflow_run` of `pr` and of `review-trigger`, not
  on `pull_request_review` itself. A review on a fork PR gets a read-only token, so it could not
  set statuses.
- **Merge token.** Auto-merge uses a GitHub App token, `AUTO_MERGE_TOKEN`, in its own Environment
  `auto-merge`. A merge made with `GITHUB_TOKEN` would not start `publish.yml`. Without the token,
  the owner merges green PRs by hand.
- **Recomputed decision.** The `report` job rebuilds the whole decision from trusted sources: the
  API, `git show` of the entry and of `extension.toml` at the source commit, and the published
  index. `pr.yml` runs the PR's own version of the workflow, so its artifacts are only trusted for
  the PR number, which is checked against the run's head commit.
- **Reserved ids.** An id in `reserved-ids.txt` is registrable only by an entry that the registry
  owner owns alone, and the PR needs the owner. This is how the sample `fyx-demo-counter` is
  allowed.
- **The owner's own PR.** A PR the registry owner opens counts as the owner's approval, since
  GitHub does not let an author approve their own PR.
- **No T2 yet.** An entry with a T2 capability is refused until `fyx-ext` reads
  `network_domains` and `processes` (registry-repo.md, the entry).

## Before the real repository exists

The owner:

1. Creates the repository from this folder.
2. Sets the branch rules on `main`, with required checks `entry`, `build`, `policy` and
   `agent-review`.
3. Creates the Environments `release-signing`, `blocklist-signing`, `agent-review` and
   `auto-merge`.
4. Makes the keys (registry-repo.md, "Creating the keys") and sets `SNAPSHOT_KEY_ID`.
5. Builds the builder image and puts its digest into `policy/builder.toml`.
6. Runs `blocklist` and then `publish` once by hand (Actions, "Run workflow") with `first_run`
   checked, so the first block list and snapshot start at sequence 1. After that, every run
   needs the published documents: a failed download fails the job rather than restarting the
   sequence (integration-5 R6).
