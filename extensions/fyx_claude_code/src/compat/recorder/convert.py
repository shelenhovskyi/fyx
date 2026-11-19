"""Turns a recorder.py log into (1) a sanitized recording and (2) a draft transcript.

  python convert.py <log.jsonl> --project <dir> --config <dir> --mode interactive|print
      --date YYYY-MM-DD --raw <out.raw.jsonl> --draft <out.draft.jsonl> [--windows]
  python convert.py recorded/<x>.raw.jsonl --draft <out.draft.jsonl> [--windows]

The recording keeps every event, with times relative to the first, and Windows paths under
C:\\work\\app. The draft is in `fyx-claude-transcript` form under /work/app, or with
`--windows` under C:\\work\\app with Claude's own path and URI forms (`C:\\a\\b`,
`file:///C:/a/b`, `file://C:\\a\\b`). It has no extension's answers or hooks' `expect`:
`compat::fill` (the ignored `fill_draft` test) adds them. Given a recording already sanitized
(`recorded/`), only the draft is written. Sanitized: the project and config dirs, transcript and scratchpad paths, session, prompt
and tool-use ids, other UUIDs, and pids. The auth token is never logged by the recorder.
"""

import argparse
import json
import re

UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}")
TOOLU = re.compile(r"toolu_[A-Za-z0-9]+")
# The tools the extension registers (ide::tools()), in order; the replay checks the names.
EXTENSION_TOOLS = ["openFile", "openDiff", "close_tab", "closeAllDiffTabs", "getDiagnostics"]


class Sanitizer:
    def __init__(self, project, config, posix):
        self.project = project.rstrip("\\/")
        self.config = config.rstrip("\\/")
        self.posix = posix
        self.ids = {}
        self.sessions = {}

    def id_for(self, kind, value):
        key = (kind, value)
        if key not in self.ids:
            count = sum(1 for k in self.ids if k[0] == kind) + 1
            self.ids[key] = f"{kind}-{count}"
        return self.ids[key]

    def session(self, value):
        if value not in self.sessions:
            self.sessions[value] = f"live-{len(self.sessions) + 1}"
        return self.sessions[value]

    def path(self, text):
        project = self.project
        forward = project.replace("\\", "/")
        rest = r"([^\s\"'`]*)"
        if self.posix:
            # Both URI forms Claude sends (file:///C:/... and file://C:\...) become file:///work/app.
            forms = [("file:///" + forward, "file:///work/app", True),
                     ("file://" + project, "file:///work/app", True),
                     (project, "/work/app", True),
                     (forward, "/work/app", True)]
        else:
            forms = [("file:///" + forward, "file:///C:/work/app", False),
                     ("file://" + project, "file://C:\\work\\app", False),
                     (project, "C:\\work\\app", False),
                     (forward, "C:/work/app", False)]
        for prefix, root, slashes in forms:
            text = re.sub(re.escape(prefix) + rest,
                          lambda m, root=root, slashes=slashes:
                          root + (m.group(1).replace("\\", "/") if slashes else m.group(1)),
                          text)
        config = "/tmp/claude-config" if self.posix else "C:\\claude-config"
        text = text.replace(self.config, config).replace(self.config.replace("\\", "/"), config)
        return text

    def value(self, value, key=None):
        if isinstance(value, dict):
            return {k: self.field(k, v) for k, v in value.items()}
        if isinstance(value, list):
            return [self.value(v) for v in value]
        if isinstance(value, str):
            # JSON inside a string (a tool result's text) is sanitized as JSON.
            if value[:1] in "[{":
                try:
                    inner = json.loads(value)
                except ValueError:
                    inner = None
                if isinstance(inner, (list, dict)):
                    return json.dumps(self.value(inner), ensure_ascii=False)
            text = self.path(value)
            text = TOOLU.sub(lambda m: self.id_for("toolu", m.group(0)), text)
            text = UUID.sub(lambda m: self.session(m.group(0)) if m.group(0) in self.sessions
                            else self.id_for("uuid", m.group(0)), text)
            return text
        return value

    def name_in(self, path):
        """The session a transcript or scratchpad path is named after (its last UUID, or a
        `live-N` of a sanitized recording)."""
        if UUID.search(path):
            return self.session(UUID.findall(path)[-1])
        named = re.findall(r"live-\d+", path)
        return named[-1] if named else "live"

    def field(self, key, value):
        if key == "session_id" and isinstance(value, str):
            return self.session(value)
        if key == "transcript_path" and isinstance(value, str):
            name = self.name_in(value)
            return (f"/tmp/claude-config/projects/-work-app/{name}.jsonl" if self.posix
                    else f"C:\\claude-config\\projects\\C--work-app\\{name}.jsonl")
        if key == "scratchpad_dir" and isinstance(value, str):
            name = self.name_in(value)
            return (f"/tmp/claude/{name}/scratchpad" if self.posix
                    else f"C:\\Users\\user\\AppData\\Local\\Temp\\claude\\{name}\\scratchpad")
        if key == "prompt_id" and isinstance(value, str):
            return self.id_for("prompt", value)
        if key == "pid" and isinstance(value, int):
            return 4242
        return self.value(value, key)


def relative(path):
    """`C:\\work\\app\\a\\b`, `file:///C:/work/app/a/b` or `/work/app/a/b` -> `a/b`."""
    path = path.removeprefix("file://").replace("\\", "/")
    if path[:1] == "/" and path[2:3] == ":":
        path = path[1:]
    for root in ("C:/work/app/", "/work/app/"):
        if path.startswith(root):
            return path[len(root):]
    raise SystemExit(f"not under the root: {path!r}")


def check_clean(text, project, config):
    escaped = json.dumps(project)[1:-1]
    for needle in (escaped, project.replace("\\", "/"), json.dumps(config)[1:-1],
                   "Users\\\\User\\\\", "Users/User/", "fyx-user-check", "oauth", "organization",
                   "emailAddress", "accountUuid"):
        if needle in text:
            raise SystemExit(f"left in output: {needle!r}")
    for match in re.finditer(r"[\w.+-]+@[\w-]+\.[\w.]+", text):
        raise SystemExit(f"an email-like string is left: {match.group(0)!r}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("log")
    parser.add_argument("--project")
    parser.add_argument("--config")
    parser.add_argument("--mode", choices=["interactive", "print"])
    parser.add_argument("--date")
    parser.add_argument("--raw")
    parser.add_argument("--draft", required=True)
    parser.add_argument("--windows", action="store_true")
    args = parser.parse_args()

    events = [json.loads(line) for line in open(args.log, encoding="utf-8") if line.strip()]
    if events[0].get("format") == "fyx-claude-recording":
        # Already sanitized: its paths are the project's, its config dir is the placeholder.
        recording = events.pop(0)
        args.project, args.config = "C:\\work\\app", "C:\\claude-config"
        args.mode, args.date = recording["mode"], recording["recorded"]
        args.raw = None
        for event in events:
            event["ts"] = event.pop("ms")
    elif not (args.project and args.config and args.mode and args.date and args.raw):
        parser.error("a recorder log needs --project, --config, --mode, --date and --raw")
    # Sessions that never got a prompt (a run that failed on its arguments) are left out.
    prompted = {e["stdin"].get("session_id") for e in events
                if e["t"] == "hook" and e["channel"].endswith("/user-prompt")}
    events = [e for e in events
              if not (e["t"] == "hook" and e["stdin"].get("session_id") not in prompted)]
    start = events[0]["ts"]

    raw = Sanitizer(args.project, args.config, posix=False)
    raw_name = (args.raw or args.log).replace("\\", "/").rsplit("/", 1)[-1]
    note = ("Recorded live from Claude Code 2.1.326 (Windows) by src/compat/recorder/recorder.py, "
            "which played the IDE; sanitized by convert.py. Every frame both ways and every hook.")
    lines = [json.dumps({"format": "fyx-claude-recording", "version": 1,
                         "claude_code": "2.1.326", "mode": args.mode, "recorded": args.date,
                         "root": "C:\\work\\app", "note": note}, ensure_ascii=False)]
    for event in events:
        event = dict(event)
        event["ms"] = event.pop("ts") - start
        event = {"ms": event.pop("ms"), **raw.value(event)}
        lines.append(json.dumps(event, ensure_ascii=False))
    text = "\n".join(lines) + "\n"
    if args.raw:
        check_clean(text, args.project, args.config)
        open(args.raw, "w", encoding="utf-8", newline="\n").write(text)

    draft = Sanitizer(args.project, args.config, posix=not args.windows)
    root = "C:\\work\\app" if args.windows else "/work/app"
    mapped = ("on Windows, with Claude's own path and URI forms" if args.windows
              else "on Windows (paths mapped to /work/app)")
    answers = {}
    for event in events:
        msg = event.get("msg") or {}
        if event["t"] == "mcp" and event["dir"] == "out" and "id" in msg:
            answers[msg["id"]] = msg.get("result")
    header = {"format": "fyx-claude-transcript", "version": 1, "claude_code": "2.1.326",
              "mode": args.mode, "recorded": args.date, "root": root,
              "note": f"Recorded live from Claude Code 2.1.326 {mapped}; "
                      "the raw recording is recorded/" + raw_name
                      + ". The extension's answers and the hooks' effects were filled by compat::fill "
                      "and checked against the recording."}
    out = [json.dumps(header, ensure_ascii=False)]
    diagnostics_state = None
    for event in events:
        t = event["t"]
        msg = draft.value(event.get("msg")) if "msg" in event else None
        if t == "hook":
            out.append(json.dumps({"t": "hook", "channel": event["channel"],
                                   "stdin": draft.value(event["stdin"])}, ensure_ascii=False))
        elif t == "mcp" and event["dir"] == "in":
            if msg.get("method") == "tools/call" and msg["params"]["name"] == "getDiagnostics":
                uri = (msg["params"].get("arguments") or {}).get("uri")
                original = (event["msg"]["params"].get("arguments") or {}).get("uri")
                result = answers.get(msg["id"])
                if result and (original is None or original.startswith("file:///")):
                    files = []
                    for entry in json.loads(result["content"][0]["text"]):
                        path = relative(draft.value(entry["uri"]))
                        files.append({"path": path, "diagnostics": [
                            {"range": {"start": [d["range"]["start"]["line"], d["range"]["start"]["character"]],
                                       "end": [d["range"]["end"]["line"], d["range"]["end"]["character"]]},
                             "severity": d["severity"], "message": d["message"],
                             **({"source": d["source"]} if "source" in d else {})}
                            for d in entry["diagnostics"]]})
                    if files != diagnostics_state:
                        diagnostics_state = files
                        out.append(json.dumps({"t": "diagnostics", "files": files}, ensure_ascii=False))
            out.append(json.dumps({"t": "mcp", "dir": "in", "msg": msg}, ensure_ascii=False))
        elif t == "mcp" and event["dir"] == "out":
            result = msg.get("result") or {}
            if "serverInfo" in result:
                out.append(json.dumps({"t": "mcp", "dir": "out", "msg": {
                    "jsonrpc": "2.0", "id": msg["id"], "result": {
                        "protocolVersion": result["protocolVersion"],
                        "capabilities": {"tools": {}},
                        "serverInfo": {"name": "Fyx", "version": "0.1.0"}}}}))
            elif "tools" in result:
                out.append(json.dumps({"t": "mcp", "dir": "out", "msg": {
                    "jsonrpc": "2.0", "id": msg["id"],
                    "result": {"tools": [{"name": n} for n in EXTENSION_TOOLS]}}}))
            elif msg.get("method") == "selection_changed":
                p = msg["params"]
                out.append(json.dumps({"t": "editor",
                                       "path": relative(p["filePath"]),
                                       "selection": {"start": [p["selection"]["start"]["line"], p["selection"]["start"]["character"]],
                                                     "end": [p["selection"]["end"]["line"], p["selection"]["end"]["character"]],
                                                     "text": p["text"]}}, ensure_ascii=False))
            elif msg.get("method") == "at_mentioned":
                p = msg["params"]
                out.append(json.dumps({"t": "mention", "path": relative(p["filePath"]),
                                       "lines": [p["lineStart"], p["lineEnd"]]}))
            elif result.get("content") and result["content"][0].get("text") in ("FILE_SAVED", "DIFF_REJECTED"):
                if result["content"][0]["text"] == "FILE_SAVED":
                    final = result["content"][1]["text"]
                    out.append(json.dumps({"t": "review", "call": msg["id"], "decision": "partial",
                                           "final_content": final}, ensure_ascii=False))
                else:
                    out.append(json.dumps({"t": "review", "call": msg["id"], "decision": "rejected"}))
            # Other answers are the extension's to give: compat::fill writes them.
    text = "\n".join(out) + "\n"
    if args.raw:  # A sanitized recording was checked when it was written.
        check_clean(text, args.project, args.config)
    open(args.draft, "w", encoding="utf-8", newline="\n").write(text)
    print(f"{args.raw or args.log}: {len(lines)} lines; {args.draft}: {len(out)} lines")


if __name__ == "__main__":
    main()
