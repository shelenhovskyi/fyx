"""A recording IDE server for Claude Code's IDE protocol (fyx_claude_code compat recordings).

It plays the IDE side: a WebSocket on 127.0.0.1 with subprotocol `mcp` that checks
`X-Claude-Code-Ide-Authorization`, the lock file `<CLAUDE_CONFIG_DIR>/ide/<port>.lock`, scripted
tool answers, and every frame both ways logged as JSONL. It can also run an interactive `claude`
in a ConPTY, driven over a small control API on the same port (/ctl/...).

  uv run --with aiohttp --with pywinpty --with pyte recorder.py --config <dir> --project <dir> --out <file>
"""

import argparse
import asyncio
import json
import os
import re
import secrets
import threading
import time
from pathlib import Path

import pyte
from aiohttp import WSMsgType, web

HEADER = "X-Claude-Code-Ide-Authorization"

TOOLS = [
    ("openDiff", "Open a diff view for a file", {
        "old_file_path": "string", "new_file_path": "string",
        "new_file_contents": "string", "tab_name": "string"}),
    ("getDiagnostics", "Get language diagnostics from the IDE", {"uri": "string"}),
    ("close_tab", "Close a tab by name", {"tab_name": "string"}),
    ("closeAllDiffTabs", "Close all diff tabs", {}),
    ("getOpenEditors", "Get the open editor tabs", {}),
    ("getCurrentSelection", "Get the current text selection", {}),
    ("getLatestSelection", "Get the most recent text selection", {}),
    ("getWorkspaceFolders", "Get the workspace folders", {}),
    ("checkDocumentDirty", "Check whether a document has unsaved changes", {"filePath": "string"}),
    ("saveDocument", "Save a document", {"filePath": "string"}),
    ("openFile", "Open a file in the editor", {"filePath": "string"}),
]


def now_ms():
    return int(time.time() * 1000)


class Recorder:
    def __init__(self, args):
        self.config = Path(args.config)
        self.project = Path(args.project).resolve()
        self.out = open(args.out, "a", encoding="utf-8")
        self.token = secrets.token_hex(16)
        self.port = args.port
        self.lock_path = None
        self.connections = {}
        self.next_conn = 1
        self.diffs = 0
        self.pending_diffs = {}
        self.pty = None
        self.screen = pyte.Screen(140, 45)
        self.stream = pyte.Stream(self.screen)
        self.screen_lock = threading.Lock()
        self.selection = None
        self.loop = None

    def log(self, event):
        event = {"ts": now_ms(), **event}
        self.out.write(json.dumps(event, ensure_ascii=False) + "\n")
        self.out.flush()

    # --- MCP over WebSocket -------------------------------------------------------------

    async def ws_handler(self, request):
        got = request.headers.get(HEADER)
        protocols = request.headers.get("Sec-WebSocket-Protocol")
        self.log({"t": "ws-open", "auth_ok": got == self.token, "subprotocols": protocols,
                  "path": request.path,
                  "headers": sorted(k for k in request.headers.keys())})
        if got != self.token:
            return web.Response(status=401, text="bad auth")
        ws = web.WebSocketResponse(protocols=("mcp",), heartbeat=None, max_msg_size=64 << 20)
        await ws.prepare(request)
        conn = self.next_conn
        self.next_conn += 1
        self.connections[conn] = ws
        try:
            async for msg in ws:
                if msg.type == WSMsgType.TEXT:
                    try:
                        data = json.loads(msg.data)
                    except ValueError:
                        self.log({"t": "mcp-bad", "conn": conn, "raw": msg.data})
                        continue
                    self.log({"t": "mcp", "dir": "in", "conn": conn, "msg": data})
                    asyncio.ensure_future(self.handle(conn, data))
                elif msg.type in (WSMsgType.PING, WSMsgType.PONG):
                    self.log({"t": "ws-" + msg.type.name.lower(), "conn": conn})
                else:
                    self.log({"t": "ws-frame", "conn": conn, "type": msg.type.name})
        finally:
            self.connections.pop(conn, None)
            self.log({"t": "ws-close", "conn": conn, "code": ws.close_code})
        return ws

    async def send(self, conn, data):
        ws = self.connections.get(conn)
        if ws is None or ws.closed:
            self.log({"t": "send-dropped", "conn": conn, "msg": data})
            return
        self.log({"t": "mcp", "dir": "out", "conn": conn, "msg": data})
        await ws.send_str(json.dumps(data))

    async def notify_all(self, method, params):
        for conn in list(self.connections):
            await self.send(conn, {"jsonrpc": "2.0", "method": method, "params": params})

    async def handle(self, conn, data):
        method = data.get("method")
        rid = data.get("id")
        if rid is None:
            return  # notifications: logged only
        if method == "initialize":
            params = data.get("params") or {}
            result = {"protocolVersion": params.get("protocolVersion", "2025-06-18"),
                      "capabilities": {"tools": {"listChanged": True}},
                      "serverInfo": {"name": "fyx-recorder", "version": "0.1.0"}}
        elif method == "tools/list":
            result = {"tools": [
                {"name": name, "description": desc,
                 "inputSchema": {"type": "object",
                                 "properties": {k: {"type": v} for k, v in props.items()}}}
                for name, desc, props in TOOLS]}
        elif method == "tools/call":
            result = await self.call(conn, rid, data.get("params") or {})
        elif method == "ping":
            result = {}
        else:
            await self.send(conn, {"jsonrpc": "2.0", "id": rid,
                                   "error": {"code": -32601, "message": f"no method {method}"}})
            return
        await self.send(conn, {"jsonrpc": "2.0", "id": rid, "result": result})

    def text(self, value):
        if not isinstance(value, str):
            value = json.dumps(value)
        return {"content": [{"type": "text", "text": value}]}

    def uri(self, name):
        return (self.project / name).as_uri()

    async def call(self, conn, rid, params):
        name = params.get("name")
        args = params.get("arguments") or {}
        app = str(self.project / "app.py")
        if name == "openDiff":
            self.diffs += 1
            n = self.diffs
            await asyncio.sleep(2.0)
            if n == 1:
                # Accepted, with an edit of the user's own in the diff view.
                contents = args.get("new_file_contents", "")
                edited = contents.replace("def greet(name):", "def greet(name):  # reviewed in Fyx", 1)
                if edited == contents:
                    edited = contents.rstrip("\n") + "\n# reviewed in Fyx\n"
                self.log({"t": "note", "what": f"openDiff #{n}: accepted with a user edit"})
                return {"content": [{"type": "text", "text": "FILE_SAVED"},
                                    {"type": "text", "text": edited}]}
            self.log({"t": "note", "what": f"openDiff #{n}: rejected"})
            return self.text("DIFF_REJECTED")
        if name == "close_tab":
            return self.text("TAB_CLOSED")
        if name == "closeAllDiffTabs":
            return self.text("CLOSED_0_DIFF_TABS")
        if name == "getDiagnostics":
            source = (self.project / "app.py").read_text(encoding="utf-8")
            lines = source.splitlines()
            diags = []
            for i, line in enumerate(lines):
                if "lenght(" in line:
                    c = line.index("lenght")
                    diags.append({"message": '"lenght" is not defined', "severity": "Error",
                                  "range": {"start": {"line": i, "character": c},
                                            "end": {"line": i, "character": c + 6}},
                                  "source": "Pylance", "code": "reportUndefinedVariable"})
                if line.strip() == "import os":
                    diags.append({"message": 'Import "os" is not accessed', "severity": "Hint",
                                  "range": {"start": {"line": i, "character": 7},
                                            "end": {"line": i, "character": 9}},
                                  "source": "Pylance"})
            uri = args.get("uri")
            files = [{"uri": self.uri("app.py"), "diagnostics": diags}]
            if uri and uri != self.uri("app.py"):
                files = [{"uri": uri, "diagnostics": []}]
            return self.text(files)
        if name == "getOpenEditors":
            return self.text({"tabs": [{"uri": self.uri("app.py"), "isActive": True,
                                        "label": "app.py", "languageId": "python",
                                        "isDirty": False}]})
        if name in ("getCurrentSelection", "getLatestSelection"):
            sel = self.selection or {"text": "", "filePath": app,
                                     "selection": {"start": {"line": 0, "character": 0},
                                                   "end": {"line": 0, "character": 0},
                                                   "isEmpty": True}}
            return self.text({"success": True, **sel})
        if name == "getWorkspaceFolders":
            return self.text({"success": True, "folders": [
                {"name": self.project.name, "uri": self.project.as_uri(),
                 "path": str(self.project)}], "rootPath": str(self.project)})
        if name == "checkDocumentDirty":
            return self.text({"success": True, "filePath": args.get("filePath"),
                              "isDirty": False, "isUntitled": False})
        if name == "saveDocument":
            return self.text({"success": True, "filePath": args.get("filePath"),
                              "saved": True, "message": "Document saved"})
        if name == "openFile":
            return self.text(f"Opened file: {args.get('filePath')}")
        return {"content": [{"type": "text", "text": f"unknown tool {name}"}], "isError": True}

    # --- control API ------------------------------------------------------------------

    async def ctl_hook(self, request):
        body = await request.text()
        try:
            stdin = json.loads(body)
        except ValueError:
            stdin = body
        self.log({"t": "hook", "channel": request.query.get("channel"), "stdin": stdin})
        return web.Response(text="")

    async def ctl_select(self, request):
        body = await request.json()
        path = str(self.project / body["file"])
        lines = (self.project / body["file"]).read_text(encoding="utf-8").splitlines(True)
        (sl, sc), (el, ec) = body["start"], body["end"]
        if sl == el:
            text = lines[sl][sc:ec]
        else:
            text = lines[sl][sc:] + "".join(lines[sl + 1:el]) + lines[el][:ec]
        self.selection = {"text": text, "filePath": path, "fileUrl": Path(path).as_uri(),
                          "selection": {"start": {"line": sl, "character": sc},
                                        "end": {"line": el, "character": ec},
                                        "isEmpty": text == ""}}
        await self.notify_all("selection_changed", self.selection)
        return web.json_response({"ok": True, "connections": len(self.connections)})

    async def ctl_mention(self, request):
        body = await request.json()
        params = {"filePath": str(self.project / body["file"]),
                  "lineStart": body.get("lineStart"), "lineEnd": body.get("lineEnd")}
        await self.notify_all("at_mentioned", params)
        return web.json_response({"ok": True})

    async def ctl_start(self, request):
        from winpty import PtyProcess
        body = await request.json()
        # Nothing of the session that runs the recorder: no CLAUDE*/ANTHROPIC*/MCP* variables.
        env = {k: v for k, v in os.environ.items()
               if not k.upper().startswith(("CLAUDE", "ANTHROPIC", "MCP", "FYX"))}
        env.update(body.get("env", {}))
        self.pty = PtyProcess.spawn(body["argv"], cwd=str(self.project), env=env,
                                    dimensions=(45, 140))
        self.log({"t": "note", "what": "started claude", "argv": body["argv"][1:]})
        threading.Thread(target=self.read_pty, daemon=True).start()
        return web.json_response({"ok": True})

    def read_pty(self):
        while self.pty is not None:
            try:
                data = self.pty.read(4096)
            except EOFError:
                break
            except Exception as error:  # noqa: BLE001
                self.log({"t": "note", "what": f"pty read: {error}"})
                break
            with self.screen_lock:
                self.stream.feed(data)
        self.log({"t": "note", "what": "claude exited"})

    def screen_text(self):
        with self.screen_lock:
            return "\n".join(line.rstrip() for line in self.screen.display)

    async def ctl_type(self, request):
        body = await request.json()
        if "text" in body:
            self.pty.write(body["text"])
            await asyncio.sleep(0.5)
        for key in body.get("keys", []):
            self.pty.write(key)
            await asyncio.sleep(0.3)
        return web.json_response({"ok": True})

    async def ctl_screen(self, request):
        return web.Response(text=self.screen_text())

    async def ctl_wait(self, request):
        pattern = re.compile(request.query["re"], re.S)
        timeout = float(request.query.get("timeout", "120"))
        settle = float(request.query.get("settle", "1.5"))
        deadline = time.time() + timeout
        while time.time() < deadline:
            if pattern.search(self.screen_text()):
                await asyncio.sleep(settle)
                return web.Response(text="MATCHED\n" + self.screen_text())
            await asyncio.sleep(0.5)
        return web.Response(status=408, text="TIMEOUT\n" + self.screen_text())

    async def ctl_state(self, request):
        return web.json_response({"connections": list(self.connections), "diffs": self.diffs,
                                  "pty_alive": bool(self.pty and self.pty.isalive())})

    async def ctl_stop(self, request):
        if self.pty is not None:
            try:
                self.pty.terminate(force=True)
            except Exception:  # noqa: BLE001
                pass
        self.cleanup()
        asyncio.get_event_loop().call_later(0.5, os._exit, 0)
        return web.Response(text="stopping")

    def cleanup(self):
        if self.lock_path and self.lock_path.exists():
            self.lock_path.unlink()
        self.log({"t": "note", "what": "recorder stopped"})

    async def run(self):
        app = web.Application()
        app.router.add_get("/", self.ws_handler)
        app.router.add_post("/ctl/hook", self.ctl_hook)
        app.router.add_post("/ctl/select", self.ctl_select)
        app.router.add_post("/ctl/mention", self.ctl_mention)
        app.router.add_post("/ctl/start", self.ctl_start)
        app.router.add_post("/ctl/type", self.ctl_type)
        app.router.add_get("/ctl/screen", self.ctl_screen)
        app.router.add_get("/ctl/wait", self.ctl_wait)
        app.router.add_get("/ctl/state", self.ctl_state)
        app.router.add_post("/ctl/stop", self.ctl_stop)
        runner = web.AppRunner(app)
        await runner.setup()
        site = web.TCPSite(runner, "127.0.0.1", self.port)
        await site.start()
        port = site._server.sockets[0].getsockname()[1]
        ide = self.config / "ide"
        ide.mkdir(parents=True, exist_ok=True)
        self.lock_path = ide / f"{port}.lock"
        lock = {"pid": os.getpid(), "workspaceFolders": [str(self.project)], "ideName": "Fyx",
                "transport": "ws", "runningInWindows": True, "authToken": self.token}
        self.lock_path.write_text(json.dumps(lock), encoding="utf-8")
        self.log({"t": "note", "what": "listening", "port": port,
                  "lock": {**lock, "authToken": "<redacted>"}})
        print(f"PORT {port}", flush=True)
        while True:
            await asyncio.sleep(3600)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", required=True)
    parser.add_argument("--project", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--port", type=int, default=0)
    recorder = Recorder(parser.parse_args())
    try:
        asyncio.run(recorder.run())
    finally:
        recorder.cleanup()


if __name__ == "__main__":
    main()
