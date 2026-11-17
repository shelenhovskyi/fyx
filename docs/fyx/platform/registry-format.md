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

A **keyset** lists the public keys a reader accepts. The client compiles in the registry key and
one rotation key (decision 18); the CI tool reads a keyset file:

```json
{"keys": [
  {"key_id": "fyx-2026-a", "ed25519": "<base64 of 32 bytes>"},
  {"key_id": "fyx-2026-b", "ed25519": "<base64 of 32 bytes>"}
]}
```

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
`--key-file` or from the `FYX_REGISTRY_SIGNING_KEY` environment variable (the same JSON), and
`fyx-registry public-key` prints the matching keyset entry.

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

The first field is a domain tag (`fyx-registry/release/v1`, `fyx-registry/blocklist/v1`), so a
signature over a release can never pass as one over a block list. Sets (owners, capabilities,
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
client uses every release that verifies even if another entry is damaged (the damaged ones are
reported as rejected). The index as a whole is not signed.

Release fields, in canonical order:

| Field | Rules |
|---|---|
| `id` | extension id |
| `version` | `major.minor.patch` |
| `name` | 1–64 characters, no control characters, no surrounding whitespace |
| `description` | 0–300 characters, same rules |
| `owners` | at least one GitHub login (`A-Z a-z 0-9 -`, 1–39, no leading or trailing `-`); shown as the publisher |
| `source.repository`, `.commit`, `.path` | `https://` URL; full 40-digit lowercase SHA; relative path without `..` (`.` for the root) |
| `package.url`, `.sha256`, `.size` | `https://` URL; hash and size of the `.fyxext`; size 1 byte to 50 MiB (decision 12) |
| `published_at` | when CI published it; the age gate counts from here |
| `urgent` | security fix: skips the age gate |
| `tier` | `t0`, `t1` or `t2`; at least the highest tier among the known capabilities |
| `capabilities` | names like `ui:panel`; ones this client does not know are allowed but never granted silently |
| `network_domains` | optional; `example.com` or `*.example.com`; needs `network:fetch` |
| `processes` | optional; program names without paths (`A-Z a-z 0-9 . _ + -`, up to 64); needs `process:spawn` |

A version listed twice is rejected in every copy. `fyx-registry index add` refuses to add a
version that is already published: a published release is never replaced, only blocked.

A downloaded package is checked against `package.size` and `package.sha256` before it is
unpacked (`Release::verify_package`).

## `blocklist.json`

```json
{
  "signed": {
    "schema": 1,
    "sequence": 42,
    "generated_at": 1763400000,
    "entries": [
      {"id": "todo", "versions": ">=1.2.0, <1.2.3", "reason": "Uploads open buffers.", "urgent": true}
    ]
  },
  "signature": {"key_id": "fyx-2026-a", "ed25519": "<base64>"}
}
```

- `sequence` increases with every published list.
- `versions` is `*` (every version) or comparators joined by `,` that must all hold. A comparator
  is `=`, `>`, `>=`, `<` or `<=` followed by `major.minor.patch`. There is no `^` or `~`, and
  pre-release versions compare by plain semver order, so `>=1.2.0` also covers `1.3.0-beta`
  (a file installed from disk may carry one).
- `reason`: 1–300 characters without control characters, shown to the user.

Canonical order: schema, sequence, generated_at, then for each entry: id, the comparators (each
operator and version as strings), reason, urgent.

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
| same `sequence`, same content | `fetched_at` updated (`Refresh`), never moved backwards |
| same `sequence`, different content | kept, reported as a conflict |
| higher `sequence`, or no cache yet | replaced (`Replace`) and applied |

Fetches run at start and every 6 h (`fetch_due`). `freshness` reports `Missing` (never fetched:
nothing is blocked yet), `Fresh`, or `Stale` once the last successful fetch is 7 days old, for
the "Block list is N days old" warning (spec §4 failure 5). Being offline never disables an
extension.

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
fyx-registry verify blocklist blocklist.json --keyset keyset.json
```

- `sign release` takes the unsigned release (the `signed` part above), validates it, checks it
  against the package file when `--package` is given, and writes the envelope.
- `sign blocklist` takes the unsigned block list and writes the envelope.
- `verify release|index|blocklist` checks signatures and contents; `verify release --package`
  also checks the package file. Any failure exits non-zero.
- `index add` verifies the whole existing index and the new release, refuses a version that is
  already there, keeps releases sorted by id and version, sets `generated_at` (now, or
  `--generated-at`), and replaces the file atomically. A missing index is created.
- `public-key` prints the keyset entry for the signing key.

Without `--key-file`, the signing commands read `FYX_REGISTRY_SIGNING_KEY`.

## Schema changes

Every field is signed, so adding or changing a field changes the canonical bytes. Such a change
bumps the schema number and the domain tag (`.../v2`). A client refuses a schema it does not know
with `UnsupportedSchema` and keeps what it has; the registry keeps publishing the old schema
next to the new one (for example `index.json` and `index-v2.json`) for as long as clients that
read it are supported.
