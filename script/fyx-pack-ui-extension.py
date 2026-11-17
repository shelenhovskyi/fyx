#!/usr/bin/env python3
"""Packs a UI extension folder into a `.fyxext` file (plan 1.1's package format).

A stopgap until `fyx-ext package` (plan 1.4): the zip holds `extension.toml`, the component its
`[fyx_ui]` table names, and `assets/` when there is one. `--version` writes another version into
the packaged `extension.toml` (to make an update of the same build for a manual check).

    python script/fyx-pack-ui-extension.py extensions/fyx_demo_counter --out D:/fyx-check
    python script/fyx-pack-ui-extension.py extensions/fyx_demo_counter --version 0.2.0 --out D:/fyx-check
"""

import argparse
import pathlib
import re
import sys
import tomllib
import zipfile


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("folder", type=pathlib.Path)
    parser.add_argument("--version", help="version to write into the packaged extension.toml")
    parser.add_argument("--out", type=pathlib.Path, default=pathlib.Path("."))
    args = parser.parse_args()

    manifest_path = args.folder / "extension.toml"
    manifest_text = manifest_path.read_text(encoding="utf-8")
    manifest = tomllib.loads(manifest_text)
    fyx_ui = manifest.get("fyx_ui")
    if fyx_ui is None:
        print(f"{manifest_path} has no [fyx_ui] table", file=sys.stderr)
        return 1
    component = args.folder / fyx_ui["component"]
    if not component.is_file():
        print(f"{component} is not built (script/fyx-build-demos builds the demos)", file=sys.stderr)
        return 1

    version = manifest["version"]
    if args.version:
        manifest_text, count = re.subn(
            r'^version\s*=\s*"[^"]*"', f'version = "{args.version}"', manifest_text, count=1, flags=re.M
        )
        if count != 1:
            print(f"no top-level version line in {manifest_path}", file=sys.stderr)
            return 1
        version = args.version

    args.out.mkdir(parents=True, exist_ok=True)
    package = args.out / f"{manifest['id']}-{version}.fyxext"
    with zipfile.ZipFile(package, "w", zipfile.ZIP_DEFLATED) as zip_file:
        zip_file.writestr("extension.toml", manifest_text)
        zip_file.write(component, fyx_ui["component"])
        assets = args.folder / "assets"
        if assets.is_dir():
            for path in sorted(assets.rglob("*")):
                if path.is_file():
                    zip_file.write(path, path.relative_to(args.folder).as_posix())
    print(package)
    return 0


if __name__ == "__main__":
    sys.exit(main())
