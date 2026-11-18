# Registry formats

The files the Fyx extension registry publishes and the client reads, as implemented by
`crates/fyx_registry` (Platform 1 [spec](spec.md), decisions 14–21; [plan](plan.md) tasks
3.2–3.4). The crate is pure: it takes bytes and the current time and returns verified documents
and decisions. Fetching, storing and the UI belong to the registry client (3.3, 3.4) and the
Extensions page (3.5).

## Conventions

- **JSON**, UTF-8. Unknown fields are refused everywhere: every field is signed, so a new field
  needs a new schema number (see [Schema changes](#schema-changes)).
- **Times** are Unix seconds, UTC, as JSON integers.
- **Extension ids** are as in `extension.toml`: `a-z`, `0-9` and `-`, starting with a letter, not
  ending with `-`, at most 64 characters.
- **Versions** are `major.minor.patch`; pre-release and build suffixes are refused in schema 1.
- **Hashes** are SHA-256 as 64 lowercase hex digits. **Keys and signatures** are standard base64
  with padding.

## Keys

The registry signs with Ed25519 (`ring`, already in the workspace). A signature names its key:

```json
{"key_id": "fyx-2026-a", "ed25519": "<base64 of 64 bytes>"}
```

Key ids are 1–64 characters of `a-z`, `0-9`, `-` and `.`, starting with a letter or digit.

A **keyset** lists the public keys a reader accepts, each with its **roles**: `release`,
`blocklist` and `snapshot`. A key signs only documents of its roles, so a leaked block-list key
cannot publish a release (security-2 T9). The client compiles in the registry key and one
rotation key (decision 18); the CI tool reads a keyset file:

```json
{"keys": [
  {"key_id": "fyx-2026-a", "ed25519": "<base64 of 32 bytes>", "roles": ["release"]},
  {"key_id": "fyx-2026-b", "ed25519": "<base64 of 32 bytes>", "roles": ["blocklist", "snapshot"]}
]}
```

A signature by a key without the document's role is refused as `WrongRole`. A key listed in the
current snapshot's `revoked_keys` signs nothing any more: releases and block lists are checked
with the keyset minus the revoked keys (`VerifiedSnapshot::effective_keyset`).

Rotation: sign new documents with the rotation key; old releases keep verifying with the old key
while it is in the keyset. Before a client release drops the old key, the CI re-signs every
release with the new one. A signature by a key that is not in the keyset is refused as
`UnknownKey`; one that does not match is `BadSignature`.

The **signing key file** (CI secret, never committed, never created by the tool) holds the
32-byte Ed25519 seed:

```json
{"key_id": "fyx-2026-a", "seed": "<base64 of 32 bytes>"}
```

Any 32 random bytes are a valid seed, e.g. `openssl rand -base64 32`. The tool reads it from
`--key-file` (on Unix the file must not be readable by group or others) or from the
`FYX_REGISTRY_SIGNING_KEY` environment variable (the same JSON), which it removes from its
environment at once. `fyx-registry public-key --role <role>…` prints the matching keyset entry.

## Signed documents

Every signed document is an envelope:

```json
{"signed": { ... }, "signature": {"key_id": "...", "ed25519": "..."}}
```

The signature covers the **canonical bytes** of `signed`, not its JSON text, so re-formatting or
re-ordering the JSON never breaks a signature, and no JSON spelling trick can change what was
signed. The canonical form is a sequence of fields in a fixed order:

| Field kind | Encoding |
|---|---|
| string, byte string | length as big-endian `u64`, then the bytes (strings as UTF-8) |
| `u64` | 8 bytes, big-endian |
| `bool` | 1 byte, `0` or `1` |
| list | length as big-endian `u64`, then each item |

The first field is a domain tag (`fyx-registry/release/v1`, `fyx-registry/blocklist/v1`,
`fyx-registry/snapshot/v1`), so a signature over a release can never pass as one over a block
list. Sets (owners, capabilities,
domains, programs) are encoded sorted and without duplicates. Tests decode the canonical bytes
of random documents back to equal documents, which shows the encoding is unambiguous.

## `index.json`

```json
{
  "schema": 1,
  "generated_at": 1763400000,
  "releases": [
    {
      "signed": {
        "id": "todo",
        "version": "1.2.0",
        "name": "Todo",
        "description": "Keeps a list of things to do.",
        "owners": ["alice"],
        "source": {
          "repository": "https://github.com/alice/todo",
          "commit": "0123456789abcdef0123456789abcdef01234567",
          "path": "."
        },
        "package": {
          "url": "https://github.com/<org>/fyx-extensions/releases/download/<tag>/todo-1.2.0.fyxext",
          "sha256": "<64 hex digits>",
          "size": 48213
        },
        "published_at": 1763390000,
        "urgent": false,
        "tier": "t2",
        "capabilities": ["network:fetch", "ui:panel"],
        "network_domains": ["api.example.com"],
        "processes": []
      },
      "signature": {"key_id": "fyx-2026-a", "ed25519": "<base64>"}
    }
  ]
}
```

Each release is signed on its own, so CI adds a release without re-signing the rest, and a
client uses every release that verifies even if another entry is damaged: each entry is parsed
on its own, and a damaged one is reported as rejected (bugs-1 #12). The index as a whole, its
time and its set of releases, is pinned by the signed snapshot below. `index.json` is at most
32 MiB and 100 000 entries.

Release fields, in canonical order:

| Field | Rules |
|---|---|
| `id` | extension id |
| `version` | `major.minor.patch` |
| `name` | 1–64 characters of letters, digits, spaces and plain punctuation (`-_.,:;!?&+'()/#`); no repeated spaces, no surrounding whitespace |
| `description` | 0–300 characters; no control (Cc), format (Cf: bidi overrides, zero-width) or U+2028/U+2029 characters; no surrounding whitespace |
| `owners` | at least one GitHub login (`A-Z a-z 0-9 -`, 1–39, no leading or trailing `-`); shown as the publisher |
| `source.repository`, `.commit`, `.path` | `https://` URL; full 40-digit lowercase SHA; relative path without `..` (`.` for the root) |
| `package.url`, `.sha256`, `.size` | `https://` URL; hash and size of the `.fyxext`; size 1 byte to 50 MiB (decision 12) |
| `published_at` | when CI published it; the age gate counts from here |
| `urgent` | security fix: skips the age gate |
| `tier` | `t0`, `t1` or `t2`; at least the highest tier among the known capabilities |
| `capabilities` | names like `ui:panel`; ones this client does not know are allowed but never granted silently |
| `network_domains` | optional; `example.com` or `*.example.com`; at least two labels, no IP literals, no `localhost`, and neither a public suffix nor a wildcard over one (`*.com`, `*.co.uk`, `*.github.io`; a PSL snapshot in `text.rs`); needs `network:fetch` |
| `processes` | optional; program names without paths (`A-Z a-z 0-9 . _ + -`, up to 64); needs `process:spawn` |

Duplicates are counted among releases that verify (security-2 S11), so a forged copy cannot evict
the genuine one: identical valid copies count once, two different valid copies of one version
are both rejected. The CI (`verify_strict`) refuses any duplicate. `fyx-registry index add`
refuses to add a version that is already published: a published release is never replaced, only
blocked.

## `snapshot.json`

```json
{
  "signed": {
    "schema": 1,
    "sequence": 913,
    "generated_at": 1763400000,
    "expires_at": 1763572800,
    "index_sha256": "<64 hex digits>",
    "index_size": 1048576,
    "revoked_keys": ["fyx-2025-a"]
  },
  "signature": {"key_id": "fyx-2026-b", "ed25519": "<base64>"}
}
```

Signed by a `snapshot` key (security-2 S6). It names the exact `index.json` by size and SHA-256,
so a mirror cannot withhold a release or serve an older index; its `generated_at` is the signed
time of the index. Rules:

- `sequence` increases with every snapshot, re-signing included. The client keeps the highest
  one it accepted and refuses a lower one, or the same one with other contents (`Rollback`).
- `expires_at` is after `generated_at` and at most 7 days later. The CI re-signs at least daily
  (48 h validity by default); an expired snapshot is refused (`Expired`), so a frozen mirror
  shows up.
- `revoked_keys` (optional, at most 256) are removed from the keyset for releases and block
  lists.

A client verifies the snapshot first (`VerifiedSnapshot::verify`), then the index with
`Index::verify_with_snapshot(bytes, &snapshot, &keyset)`. Canonical order: schema, sequence,
generated_at, expires_at, index_sha256, index_size, revoked_keys.

A downloaded package is checked against `package.size` and `package.sha256` before it is
unpacked (`Release::verify_package`).

## `blocklist.json`

```json
{
  "signed": {
    "schema": 1,
    "sequence": 42,
    "generated_at": 1763400000,
    "expires_at": 1763572800,
    "entries": [
      {"id": "todo", "versions": ">=1.2.0, <1.2.3", "reason": "Uploads open buffers.", "urgent": true},
      {"id": "todo", "versions": "*", "package_sha256": "<64 hex digits>", "reason": "A forged package.", "urgent": true}
    ]
  },
  "signature": {"key_id": "fyx-2026-a", "ed25519": "<base64>"}
}
```

- Signed by a `blocklist` key. At most 4 MiB and 10 000 entries.
- `sequence` increases with every published list; the CI publishes a new one at least daily.
- `expires_at` is after `generated_at` and at most 7 days later (security-2 S5).
- `versions` is `*` (every version) or comparators joined by `,` that must all hold. A comparator
  is `=`, `>`, `>=`, `<` or `<=` followed by `major.minor.patch`. There is no `^` or `~`.
  Versions compare by semver precedence, which ignores build metadata, so `=1.2.0` also blocks
  `1.2.0+local` (bugs-1 #21); pre-release versions compare by plain semver order, so `>=1.2.0`
  also covers `1.3.0-beta`. A local file must have a plain `major.minor.patch` version
  (`check_local_version`).
- `package_sha256` (optional): the entry blocks only the package with that hash, so a file from
  disk cannot escape a block by claiming another version; the install store records each
  installed package's hash (`InstalledVersion::package_sha256`).
- `reason`: 1–300 characters, the same character rules as `description`, shown to the user.

Canonical order: schema, sequence, generated_at, expires_at, then for each entry: id, the
comparators (each operator and version as strings), reason, urgent, whether a package hash
follows, and the hash.

### The cached copy

The client stores the last good list with the time it was fetched:

```json
{"fetched_at": 1763400000, "blocklist": { "signed": { ... }, "signature": { ... } }}
```

It is verified again when loaded. `BlocklistCache::apply_fetch` decides what each fetch does:

| Fetch result | Cache |
|---|---|
| network error, not JSON, bad or unknown signature, newer schema | kept as is (`Keep`); nothing is disabled |
| lower `sequence` than the cache | kept: a replayed old list must not unblock anything |
| same `sequence`, same content | `fetched_at` updated (`Refresh`), never moved backwards; the list's age does not change |
| same `sequence`, different content | kept, reported as a conflict |
| higher `sequence`, or no cache yet | replaced (`Replace`) and applied |

Fetches run at start and every 6 h (`fetch_due`); a last attempt stamped in the future (a fast
clock) does not postpone the next one (bugs-1 #22). `freshness` reports `Missing` (never
fetched: nothing is blocked yet), `Fresh`, or `Stale` once the list's signed `generated_at` is 7
days old or its `expires_at` has passed, however recently it was fetched, for the "Block list is
N days old" warning (spec §4 failure 5). Being offline never disables an extension.

## Age gate

`age_gate(release, now)`: a release installs or updates only when `now >= published_at + 24 h`,
unless `urgent`. A `published_at` in the future counts as just published, so clock skew never
shortens the wait. Rollback to the kept previous version is not gated: it is already installed.

## Update planning

`plan_updates(installed, index, blocklist, now)` returns actions per installed extension. It is
level-triggered: it states what each extension should be now, so the host can run it on every
index or block list change; a lifted block simply stops producing `Disable`.

The installed state per extension (kept in `state.json` by the install store, task 1.1): active
version and its origin (`registry` or `local_file`), the kept previous version, whether the user
pinned the active version, and the permissions the user has approved (the active version's
declaration plus any held update they approved).

Candidates are releases newer than the active version that are not blocked. A candidate is held
when any of these hold, and all that apply are listed:

- `Pinned`: the user pinned the active version;
- `TooNew { ready_at }`: the age gate has not passed;
- `NeedsApproval`: compared with the approved permissions, it adds a T1, T2 or unknown capability,
  a network domain, a program, or raises the tier (decision 21). T0 additions install silently.

The newest candidate that is not held is the update target. Then:

| Active version | Action |
|---|---|
| not blocked | `Update` to the target, if any |
| blocked, a target exists | `Update` to it (`ReplacesBlocked`) |
| blocked, no target, previous kept and not blocked, not pinned | `Rollback` to the previous version |
| blocked otherwise | `Disable`; the extension stays installed (spec §4 failure 2) |

In every case, if a candidate newer than the target is held, one `Hold` names the newest such
candidate and its reasons. A held release is never installed to replace a blocked one (spec §4
failure 1: the page offers "Approve update" or "Uninstall"). A pinned blocked version is only
disabled. Versions installed from a local file are never updated automatically, but the block
list applies to them.

`plan_install(id, index, blocklist, now)` picks the newest release of an extension that is not
blocked and passes the age gate, or says why none can install (`NotInIndex`, `Blocked`,
`TooNew`).

## `fyx-registry` (CI tool)

```bash
fyx-registry sign release release.json --package todo-1.2.0.fyxext --key-file key.json --out signed.json
```

```bash
fyx-registry index add --index index.json --release signed.json --keyset keyset.json
```

```bash
fyx-registry verify index index.json --keyset keyset.json
```

```bash
fyx-registry sign blocklist blocklist-unsigned.json --out blocklist.json
```

```bash
fyx-registry sign snapshot --index index.json --sequence 914 --key-file snapshot-key.json --out snapshot.json
```

```bash
fyx-registry verify index index.json --keyset keyset.json --snapshot snapshot.json
```

```bash
fyx-registry verify blocklist blocklist.json --keyset keyset.json
```

- `sign release` takes the unsigned release (the `signed` part above), validates it, checks it
  against the package file (`--package`, required), and writes the envelope.
- `sign blocklist` takes the unsigned block list and writes the envelope.
- `sign snapshot` hashes the index, sets `generated_at` (now, or `--generated-at`) and
  `expires_at` (`--validity-hours`, 48 by default), adds `--revoke <key>`s, and writes the
  envelope.
- `verify release|index|blocklist|snapshot` checks signatures and contents; `verify release
  --package` also checks the package file, `verify index --snapshot` that the snapshot names this
  index. Any failure exits non-zero.
- `index add` verifies the whole existing index and the new release, refuses a version that is
  already there, keeps releases sorted by id and version, sets `generated_at` (now, or
  `--generated-at`), and replaces the file atomically. A missing index is created.
- `public-key --role <role>…` prints the keyset entry for the signing key.
- Files are replaced through a temporary file with a random name, created exclusively.

Without `--key-file`, the signing commands read `FYX_REGISTRY_SIGNING_KEY`.

## Schema changes

Every field is signed, so adding or changing a field changes the canonical bytes. Such a change
bumps the schema number and the domain tag (`.../v2`). A client refuses a schema it does not know
with `UnsupportedSchema` and keeps what it has; the registry keeps publishing the old schema
next to the new one (for example `index.json` and `index-v2.json`) for as long as clients that
read it are supported.
