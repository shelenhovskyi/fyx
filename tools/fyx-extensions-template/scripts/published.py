#!/usr/bin/env python3
"""Download the published registry documents from the `index` release (integration-5 R6).

  published.py --repo OWNER/NAME --dir DIR [--first-run] NAME...

Each NAME (`index.json`, `snapshot.json`, `blocklist.json`) is downloaded into DIR. A failed
`gh` call always fails the job: a network or API error must never look like "nothing published
yet", or the next document would restart at sequence 1 and every client would refuse it as a
rollback. A document the release does not have is an error too, unless `--first-run` says this
is the run that publishes it for the first time; then it is left out of DIR and the job starts at
sequence 1. The missing names are printed one per line.

Standard library only; `gh` must be on PATH (or set `FYX_GH`).
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

RELEASE = "index"


class Failed(Exception):
    pass


def download(repo: str, names: list[str], out: Path, first_run: bool, run=subprocess.run) -> list[str]:
    """Downloads `names` into `out`; returns the ones the release lacks (only with `first_run`)."""
    gh = os.environ.get("FYX_GH", "gh")
    view = run(
        [gh, "release", "view", RELEASE, "--repo", repo, "--json", "assets"],
        capture_output=True,
        text=True,
    )
    if view.returncode == 0:
        try:
            assets = {asset["name"] for asset in json.loads(view.stdout)["assets"]}
        except (ValueError, KeyError, TypeError) as error:
            raise Failed(f"`gh release view {RELEASE}` printed no asset list: {error}") from error
    elif "release not found" in view.stderr.lower():
        assets = set()
    else:
        raise Failed(f"`gh release view {RELEASE}` failed: {view.stderr.strip()}")
    missing = [name for name in names if name not in assets]
    if missing and not first_run:
        raise Failed(
            f"the {RELEASE} release has no {', '.join(missing)}; if nothing was published yet, "
            "run this workflow by hand with first_run"
        )
    out.mkdir(parents=True, exist_ok=True)
    for name in names:
        if name in missing:
            continue
        fetched = run(
            [gh, "release", "download", RELEASE, "--repo", repo, "--pattern", name,
             "--dir", str(out), "--clobber"],
            capture_output=True,
            text=True,
        )
        if fetched.returncode != 0 or not (out / name).is_file():
            raise Failed(f"`gh release download {RELEASE} --pattern {name}` failed: {fetched.stderr.strip()}")
    return missing


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo", required=True)
    parser.add_argument("--dir", type=Path, required=True)
    # `--allow-missing`: the same, for a document that is optional in every run (refresh.yml's
    # block list, which blocklist.yml publishes on its own schedule).
    parser.add_argument("--first-run", "--allow-missing", dest="first_run", action="store_true")
    parser.add_argument("names", nargs="+")
    args = parser.parse_args(argv)
    try:
        missing = download(args.repo, args.names, args.dir, args.first_run)
    except Failed as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    for name in missing:
        print(f"::notice::first run: no {name} published yet", file=sys.stderr)
        print(name)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
