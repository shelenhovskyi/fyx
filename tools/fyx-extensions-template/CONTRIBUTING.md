# Adding or updating an extension

An extension is one file, `extensions/<id>.toml`: where its source is and who owns it. The
registry's CI builds the package from that source; you never upload a binary.

```toml
# extensions/todo.toml
id     = "todo"                                   # equals the file name and extension.toml's id
owners = ["alice", "bob"]                         # GitHub logins, 1 to 8

[source]
repository = "https://github.com/alice/todo"
commit     = "0123456789abcdef0123456789abcdef01234567"   # a full commit SHA on a branch
path       = "."                                  # the extension's folder in that repository
```

Rules (checked by CI):

- The id is `a-z`, `0-9` and `-`, starts with a letter, at most 64 characters, is not reserved
  (`policy/reserved-ids.txt`) and is not one edit away from an existing id.
- The commit is a full 40-digit lowercase SHA that is on one of the repository's branches, and
  the repository has a `Cargo.lock`.
- `extension.toml` at that commit has the same id and a `major.minor.patch` version.

**Adding:** open a PR with the new file. **Updating:** bump `version` in `extension.toml`, push
the commit, and open a PR that changes `source.commit`. A published version is never replaced.

Who has to approve:

| Change | Approval |
|---|---|
| New extension, tier T0 or T1 | nobody: it merges when CI is green and the review passes |
| New version, T0 or T1 | one of the entry's owners (as the PR's author, or by approving) |
| Anything T2 (network, processes, editing files, observing agents) | the registry owner, every version |
| `urgent = true` (skips the 24 h wait before installs) | the registry owner |
| Owners added or removed | an existing owner of the entry |
| A review concern (`needs-owner`) | the registry owner |

An entry is never deleted. To withdraw a version, report it; the owner blocks it.

Every PR gets a comment with its tier, what changed since the last release, and who has to
approve. Text in your PR that addresses a reviewer or an AI is itself a review concern.
