#!/usr/bin/env python3
"""Registry CI helpers for `shelenhovskyi/fyx-extensions` (docs/fyx/platform/registry-repo.md).

Standard library only (Python 3.11+). Every subcommand reads files and writes JSON or text to
stdout; none runs extension code, reads a signing key or talks to the network. Signing and
verifying are `fyx-registry`'s; this script prepares its inputs and makes the policy decision.

  entry FILE [--policy DIR] [--index INDEX]   validate an entry, print it as JSON
  release --entry E --manifest M --package P  the unsigned release (no published_at) as JSON
  stamp RELEASE --published-at SECONDS        add published_at
  package DIR --out FILE                      zip extension.toml, the component and assets/
  verdict [FILE] --inputs-sha256 HEX          a schema-checked agent verdict, else a concern
  decision-input --pr PR --before DIR --after DIR ...   assemble `decide`'s input from data
  decide INPUT                                the policy decision for one PR
  comment DECISION                            the PR comment for a decision, escaped
  blocklist TOML --sequence N --now T         the unsigned block list
  reblock PUBLISHED --now T                   the published list re-dated, sequence + 1
  revocations SNAPSHOT --keyset KEYSET        `--revoke` arguments to carry over
  expires-within FILE --hours H --now T       exit 1 if a signed document expires that soon
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import tomllib
import io
import zipfile
from pathlib import Path

POLICY_DIR = Path(__file__).resolve().parent.parent / "policy"

ID_RE = re.compile(r"^[a-z][a-z0-9-]{0,63}$")
LOGIN_RE = re.compile(r"^[A-Za-z0-9](?:[A-Za-z0-9]|-(?=[A-Za-z0-9])){0,38}$")
REPOSITORY_RE = re.compile(r"^https://github\.com/[A-Za-z0-9-]+/[A-Za-z0-9._-]+$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
VERSION_RE = re.compile(r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
ENTRY_KEYS = {"id", "owners", "source", "urgent"}
SOURCE_KEYS = {"repository", "commit", "path"}
TIERS = ["t0", "t1", "t2"]
VERDICT_CATEGORIES = {
    "capability-mismatch",
    "obfuscation",
    "network",
    "process",
    "assets",
    "prompt-injection",
    "other",
}
MAX_REVIEW_INPUT_BYTES = 400 * 1024
BLOCKLIST_VALIDITY_HOURS = 48
NO_REVIEWER = "no reviewer configured"


class Refused(Exception):
    """An input the registry refuses; the message is shown to the PR author."""


# ---------------------------------------------------------------------------------------------
# Policy files


def load_policy(policy_dir: Path) -> dict:
    registry = tomllib.loads((policy_dir / "registry.toml").read_text(encoding="utf-8"))
    tiers = json.loads((policy_dir / "capability-tiers.json").read_text(encoding="utf-8"))
    reserved = []
    for line in (policy_dir / "reserved-ids.txt").read_text(encoding="utf-8").splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            reserved.append(line)
    return {
        "owner": registry["owner"],
        "download_base": registry["download_base"].rstrip("/"),
        "tiers": tiers["capabilities"],
        "reserved": reserved,
    }


def is_reserved(extension_id: str, reserved: list[str]) -> bool:
    for pattern in reserved:
        if pattern.endswith("*"):
            if extension_id.startswith(pattern[:-1]):
                return True
        elif extension_id == pattern:
            return True
    return False


def edit_distance_at_most_one(a: str, b: str) -> bool:
    if a == b:
        return True
    if abs(len(a) - len(b)) > 1:
        return False
    if len(a) == len(b):
        return sum(x != y for x, y in zip(a, b)) == 1
    if len(a) > len(b):
        a, b = b, a
    # b is one longer: skipping one character of b gives a.
    return any(b[:i] + b[i + 1 :] == a for i in range(len(b)))


def near_copy_of(extension_id: str, existing: list[str]) -> str | None:
    """An existing id this one imitates: edit distance 1, or equal once `-` is removed."""
    for other in existing:
        if other == extension_id:
            continue
        if edit_distance_at_most_one(extension_id, other) or (
            extension_id.replace("-", "") == other.replace("-", "")
        ):
            return other
    return None


# ---------------------------------------------------------------------------------------------
# Entries and releases


def parse_entry(text: str, file_stem: str | None, policy: dict, existing: list[str]) -> dict:
    try:
        entry = tomllib.loads(text)
    except tomllib.TOMLDecodeError as error:
        raise Refused(f"the entry is not TOML: {error}") from error
    unknown = set(entry) - ENTRY_KEYS
    if unknown:
        raise Refused(f"unknown keys in the entry: {', '.join(sorted(unknown))}")
    extension_id = entry.get("id")
    if not isinstance(extension_id, str) or not ID_RE.match(extension_id):
        raise Refused(
            "id must be a-z, 0-9 and -, start with a letter, and have at most 64 characters"
        )
    if file_stem is not None and file_stem != extension_id:
        raise Refused(f"the file must be named extensions/{extension_id}.toml")
    owners = entry.get("owners")
    if (
        not isinstance(owners, list)
        or not 1 <= len(owners) <= 8
        or not all(isinstance(owner, str) and LOGIN_RE.match(owner) for owner in owners)
        or len(set(owners)) != len(owners)
    ):
        raise Refused("owners must be 1 to 8 distinct GitHub logins")
    # Reserved ids (fyx-*, …) are the registry owner's own: only an entry owned by the owner
    # alone may use one, and the policy check then asks for the owner's approval.
    if is_reserved(extension_id, policy["reserved"]) and sorted(owners) != [policy["owner"]]:
        raise Refused(f"the id {extension_id} is reserved")
    copy = near_copy_of(extension_id, existing)
    if copy is not None:
        raise Refused(f"the id {extension_id} is too close to the existing {copy}")
    source = entry.get("source")
    if not isinstance(source, dict):
        raise Refused("the entry needs a [source] table")
    unknown = set(source) - SOURCE_KEYS
    if unknown:
        raise Refused(f"unknown keys in [source]: {', '.join(sorted(unknown))}")
    repository = source.get("repository")
    if not isinstance(repository, str) or not REPOSITORY_RE.match(repository):
        raise Refused("source.repository must be https://github.com/<owner>/<repo>")
    commit = source.get("commit")
    if not isinstance(commit, str) or not COMMIT_RE.match(commit):
        raise Refused("source.commit must be a full 40-digit lowercase SHA, not a branch or tag")
    path = source.get("path", ".")
    if (
        not isinstance(path, str)
        or path.startswith("/")
        or "\\" in path
        or ":" in path
        or any(part in ("..", "") for part in path.split("/"))
    ):
        raise Refused("source.path must be a relative path without ..")
    urgent = entry.get("urgent", False)
    if not isinstance(urgent, bool):
        raise Refused("urgent must be true or false")
    return {
        "id": extension_id,
        "owners": sorted(owners),
        "source": {"repository": repository, "commit": commit, "path": path},
        "urgent": urgent,
    }


def tier_of(capability: str, tiers: dict) -> str:
    # A capability this registry does not know is never granted silently: it counts as T2.
    return tiers.get(capability, "t2")


# A Zed extension inside a Fyx package gets upstream's `fetch` and `process:exec` with none of
# Fyx's domain or program checks. The registry accepts it, but such a release is always tier T2
# (the owner approves every version, no auto-merge), records `zed_extension: true` (signed;
# `fyx_registry::Release`), and is flagged as running an unrestricted Zed extension (0.3 design
# Q1, decided by the owner 2026-11-19).
ZED_EXTENSION_COMPONENT = "extension.wasm"
ZED_EXTENSION_NOTICE = (
    "runs an unrestricted Zed extension (extension.wasm): its network and process access "
    "bypasses Fyx's permission checks"
)


def is_zed_extension_name(name: str) -> bool:
    """`extension.wasm` at any depth, in any letter case, with Windows' trailing dots and spaces
    ignored, as `fyx_registry::runs_zed_extension` matches it."""
    return re.split(r"[/\\]", name)[-1].rstrip(". ").lower() == ZED_EXTENSION_COMPONENT


def zed_extension_files(package: bytes = b"", source_files: list[str] | tuple = ()) -> list[str]:
    """The `extension.wasm` files in the package and in the entry's source file list."""
    names = list(source_files)
    if package and zipfile.is_zipfile(io.BytesIO(package)):
        # Only `fyx-ext package` output reaches here in CI; the client checks the format.
        names += zipfile.ZipFile(io.BytesIO(package)).namelist()
    return sorted({name for name in names if is_zed_extension_name(name)})


def runs_zed_extension(package: bytes) -> bool:
    """Whether the package has a Zed `extension.wasm` (as `fyx_registry::runs_zed_extension`)."""
    return bool(zed_extension_files(package))


def derive_release(
    entry: dict,
    manifest_text: str,
    package: bytes,
    policy: dict,
    source_files: list[str] | tuple = (),
) -> dict:
    """The `signed` part of a release, without `published_at` (registry-repo.md, the entry).
    A release that carries a Zed extension is tier T2 whatever its capabilities say."""
    manifest = tomllib.loads(manifest_text)
    if manifest.get("id") != entry["id"]:
        raise Refused(
            f"extension.toml has id {manifest.get('id')!r}, the entry says {entry['id']!r}"
        )
    version = manifest.get("version")
    if not isinstance(version, str) or not VERSION_RE.match(version):
        raise Refused("extension.toml's version must be major.minor.patch")
    fyx_ui = manifest.get("fyx_ui") or {}
    capabilities = sorted(set(fyx_ui.get("capabilities", [])))
    tier = max((tier_of(c, policy["tiers"]) for c in capabilities), default="t0")
    if tier == "t2":
        # network_domains and processes come with T2 (decisions 23, 24); until fyx-ext reads
        # them, a T2 entry cannot be described completely.
        raise Refused("T2 capabilities are not accepted yet: " + ", ".join(capabilities))
    name = entry["id"] + "-" + version
    # A package that also runs a Zed extension is T2 whatever it declares, so it always needs
    # the owner's approval and never auto-merges (owner's Q1, 2026-11-19).
    zed = bool(zed_extension_files(package, source_files))
    if zed:
        tier = "t2"
    release = {
        "id": entry["id"],
        "version": version,
        "name": manifest.get("name", ""),
        "description": manifest.get("description", ""),
        "owners": entry["owners"],
        "source": entry["source"],
        "package": {
            "url": f"{policy['download_base']}/{entry['id']}-v{version}/{name}.fyxext",
            "sha256": hashlib.sha256(package).hexdigest(),
            "size": len(package),
        },
        "urgent": entry["urgent"],
        "tier": tier,
        "capabilities": capabilities,
    }
    # Signed only when true: fyx_registry's Release skips `zed_extension` when false.
    if zed:
        release["zed_extension"] = True
    return release


def package_dir(directory: Path, out: Path) -> None:
    """The `.fyxext` layout of `fyx-ext package`: extension.toml, the component, assets/."""
    manifest = tomllib.loads((directory / "extension.toml").read_text(encoding="utf-8"))
    component = manifest["fyx_ui"]["component"].replace("\\", "/")
    files = [("extension.toml", directory / "extension.toml"), (component, directory / component)]
    assets = directory / "assets"
    if assets.is_dir():
        for path in sorted(assets.rglob("*")):
            if path.is_file():
                files.append((path.relative_to(directory).as_posix(), path))
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as archive:
        for name, path in files:
            # A fixed timestamp keeps the package byte-identical between builds.
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, path.read_bytes())


# ---------------------------------------------------------------------------------------------
# The agent verdict


def checked_verdict(raw: str | None, inputs_sha256: str, inputs_size: int = 0) -> dict:
    """The verdict as the report uses it. Anything but a valid `pass` is a `concern`."""

    def concern(detail: str, category: str = "other") -> dict:
        return {
            "verdict": "concern",
            "reasons": [{"category": category, "file": "", "detail": detail}],
            "inputs_sha256": inputs_sha256,
        }

    if inputs_size > MAX_REVIEW_INPUT_BYTES:
        return concern("too large for automated review")
    if raw is None:
        return concern(NO_REVIEWER)
    try:
        verdict = json.loads(raw)
    except json.JSONDecodeError:
        return concern("the reviewer's verdict is not JSON")
    if not isinstance(verdict, dict) or set(verdict) != {"verdict", "reasons", "inputs_sha256"}:
        return concern("the reviewer's verdict does not match the schema")
    if verdict["verdict"] not in ("pass", "concern"):
        return concern("the reviewer's verdict does not match the schema")
    if verdict["inputs_sha256"] != inputs_sha256:
        return concern("the verdict is for other inputs")
    reasons = verdict["reasons"]
    if not isinstance(reasons, list) or len(reasons) > 20:
        return concern("the reviewer's verdict does not match the schema")
    for reason in reasons:
        if (
            not isinstance(reason, dict)
            or set(reason) != {"category", "file", "detail"}
            or reason["category"] not in VERDICT_CATEGORIES
            or not isinstance(reason["file"], str)
            or not isinstance(reason["detail"], str)
            or len(reason["detail"]) > 500
        ):
            return concern("the reviewer's verdict does not match the schema")
    return verdict


# ---------------------------------------------------------------------------------------------
# The policy decision (registry-repo.md, "The policy check")


def decide(data: dict) -> dict:
    owner = data["registry_owner"]
    author = data["pr_author"]
    approvals = set(data.get("approvals", []))
    labels = set(data.get("labels", []))
    before = data.get("entry_before")
    after = data.get("entry_after")
    release = data.get("release")
    last = data.get("last_published")
    entry_files = [f for f in data.get("changed_files", []) if f.startswith("extensions/")]
    other_files = [f for f in data.get("changed_files", []) if not f.startswith("extensions/")]
    failures: list[str] = []
    required: list[dict] = []
    changes: dict = {}

    def approved_by_owner() -> bool:
        return author == owner or owner in approvals

    if not entry_files:
        # Not an entry PR: what it touches is under CODEOWNERS, so GitHub asks the owner.
        return {
            "kind": "other",
            "status": "success",
            "failures": [],
            "required": [],
            "tier": None,
            "changes": {},
            "auto_merge": False,
            "agent_review": "success",
            "add_labels": [],
        }

    kind = "new-version" if before else "new-extension"
    if len(entry_files) > 1:
        failures.append("a PR may change one entry only")
    if other_files:
        failures.append("an entry PR changes nothing but its entry")
    if after is None:
        failures.append("entries are never deleted; block the extension instead")
    for error in data.get("entry_errors", []):
        failures.append(error)
    for step, ok in data.get("ci", {}).items():
        if not ok:
            failures.append(f"CI step {step} failed")
    if after and after["id"] in data.get("blocked_ids", []):
        failures.append(f"{after['id']} is blocked for every version")
    if release is None and after is not None and not data.get("entry_errors"):
        failures.append("no release was derived from the build")

    tier = release["tier"] if release else None
    if release and last:
        if version_key(release["version"]) <= version_key(last["version"]):
            failures.append(
                f"version {release['version']} is not above the published {last['version']}"
            )
        changes = {
            "capabilities_added": sorted(set(release["capabilities"]) - set(last["capabilities"])),
            "capabilities_removed": sorted(
                set(last["capabilities"]) - set(release["capabilities"])
            ),
            "network_domains_added": sorted(
                set(release.get("network_domains", [])) - set(last.get("network_domains", []))
            ),
            "processes_added": sorted(
                set(release.get("processes", [])) - set(last.get("processes", []))
            ),
            "tier": [last["tier"], release["tier"]],
        }
    elif release:
        changes = {"capabilities_added": release["capabilities"], "tier": [None, release["tier"]]}

    if before and after:
        added = sorted(set(after["owners"]) - set(before["owners"]))
        removed = sorted(set(before["owners"]) - set(after["owners"]))
        if added or removed:
            changes["owners_added"] = added
            changes["owners_removed"] = removed
            existing = set(before["owners"])
            required.append(
                {
                    "who": "an existing owner of the entry",
                    "reason": "owners change",
                    "satisfied": author in existing or bool(existing & approvals),
                }
            )
        if tier in ("t0", "t1"):
            entry_owners = set(before["owners"])
            required.append(
                {
                    "who": "an owner of the entry",
                    "reason": "a new version",
                    "satisfied": author in entry_owners or bool(entry_owners & approvals),
                }
            )
    if tier == "t2":
        reason = "a T2 release"
        if data.get("zed_extension"):
            reason = "a T2 release: it " + ZED_EXTENSION_NOTICE
        required.append({"who": owner, "reason": reason, "satisfied": approved_by_owner()})
    if after and is_reserved(after["id"], data.get("reserved_ids", [])):
        required.append(
            {"who": owner, "reason": "a reserved id", "satisfied": approved_by_owner()}
        )
    if after and after.get("urgent"):
        required.append(
            {"who": owner, "reason": "urgent skips the age gate", "satisfied": approved_by_owner()}
        )

    verdict = data.get("agent_verdict") or checked_verdict(None, "")
    add_labels = []
    if verdict["verdict"] != "pass":
        add_labels.append("needs-owner")
    if verdict["verdict"] != "pass" or "needs-owner" in labels:
        required.append(
            {
                "who": owner,
                "reason": "the agent review raised a concern (needs-owner)",
                "satisfied": approved_by_owner(),
            }
        )

    if failures:
        status = "failure"
    elif all(requirement["satisfied"] for requirement in required):
        status = "success"
    else:
        status = "pending"
    agent_ok = (verdict["verdict"] == "pass" and "needs-owner" not in labels) or (
        approved_by_owner()
    )
    return {
        "kind": kind,
        "status": status,
        "failures": failures,
        "required": required,
        "tier": tier,
        "changes": changes,
        "auto_merge": status == "success"
        and tier in ("t0", "t1")
        and after is not None
        and not after.get("urgent"),
        "agent_review": "success" if agent_ok else "failure",
        "verdict": verdict,
        "add_labels": add_labels,
        "zed_extension": data.get("zed_extension", []),
    }


def decision_input(
    pr: dict,
    jobs: dict | None,
    before_dir: Path,
    after_dir: Path,
    manifest_text: str | None,
    package: bytes | None,
    index: dict | None,
    blocklist: dict | None,
    verdict: dict | None,
    policy: dict,
    source_files: list[str] | tuple = (),
) -> dict:
    """`decide`'s input, from what the trusted report job fetched itself: the PR from the API,
    the entry before (base) and after (head) as files, extension.toml at the source commit, and
    the published index and block list. Nothing here comes from running PR code."""
    changed = [file["path"] for file in pr.get("files", [])]
    entry_files = [f for f in changed if re.match(r"^extensions/[^/]+\.toml$", f)]
    releases = [r["signed"] for r in (index or {}).get("releases", [])]
    existing = sorted({r["id"] for r in releases})
    data = {
        "registry_owner": policy["owner"],
        "pr_author": pr["author"]["login"],
        "approvals": approvals(pr.get("reviews", [])),
        "labels": [label["name"] for label in pr.get("labels", [])],
        "changed_files": changed,
        "reserved_ids": policy["reserved"],
        "blocked_ids": sorted(
            {e["id"] for e in (blocklist or {}).get("signed", {}).get("entries", [])
             if e["versions"] == "*"}
        ),
        "entry_before": None,
        "entry_after": None,
        "release": None,
        "last_published": None,
        "entry_errors": [],
        "ci": ci_results(jobs),
        "agent_verdict": verdict,
        "zed_extension": zed_extension_files(package or b"", source_files),
    }
    if len(entry_files) != 1:
        return data
    name = entry_files[0]
    before_file, after_file = before_dir / name, after_dir / name
    if before_file.exists():
        # The published entry is valid by construction; parse it without the id rules, which
        # may have changed since.
        data["entry_before"] = tomllib.loads(before_file.read_text(encoding="utf-8"))
        data["entry_before"]["owners"] = sorted(data["entry_before"]["owners"])
    if not after_file.exists():
        return data
    try:
        after = parse_entry(
            after_file.read_text(encoding="utf-8"), Path(name).stem, policy, existing
        )
    except Refused as error:
        data["entry_errors"].append(str(error))
        return data
    data["entry_after"] = after
    published = [r for r in releases if r["id"] == after["id"]]
    if published:
        data["last_published"] = max(published, key=lambda r: version_key(r["version"]))
    if manifest_text is None:
        data["entry_errors"].append("extension.toml could not be read at source.commit")
        return data
    try:
        data["release"] = derive_release(
            after, manifest_text, package or b"", policy, source_files
        )
    except Refused as error:
        data["entry_errors"].append(str(error))
    return data


def approvals(reviews: list[dict]) -> list[str]:
    """Logins whose latest decisive review approves (a later comment does not undo it)."""
    latest: dict[str, str] = {}
    for review in reviews:
        state = review.get("state")
        if state in ("APPROVED", "CHANGES_REQUESTED", "DISMISSED"):
            latest[review["author"]["login"]] = state
    return sorted(login for login, state in latest.items() if state == "APPROVED")


def ci_results(jobs: dict | None) -> dict:
    """pr.yml's `entry` and `build` jobs (matrix jobs are named `entry (…)`): all must succeed."""
    results = {"entry": False, "build": False}
    if not jobs:
        return results
    for step in results:
        mine = [
            job for job in jobs.get("jobs", [])
            if job["name"] == step or job["name"].startswith(step + " (")
        ]
        results[step] = bool(mine) and all(job.get("conclusion") == "success" for job in mine)
    return results


def version_key(version: str) -> tuple[int, int, int]:
    major, minor, patch = (int(part) for part in version.split("."))
    return (major, minor, patch)


def escape(text: str) -> str:
    """Plain text for a PR comment: no mention, no link, no HTML, no Markdown structure."""
    zero_width = "\u200b"
    text = text.replace("\r", " ").replace("\n", " ")
    text = text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
    text = re.sub(r"[\\`*_{}\[\]()#+!|~]", lambda m: "\\" + m.group(0), text)
    text = text.replace("@", "@" + zero_width)
    text = re.sub(r"(?i)\b(https?|ftp|file|data|javascript):", r"\1" + zero_width + ":", text)
    text = text.replace("www.", "www" + zero_width + ".")
    return text


def comment(decision: dict) -> str:
    lines = ["<!-- fyx-registry-policy -->", "### Registry policy", ""]
    if decision["kind"] == "other":
        lines.append("Not an entry change: the owner reviews it through Code Owners.")
        return "\n".join(lines) + "\n"
    lines.append(f"- Change: {decision['kind']}, tier {decision['tier'] or 'unknown'}")
    if decision.get("zed_extension"):
        files = ", ".join(escape(name) for name in decision["zed_extension"])
        lines.append(f"- **Warning:** this extension {ZED_EXTENSION_NOTICE}. Files: {files}")
    lines.append(f"- Status: **{decision['status']}**")
    for failure in decision["failures"]:
        lines.append(f"- Refused: {escape(failure)}")
    for key, value in decision["changes"].items():
        lines.append(f"- {key.replace('_', ' ')}: {escape(json.dumps(value))}")
    for requirement in decision["required"]:
        mark = "done" if requirement["satisfied"] else "waiting"
        lines.append(
            f"- Needs {escape(requirement['who'])} ({escape(requirement['reason'])}): {mark}"
        )
    if decision["auto_merge"]:
        lines.append("- Auto-merge: on")
    verdict = decision.get("verdict") or {}
    lines.append(f"- Agent review: {escape(str(verdict.get('verdict', 'concern')))}")
    for reason in verdict.get("reasons", []):
        lines.append(
            f"  - {escape(reason['category'])} {escape(reason['file'])}: "
            f"{escape(reason['detail'])}"
        )
    return "\n".join(lines) + "\n"


# ---------------------------------------------------------------------------------------------
# Block list and snapshots


def unsigned_blocklist(text: str, sequence: int, now: int) -> dict:
    source = tomllib.loads(text)
    unknown = set(source) - {"entry"}
    if unknown:
        raise Refused(f"unknown keys in blocklist.toml: {', '.join(sorted(unknown))}")
    entries = []
    for entry in source.get("entry", []):
        unknown = set(entry) - {"id", "versions", "reason", "urgent", "package_sha256"}
        if unknown:
            raise Refused(f"unknown keys in a block entry: {', '.join(sorted(unknown))}")
        converted = {
            "id": entry["id"],
            "versions": entry["versions"],
            "reason": entry["reason"],
            "urgent": bool(entry.get("urgent", False)),
        }
        if "package_sha256" in entry:
            converted["package_sha256"] = entry["package_sha256"]
        entries.append(converted)
    return {
        "schema": 1,
        "sequence": sequence,
        "generated_at": now,
        "expires_at": now + BLOCKLIST_VALIDITY_HOURS * 3600,
        "entries": entries,
    }


def carried_revocations(snapshot: dict, keyset: dict) -> list[str]:
    """The published snapshot's revocations of release and snapshot keys. Block-list keys and the
    rotation key are left out: only the offline rotation key may revoke them, and clients keep
    those revocations by themselves (registry-format.md, `snapshot.json`)."""
    roles = {key["key_id"]: set(key["roles"]) for key in keyset["keys"]}
    carried = []
    for key_id in snapshot["signed"].get("revoked_keys", []):
        key_roles = roles.get(key_id, set())
        if "blocklist" in key_roles or "blocklist-revocation" in key_roles:
            continue
        carried.append(key_id)
    return sorted(carried)


# ---------------------------------------------------------------------------------------------
# Command line


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    commands = parser.add_subparsers(dest="command", required=True)

    entry = commands.add_parser("entry")
    entry.add_argument("file", type=Path)
    entry.add_argument("--policy", type=Path, default=POLICY_DIR)
    entry.add_argument("--index", type=Path, help="the published index.json, for near-copy ids")

    release = commands.add_parser("release")
    release.add_argument("--entry", type=Path, required=True)
    release.add_argument("--manifest", type=Path, required=True)
    release.add_argument("--package", type=Path, required=True)
    release.add_argument("--policy", type=Path, default=POLICY_DIR)

    stamp = commands.add_parser("stamp")
    stamp.add_argument("release", type=Path)
    stamp.add_argument("--published-at", type=int, required=True)

    package = commands.add_parser("package")
    package.add_argument("dir", type=Path)
    package.add_argument("--out", type=Path, required=True)

    verdict = commands.add_parser("verdict")
    verdict.add_argument("file", type=Path, nargs="?")
    verdict.add_argument("--inputs-sha256", required=True)
    verdict.add_argument("--inputs-size", type=int, default=0)

    assemble = commands.add_parser("decision-input")
    assemble.add_argument("--pr", type=Path, required=True)
    assemble.add_argument("--jobs", type=Path)
    assemble.add_argument("--before", type=Path, required=True)
    assemble.add_argument(
        "--source-files", type=Path, help="the entry's source file list, one path per line"
    )
    assemble.add_argument("--after", type=Path, required=True)
    assemble.add_argument("--manifest", type=Path)
    assemble.add_argument("--package", type=Path)
    assemble.add_argument("--index", type=Path)
    assemble.add_argument("--blocklist", type=Path)
    assemble.add_argument("--verdict", type=Path)
    assemble.add_argument("--policy", type=Path, default=POLICY_DIR)

    decide_parser = commands.add_parser("decide")
    decide_parser.add_argument("input", type=Path)

    comment_parser = commands.add_parser("comment")
    comment_parser.add_argument("decision", type=Path)

    blocklist = commands.add_parser("blocklist")
    blocklist.add_argument("toml", type=Path)
    blocklist.add_argument("--sequence", type=int, required=True)
    blocklist.add_argument("--now", type=int, required=True)

    reblock = commands.add_parser("reblock")
    reblock.add_argument("published", type=Path)
    reblock.add_argument("--now", type=int, required=True)

    revocations = commands.add_parser("revocations")
    revocations.add_argument("snapshot", type=Path)
    revocations.add_argument("--keyset", type=Path, required=True)

    expires = commands.add_parser("expires-within")
    expires.add_argument("file", type=Path)
    expires.add_argument("--hours", type=int, required=True)
    expires.add_argument("--now", type=int, required=True)

    args = parser.parse_args(argv)
    out = sys.stdout
    try:
        if args.command == "entry":
            policy = load_policy(args.policy)
            existing = []
            if args.index and args.index.exists():
                index = json.loads(args.index.read_text(encoding="utf-8"))
                existing = sorted({r["signed"]["id"] for r in index.get("releases", [])})
            parsed = parse_entry(
                args.file.read_text(encoding="utf-8"), args.file.stem, policy, existing
            )
            json.dump(parsed, out, indent=2)
        elif args.command == "release":
            policy = load_policy(args.policy)
            parsed = json.loads(args.entry.read_text(encoding="utf-8"))
            json.dump(
                derive_release(
                    parsed,
                    args.manifest.read_text(encoding="utf-8"),
                    args.package.read_bytes(),
                    policy,
                ),
                out,
                indent=2,
            )
        elif args.command == "stamp":
            stamped = json.loads(args.release.read_text(encoding="utf-8"))
            stamped["published_at"] = args.published_at
            json.dump(stamped, out, indent=2)
        elif args.command == "package":
            package_dir(args.dir, args.out)
        elif args.command == "verdict":
            raw = args.file.read_text(encoding="utf-8") if args.file else None
            json.dump(checked_verdict(raw, args.inputs_sha256, args.inputs_size), out, indent=2)
        elif args.command == "decision-input":

            def optional_json(path: Path | None) -> dict | None:
                if path is None or not path.exists():
                    return None
                return json.loads(path.read_text(encoding="utf-8"))

            manifest = args.manifest.read_text(encoding="utf-8") if (
                args.manifest and args.manifest.exists()
            ) else None
            package_bytes = args.package.read_bytes() if (
                args.package and args.package.exists()
            ) else None
            json.dump(
                decision_input(
                    optional_json(args.pr),
                    optional_json(args.jobs),
                    args.before,
                    args.after,
                    manifest,
                    package_bytes,
                    optional_json(args.index),
                    optional_json(args.blocklist),
                    optional_json(args.verdict),
                    load_policy(args.policy),
                    args.source_files.read_text(encoding="utf-8").splitlines()
                    if args.source_files and args.source_files.exists()
                    else (),
                ),
                out,
                indent=2,
            )
        elif args.command == "decide":
            json.dump(decide(json.loads(args.input.read_text(encoding="utf-8"))), out, indent=2)
        elif args.command == "comment":
            out.write(comment(json.loads(args.decision.read_text(encoding="utf-8"))))
        elif args.command == "blocklist":
            text = args.toml.read_text(encoding="utf-8")
            json.dump(unsigned_blocklist(text, args.sequence, args.now), out, indent=2)
        elif args.command == "reblock":
            published = json.loads(args.published.read_text(encoding="utf-8"))["signed"]
            published["sequence"] += 1
            published["generated_at"] = args.now
            published["expires_at"] = args.now + BLOCKLIST_VALIDITY_HOURS * 3600
            json.dump(published, out, indent=2)
        elif args.command == "revocations":
            snapshot = json.loads(args.snapshot.read_text(encoding="utf-8"))
            keyset = json.loads(args.keyset.read_text(encoding="utf-8"))
            for key_id in carried_revocations(snapshot, keyset):
                out.write(f"--revoke\n{key_id}\n")
        elif args.command == "expires-within":
            document = json.loads(args.file.read_text(encoding="utf-8"))
            expires_at = document["signed"]["expires_at"]
            if expires_at - args.now < args.hours * 3600:
                print(f"{args.file} expires at {expires_at}, within {args.hours} h")
                return 1
            print(f"{args.file} expires at {expires_at}")
    except Refused as error:
        print(f"refused: {error}", file=sys.stderr)
        return 2
    if args.command not in ("comment", "package", "revocations", "expires-within"):
        out.write("\n")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
