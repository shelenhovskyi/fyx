#!/usr/bin/env python3
"""Prototype: one overview of every git worktree of a repo, which agent works in it, and which are
forgotten. Read-only: it never removes or changes anything, it only prints hints.

Usage: py script/worktrees-overview.py [repo] [--base REF] [--stale-days N] [--sizes] [--json]

Agents are found from their own session files:
  Claude Code (CLI and desktop): ~/.claude/projects/<encoded cwd>/<session>.jsonl
  Codex: ~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl (session_meta.cwd)
"""

import argparse
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

NOW = time.time()
HOME = Path.home()


def git(cwd, *args):
    r = subprocess.run(["git", "-C", str(cwd), *args], capture_output=True, text=True,
                       encoding="utf-8", errors="replace")
    return r.stdout.strip() if r.returncode == 0 else None


def norm(p):
    return os.path.normcase(os.path.normpath(str(p)))


def inside(child, parent):
    c, p = norm(child), norm(parent)
    return c == p or c.startswith(p.rstrip("\\/") + os.sep)


def ago(ts):
    if not ts:
        return "-"
    s = NOW - ts
    if s < 90:
        return "now"
    for unit, n in (("d", 86400), ("h", 3600), ("m", 60)):
        if s >= n:
            return f"{int(s // n)}{unit} ago"
    return f"{int(s)}s ago"


def list_worktrees(repo):
    out, cur = [], None
    for line in (git(repo, "worktree", "list", "--porcelain") or "").splitlines():
        if line.startswith("worktree "):
            cur = {"path": line[9:], "branch": None, "locked": False, "prunable": False}
            out.append(cur)
        elif cur is None:
            continue
        elif line.startswith("HEAD "):
            cur["head"] = line[5:]
        elif line.startswith("branch "):
            cur["branch"] = line[7:].removeprefix("refs/heads/")
        elif line == "bare":
            cur["bare"] = True
        elif line.startswith("locked"):
            cur["locked"] = True
        elif line.startswith("prunable"):
            cur["prunable"] = True
    return [w for w in out if not w.get("bare")]


def tail_lines(path, max_bytes=768 * 1024):
    with open(path, "rb") as f:
        f.seek(0, 2)
        size = f.tell()
        f.seek(max(0, size - max_bytes))
        data = f.read().decode("utf-8", errors="replace")
    lines = data.splitlines()
    return lines[1:] if size > max_bytes else lines


def claude_sessions():
    """Newest session per Claude Code project dir: cwd, title, last activity, state."""
    root = HOME / ".claude" / "projects"
    if not root.is_dir():
        return []
    sessions = []
    for d in root.iterdir():
        if not d.is_dir():
            continue
        files = sorted(d.glob("*.jsonl"), key=lambda p: p.stat().st_mtime, reverse=True)
        for f in files[:3]:
            info = {"agent": "Claude Code", "file": str(f), "mtime": f.stat().st_mtime,
                    "cwd": None, "title": None, "stop": None}
            for line in tail_lines(f):
                try:
                    o = json.loads(line)
                except ValueError:
                    continue
                t = o.get("type")
                if t == "custom-title" and o.get("customTitle"):
                    info["title"] = o["customTitle"]
                if o.get("cwd"):
                    info["cwd"] = o["cwd"]
                if t == "assistant":
                    info["stop"] = (o.get("message") or {}).get("stop_reason")
                elif t == "user":
                    info["stop"] = "user"
            if info["cwd"]:
                sessions.append(info)
    return sessions


def codex_sessions(max_age_days=60):
    root = HOME / ".codex" / "sessions"
    if not root.is_dir():
        return []
    out = []
    for f in root.glob("*/*/*/rollout-*.jsonl"):
        m = f.stat().st_mtime
        if NOW - m > max_age_days * 86400:
            continue
        try:
            with open(f, encoding="utf-8", errors="replace") as fh:
                meta = json.loads(fh.readline()).get("payload", {})
        except (ValueError, OSError):
            continue
        if meta.get("cwd"):
            out.append({"agent": "Codex", "file": str(f), "mtime": m, "cwd": meta["cwd"],
                        "title": None, "stop": None})
    return out


def agent_state(s):
    age = NOW - s["mtime"]
    if age < 120:
        return "running"
    if s["stop"] == "tool_use" and age < 1800:
        return "tool pending (permission?)"
    if s["stop"] in ("end_turn", "stop_sequence"):
        return "turn ended"
    if s["stop"] in ("user", "tool_use"):
        return "stopped mid-turn"
    return "idle"


def dir_size(path, skip=(".git",)):
    total = 0
    stack = [path]
    while stack:
        p = stack.pop()
        try:
            with os.scandir(p) as it:
                for e in it:
                    if e.name in skip:
                        continue
                    if e.is_dir(follow_symlinks=False):
                        stack.append(e.path)
                    elif e.is_file(follow_symlinks=False):
                        total += e.stat(follow_symlinks=False).st_size
        except OSError:
            pass
    return total


def gb(n):
    return f"{n / 2**30:.1f}G" if n >= 2**30 else f"{n / 2**20:.0f}M"


def owner(path, main_path):
    p = norm(path)
    if os.sep + ".claude" + os.sep + "worktrees" + os.sep in p:
        return "Claude"
    if os.sep + ".codex" + os.sep in p or "codex" in Path(p).parts[-2:-1]:
        return "Codex"
    if os.sep + "worktrees" + os.sep in p:
        return "Zed/Fyx"
    if norm(path) == norm(main_path):
        return "main"
    return "manual"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("repo", nargs="?", default=".")
    ap.add_argument("--base", help="base ref (default: origin/HEAD)")
    ap.add_argument("--stale-days", type=int, default=7)
    ap.add_argument("--sizes", action="store_true", help="measure worktree and target/ sizes (slow)")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()

    top = git(a.repo, "rev-parse", "--path-format=absolute", "--git-common-dir")
    if not top:
        sys.exit("not a git repository")
    wts = list_worktrees(a.repo)
    main_wt = wts[0]
    base = a.base or (git(a.repo, "symbolic-ref", "--short", "refs/remotes/origin/HEAD") or "HEAD")
    targets = [base] + ([main_wt["branch"]] if main_wt.get("branch") else [])

    sessions = claude_sessions() + codex_sessions()
    rows = []
    for w in wts:
        path = w["path"]
        r = {"name": Path(path).name, "path": path, "branch": w["branch"] or "(detached)",
             "owner": owner(path, main_wt["path"]), "locked": w["locked"],
             "missing": w["prunable"] or not Path(path).exists()}
        if r["missing"]:
            r["category"] = "MISSING"
            rows.append(r)
            continue
        log = git(path, "log", "-1", "--format=%ct")
        r["last_commit"] = int(log) if log else None
        status = (git(path, "status", "--porcelain") or "").splitlines()
        r["untracked"] = sum(1 for s in status if s.startswith("??"))
        r["changed"] = len(status) - r["untracked"]
        dirty_mtime = 0
        for s in status[:200]:
            fp = Path(path) / s[3:].strip().strip('"').split(" -> ")[-1]
            try:
                dirty_mtime = max(dirty_mtime, fp.stat().st_mtime)
            except OSError:
                pass
        counts = git(a.repo, "rev-list", "--left-right", "--count", f"{base}...{w['head']}")
        r["behind"], r["ahead"] = (int(x) for x in counts.split()) if counts else (None, None)
        r["merged_into"] = None
        if r["ahead"] and w is not main_wt:
            for t in targets:
                if subprocess.run(["git", "-C", a.repo, "merge-base", "--is-ancestor", w["head"], t],
                                  capture_output=True).returncode == 0:
                    r["merged_into"] = t
                    break
        unpushed = git(a.repo, "rev-list", "--count", w["head"], "--not", "--remotes")
        r["unpushed"] = int(unpushed) if unpushed else 0

        mine = [s for s in sessions if inside(s["cwd"], path)
                and not any(inside(s["cwd"], o["path"]) and len(o["path"]) > len(path) for o in wts)]
        mine.sort(key=lambda s: s["mtime"], reverse=True)
        r["sessions"] = len(mine)
        if mine:
            s = mine[0]
            r["agent"] = s["agent"]
            r["agent_title"] = s["title"]
            r["agent_state"] = agent_state(s)
            r["agent_mtime"] = s["mtime"]
        if a.sizes:
            r["size"] = dir_size(path, skip=(".git", "target"))
            t = Path(path) / "target"
            r["target_size"] = dir_size(t, skip=()) if t.is_dir() else 0

        last = max(filter(None, [r.get("last_commit"), dirty_mtime, r.get("agent_mtime")]), default=0)
        r["last_activity"] = last
        clean = r["changed"] == 0 and r["untracked"] == 0
        if w is main_wt:
            r["category"] = "MAIN"
        elif r.get("agent_state") in ("running", "tool pending (permission?)"):
            r["category"] = "ACTIVE"
        elif (clean and r["unpushed"] == 0 and (r["merged_into"] or r["ahead"] == 0)
              and NOW - r.get("agent_mtime", 0) > 86400):
            # Nothing would be lost, and no agent touched it for a day.
            r["category"] = "SAFE TO REMOVE"
        elif NOW - last > a.stale_days * 86400:
            r["category"] = "FORGOTTEN"
        else:
            r["category"] = "IDLE"
        rows.append(r)

    if a.json:
        print(json.dumps({"base": base, "worktrees": rows}, indent=2, default=str))
        return

    order = ["ACTIVE", "MAIN", "IDLE", "FORGOTTEN", "SAFE TO REMOVE", "MISSING"]
    print(f"{len(rows)} worktrees, base {base}, stale after {a.stale_days}d\n")
    for cat in order:
        group = [r for r in rows if r["category"] == cat]
        if not group:
            continue
        print(f"== {cat} ({len(group)})")
        for r in sorted(group, key=lambda r: r.get("last_activity", 0), reverse=True):
            if r["missing"]:
                print(f"  {r['name']:<34} directory is gone        -> git worktree prune")
                continue
            vs = "merged" if r["merged_into"] else f"+{r['ahead']}/-{r['behind']}"
            if r["merged_into"]:
                vs += f" into {r['merged_into']}"
            dirty = []
            if r["changed"]:
                dirty.append(f"{r['changed']} changed")
            if r["untracked"]:
                dirty.append(f"{r['untracked']} untracked")
            if r["unpushed"]:
                dirty.append(f"{r['unpushed']} unpushed")
            line1 = f"  {r['name']:<34} {r['branch'][:38]:<38} [{r['owner']}]"
            if r["locked"]:
                line1 += " locked"
            print(line1)
            agent = "no agent session found"
            if r.get("agent"):
                title = f" \"{r['agent_title'][:48]}\"" if r.get("agent_title") else ""
                agent = f"{r['agent']}{title}: {r['agent_state']}, {ago(r['agent_mtime'])}"
                if r["sessions"] > 1:
                    agent += f" ({r['sessions']} sessions)"
            print(f"      {vs:<34} {', '.join(dirty) or 'clean':<30} last activity {ago(r['last_activity'])}")
            print(f"      {agent}")
            if "size" in r:
                tsz = f", target/ {gb(r['target_size'])}" if r["target_size"] else ""
                print(f"      size {gb(r['size'])}{tsz}")
            if cat == "SAFE TO REMOVE":
                hint = "archive its session first, then " if r.get("agent") == "Claude Code" else ""
                print(f"      -> {hint}git worktree remove \"{r['path']}\"")
        print()


if __name__ == "__main__":
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")
    main()
