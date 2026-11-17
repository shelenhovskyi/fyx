#!/usr/bin/env node
// A scripted ACP (protocol v1) agent for checking agent review by hand in a dev build.
//
// Each prompt edits `demo.txt` in the session's working directory (or the file named after "edit"
// in the prompt): it upper-cases the second and the last line, which gives two separate hunks.
// - default: ask `session/request_permission` with the diff, then write through `fs/write_text_file`
//   if allowed (the flow Claude Agent uses);
// - prompt contains "direct": write through `fs/write_text_file` without asking.
// With `REVIEW_STUB_AUTOSTART=1` the agent also starts on its own right after `session/new`: first the
// permission flow on `demo.txt`, then the direct write on `demo2.txt` (for checks without typing).
//
// Configure in settings.json:
//   "agent_servers": { "Review Stub": { "type": "custom", "command": "node", "args": ["<this file>"] } }

"use strict";

const path = require("path");
const readline = require("readline");

let nextId = 1;
const pending = new Map();
const sessions = new Map();

function send(message) {
  process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...message }) + "\n");
}

function request(method, params) {
  const id = nextId++;
  send({ id, method, params });
  return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
}

function notify(method, params) {
  send({ method, params });
}

function log(text) {
  const line = `[review-stub] ${new Date().toISOString()} ${text}\n`;
  process.stderr.write(line);
  if (process.env.REVIEW_STUB_LOG) {
    require("fs").appendFileSync(process.env.REVIEW_STUB_LOG, line);
  }
}

function update(sessionId, value) {
  notify("session/update", { sessionId, update: value });
}

function say(sessionId, text) {
  update(sessionId, { sessionUpdate: "agent_message_chunk", content: { type: "text", text } });
}

function propose(oldText) {
  const lines = oldText.split("\n");
  const trailing = lines.length > 0 && lines[lines.length - 1] === "";
  const body = trailing ? lines.slice(0, -1) : lines;
  if (body.length < 3) {
    throw new Error("the file needs at least three lines");
  }
  body[1] = body[1].toUpperCase();
  body[body.length - 1] = body[body.length - 1].toUpperCase();
  return body.join("\n") + (trailing ? "\n" : "");
}

async function prompt(params) {
  const sessionId = params.sessionId;
  const cwd = sessions.get(sessionId) || process.cwd();
  const text = (params.prompt || [])
    .filter((block) => block.type === "text")
    .map((block) => block.text)
    .join(" ");
  const named = /edit\s+(\S+)/.exec(text);
  const file = path.resolve(cwd, named ? named[1] : "demo.txt");
  return runEdit(sessionId, file, /\bdirect\b/.test(text));
}

async function runEdit(sessionId, file, direct) {
  log(`proposing an edit to ${path.basename(file)} (${direct ? "direct write" : "permission request"})`);
  const { content: oldText } = await request("fs/read_text_file", { sessionId, path: file });
  const newText = propose(oldText);
  const toolCallId = `edit-${nextId++}`;
  const diff = { type: "diff", path: file, oldText, newText };
  const toolCall = {
    toolCallId,
    title: `Edit ${path.basename(file)}`,
    kind: "edit",
    status: "pending",
    content: [diff],
    locations: [{ path: file }],
  };
  update(sessionId, { sessionUpdate: "tool_call", ...toolCall });

  if (!direct) {
    const answer = await request("session/request_permission", {
      sessionId,
      toolCall,
      options: [
        { optionId: "allow", name: "Allow", kind: "allow_once" },
        { optionId: "reject", name: "Reject", kind: "reject_once" },
      ],
    });
    const outcome = answer && answer.outcome;
    log(`permission answer: ${JSON.stringify(outcome)}`);
    if (!outcome || outcome.outcome !== "selected" || outcome.optionId !== "allow") {
      update(sessionId, { sessionUpdate: "tool_call_update", toolCallId, status: "failed" });
      say(sessionId, `The edit to ${path.basename(file)} was not allowed.`);
      return { stopReason: "end_turn" };
    }
  }

  update(sessionId, { sessionUpdate: "tool_call_update", toolCallId, status: "in_progress" });
  try {
    await request("fs/write_text_file", { sessionId, path: file, content: newText });
    update(sessionId, { sessionUpdate: "tool_call_update", toolCallId, status: "completed" });
    log(`write of ${path.basename(file)} succeeded`);
    say(sessionId, `Wrote ${path.basename(file)}.`);
  } catch (error) {
    const message = error && error.message ? error.message : String(error);
    log(`write failed: ${message}`);
    update(sessionId, {
      sessionUpdate: "tool_call_update",
      toolCallId,
      status: "failed",
      content: [{ type: "content", content: { type: "text", text: message } }],
    });
    say(sessionId, `Write result: ${message}`);
  }
  return { stopReason: "end_turn" };
}

async function writeWhole(sessionId, file, content) {
  try {
    await request("fs/write_text_file", { sessionId, path: file, content });
    log(`write of ${path.basename(file)} succeeded`);
  } catch (error) {
    log(`write of ${path.basename(file)} failed: ${error.message}`);
  }
}

// `REVIEW_STUB_SCENARIO=turn-rules-bypass`, one turn:
// 1. writes covered by `.fyx/review.toml` (`deny` Cargo.lock, `auto_accept` docs/**);
// 2. two edits proposed at once (demo.txt, demo2.txt), held together in the turn's review;
// 3. a file written straight to disk, as a shell command would, bypassing review.
async function scenarioTurnRulesBypass(sessionId, cwd) {
  await writeWhole(sessionId, path.resolve(cwd, "Cargo.lock"), "# rewritten by the agent\n");
  await writeWhole(sessionId, path.resolve(cwd, "docs", "guide.md"), "# Guide, edited by the agent\n");
  await Promise.all([
    runEdit(sessionId, path.resolve(cwd, "demo.txt"), true),
    runEdit(sessionId, path.resolve(cwd, "demo2.txt"), true),
  ]);
  require("fs").writeFileSync(path.resolve(cwd, "outside.txt"), "written by a shell command\n");
  log("wrote outside.txt directly on disk");
}

// `REVIEW_STUB_SCENARIO=full`, one turn:
// 1. rules: a denied write (Cargo.lock), an auto-accepted one (docs/guide.md);
// 2. a permission request with a diff (demo3.txt), as Claude Agent asks;
// 3. two writes at once (demo.txt, demo2.txt);
// 4. a file written straight to disk (outside.txt);
// 5. the agent rewrites `.fyx/review.toml` to deny docs/**, then writes docs/guide.md again: the new
//    rules must not apply before the user confirms them.
async function scenarioFull(sessionId, cwd) {
  await writeWhole(sessionId, path.resolve(cwd, "Cargo.lock"), "# rewritten by the agent\n");
  await writeWhole(sessionId, path.resolve(cwd, "docs", "guide.md"), "# Guide, edited by the agent\n");
  await runEdit(sessionId, path.resolve(cwd, "demo3.txt"), false);
  await Promise.all([
    runEdit(sessionId, path.resolve(cwd, "demo.txt"), true),
    runEdit(sessionId, path.resolve(cwd, "demo2.txt"), true),
  ]);
  const fs = require("fs");
  fs.writeFileSync(path.resolve(cwd, "outside.txt"), "written by a shell command\n");
  log("wrote outside.txt directly on disk");
  fs.writeFileSync(path.resolve(cwd, ".fyx", "review.toml"), 'deny = ["docs/**"]\n');
  log("rewrote .fyx/review.toml directly on disk");
  await new Promise((resolve) => setTimeout(resolve, 1500));
  await writeWhole(sessionId, path.resolve(cwd, "docs", "guide.md"), "# Guide, edited again\n");
}

async function handle(message) {
  switch (message.method) {
    case "initialize":
      return {
        protocolVersion: 1,
        agentCapabilities: { loadSession: false, promptCapabilities: {} },
        authMethods: [],
      };
    case "session/new": {
      const sessionId = `stub-${Date.now()}-${nextId++}`;
      const cwd = message.params.cwd;
      sessions.set(sessionId, cwd);
      if (process.env.REVIEW_STUB_SCENARIO === "full") {
        setTimeout(() => {
          scenarioFull(sessionId, cwd).catch((error) => log(`scenario failed: ${error.message}`));
        }, 2000);
      } else if (process.env.REVIEW_STUB_SCENARIO === "turn-rules-bypass") {
        setTimeout(() => {
          scenarioTurnRulesBypass(sessionId, cwd).catch((error) =>
            log(`scenario failed: ${error.message}`),
          );
        }, 2000);
      } else if (process.env.REVIEW_STUB_AUTOSTART === "1") {
        setTimeout(async () => {
          try {
            await runEdit(sessionId, path.resolve(cwd, "demo.txt"), false);
            await runEdit(sessionId, path.resolve(cwd, "demo2.txt"), true);
          } catch (error) {
            log(`autostart failed: ${error.message}`);
          }
        }, 2000);
      }
      return { sessionId };
    }
    case "session/prompt":
      return prompt(message.params);
    case "session/cancel":
      return null;
    default:
      throw Object.assign(new Error(`method not found: ${message.method}`), { code: -32601 });
  }
}

readline.createInterface({ input: process.stdin }).on("line", (line) => {
  if (!line.trim()) {
    return;
  }
  let message;
  try {
    message = JSON.parse(line);
  } catch (error) {
    log(`bad json: ${line}`);
    return;
  }
  if (message.method === undefined && message.id !== undefined) {
    const waiter = pending.get(message.id);
    pending.delete(message.id);
    if (waiter) {
      if (message.error) {
        waiter.reject(new Error(message.error.message || JSON.stringify(message.error)));
      } else {
        waiter.resolve(message.result);
      }
    }
    return;
  }
  handle(message).then(
    (result) => {
      if (message.id !== undefined) {
        send({ id: message.id, result: result === undefined ? null : result });
      }
    },
    (error) => {
      log(`error in ${message.method}: ${error.message}`);
      if (message.id !== undefined) {
        send({ id: message.id, error: { code: error.code || -32603, message: error.message } });
      }
    },
  );
});
