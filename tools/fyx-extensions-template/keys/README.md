# Keys

`keyset.json` holds the registry's **public** keys only: six Ed25519 keys, one signing role each,
and `blocklist-revocation` on the offline snapshot rotation key (registry-repo.md, "Keys").

This template ships no `keyset.json`, because no real key exists yet. The owner creates the keys
by hand, on their own machine, as registry-repo.md "Creating the keys" says, and adds the printed
entries here through a PR. An agent never creates a real key and never sees a seed.

`pr.yml` runs `fyx-registry verify keyset keys/keyset.json --online-snapshot-key
"$SNAPSHOT_KEY_ID"` on every PR that touches this folder; `publish.yml` and `refresh.yml` run it
before they read a key.

`scripts/dry-run.sh` generates throwaway **test** keys in a temporary folder for its run and
deletes them afterwards. They are never written here.
