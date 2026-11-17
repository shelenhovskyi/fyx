# Fyx phase 4 manual tests

Run these by hand in the running editor on the Windows 11 dev machine, on the build that W0 makes
from `integration` (`FULL=1 script/gate.sh`). Green tests are not done: each script below lists what
you must *see*. Record every run in `tasks/progress/W7.md` as one line: test, commit, `PASS`, or
`FAIL` with what you saw.

Conventions:
- `$fyx` is the full path of the built binary (`target\debug\fyx.exe` once W1 renames it, otherwise
  `target\debug\zed.exe`). Shell: `pwsh`.
- Start from a clean profile for each test, so no state carries over from earlier runs. Delete
  Fyx's data and config directories first (after W1: `%APPDATA%\Fyx` and `%LOCALAPPDATA%\Fyx`). Never
  delete `...\Zed`: those belong to the installed Zed.
- "Install the demo" means the load path from review finding 7 (`[fyx_ui]` in `extension.toml`).
  Until that path exists, tests T1–T3 are blocked.

## T1 Counter panel: the click loop

Covers: dock panel container, flex/text/button/list primitives, click → one `handle-event` →
patches → repaint, theme tokens, virtualized list.

1. Build `fyx_demo_counter` for `wasm32-wasip2` and install it. Start `$fyx` on an empty folder.
2. Open the right dock. A **Counter** panel with icon `#` is listed. Open it.
   - Expect: `Count: 0` in bold. Buttons **Decrement** (disabled), **Increment** (filled) and
     **Reset** (disabled). An empty list.
3. Click **Increment** once.
   - Expect: `Count: 1` within one frame, with no visible flicker or relayout of the panel.
     Decrement and Reset become enabled. The list shows one row, `Row 1 of 1`.
4. Click **Increment** 30 times quickly.
   - Expect: `Count: 31`. Every click is counted: none dropped, none doubled. Scroll the list: rows
     read `Row i of 31` and scrolling stays smooth.
5. Scroll to the middle of the list, then click **Increment** once.
   - Expect: every visible row now reads `of 32`. This is the row-refresh check from review finding 5.
6. Click **Decrement** once, then **Reset**.
   - Expect: `Count: 31`, then `Count: 0`. Decrement and Reset are disabled again and the list is empty.
7. Switch themes: command palette → `theme selector: toggle`, then pick a light theme and a dark theme.
   - Expect: the panel's text, buttons and background follow the theme immediately. No colour stays
     from the previous theme.
8. Close the panel and reopen it.
   - Expect: it shows the current state with no errors. Then click **Increment** once more and check
     that the count still updates (re-mount check from review finding 2).
9. Open the log (`zed: open log` or Fyx's renamed equivalent).
   - Expect: no `fyx_ui` errors and no `disabled after an earlier failure`.

## T2 Todo panel

Covers: input with typing debounce, input echo, list rebuild, `row-click`, a second extension
running alongside the first.

1. Install `fyx_demo_todo` next to the counter. Start `$fyx` and open the left dock **TODOs** panel.
   - Expect: the header `Workspace TODOs` and a summary like `N of N items in 4 files` (the
     extension reads mock data until worktree access exists). Kind buttons **All** (filled),
     **TODO**, **FIXME** and **HACK**, plus **Rescan**. The list shows TODO in the accent colour,
     FIXME in the error colour and HACK in the warning colour.
2. Type `spin` into the filter input at normal speed.
   - Expect: the cursor never jumps and no typed character is lost or duplicated. The list narrows
     to the "show a spinner" item and the summary reads `1 of N`.
3. Select all the text in the input and delete it.
   - Expect: the full list comes back.
4. Click **FIXME**.
   - Expect: FIXME becomes filled and All becomes subtle. Only FIXME items are listed.
5. Click a row.
   - Expect: the detail line shows `path:line  <source line>` in monospace.
6. Click **Rescan**.
   - Expect: the detail line resets to `Click an item to see its source line.` and the list is
     unchanged.
7. With both panels open, click quickly back and forth between the counter and the todo panel.
   - Expect: both stay responsive. A slow call in one extension does not delay the other, because
     each has its own queue.

## T3 Status item

Blocked: no demo contributes a `status-item` surface yet. Proposed for W4: the counter adds a
surface `counter-status` (status bar, right) showing `Count: N`. The panel's click handler updates
it through `push-update`.

1. Start `$fyx` with the counter installed.
   - Expect: `Count: 0` in the status bar on the right. Its order relative to the built-in items
     follows `priority` (higher is closer to the edge).
2. Click **Increment** in the counter panel three times.
   - Expect: the status item reads `Count: 3`, updated in the same frame as the panel or the next
     one.
3. Close the counter panel.
   - Expect: the status item stays visible and keeps its value.
4. Switch between a light and a dark theme.
   - Expect: the item follows the status bar's colours.

## T4 Agent review: accept one hunk, reject another

Covers: W5 `agent_review`, an ACP `session/request_permission` routed into the host's review, and
host-owned consent.

Setup: an ACP agent (Claude Agent) configured with your own key, which the tester enters, never an
agent. Use a scratch git repository containing `review.txt`:

```
alpha
bravo
charlie
delta
echo
foxtrot
golf
```

1. Open the scratch repository in `$fyx`. In the agent panel, ask: *"In review.txt, change `bravo`
   to `BRAVO` and `foxtrot` to `FOXTROT`. Make no other change."*
2. When the agent proposes the edit:
   - Expect: a review view opens, built on the editor's diff, with **two** hunks. `review.txt` on
     disk is still unchanged (check with `git diff --stat` in a terminal).
3. Accept the `bravo` hunk and reject the `foxtrot` hunk.
4. Finish the review.
   - Expect: `git diff` shows only `-bravo` and `+BRAVO`; `foxtrot` is unchanged. The agent thread
     shows that one hunk was rejected.
5. While the review is open, press keys that would normally edit the buffer (for example type a
   letter).
   - Expect: the review owns key routing. Nothing is typed into `review.txt` behind the review.
6. Repeat step 1 and reject both hunks.
   - Expect: the file is unchanged on disk and the agent is told the edit was rejected.
7. Check the log.
   - Expect: no panic, and no write to `review.txt` before the decision.

## T5 Rebranded build: coexists with Zed, no network at startup

Covers: W1 rebrand and de-cloud.

1. Brand and paths. With the installed Zed open, start `$fyx` from a clean profile.
   - Expect: the title bar, about dialog and window title say **Fyx** and never "Zed". Both editors
     run at the same time. Fyx creates `%APPDATA%\Fyx` and `%LOCALAPPDATA%\Fyx`, and nothing new
     appears under `...\Zed`. The taskbar groups the two editors separately.
2. No network at startup. Close every Fyx window first. Then, in an elevated `pwsh`, record DNS and
   TCP activity for 60 seconds after launch:

   ```powershell
   Clear-DnsClientCache
   $p = Start-Process -FilePath $fyx -ArgumentList '--foreground', 'C:\Temp\fyx-empty' -PassThru
   $seen = @{}
   $end = (Get-Date).AddSeconds(60)
   while ((Get-Date) -lt $end) {
       $ids = @($p.Id) + (Get-CimInstance Win32_Process |
           Where-Object ParentProcessId -eq $p.Id).ProcessId
       Get-NetTCPConnection -OwningProcess $ids -ErrorAction SilentlyContinue |
           Where-Object { $_.RemoteAddress -notin '0.0.0.0', '::', '127.0.0.1', '::1' } |
           ForEach-Object { $seen["$($_.RemoteAddress):$($_.RemotePort) $($_.State)"] = $_.OwningProcess }
       Start-Sleep -Milliseconds 200
   }
   $seen
   Get-DnsClientCache | Select-Object Entry, Data
   ```

   - Expect: `$seen` is empty. The DNS cache has no `zed.dev`, `cloud.zed.dev`, `api.zed.dev`,
     `collab.zed.dev`, sentry or telemetry hosts. Any entry from another process on the machine is
     noise: rerun with other apps closed before calling it a FAIL.
   - The 200 ms poll can miss a short connection. Before you call a PASS, also run once with
     `pktmon start --capture --comp nics` (stop with `pktmon stop`, convert with
     `pktmon etl2txt PktMon.etl`) or watch Resource Monitor → Network.
3. With the window open, check that no sign-in button and no collab panel are shown. Check that the
   update check is off: `auto_update` in the default settings, and no request when you open the
   about dialog.
4. Deliberate network still works. Configure a BYO-key provider (for example Anthropic, with a key
   the tester enters) and send one message.
   - Expect: a connection only to that provider's host, and only after you send the message.
5. Run `$fyx --version`.
   - Expect: the output says Fyx and the upstream base `v1.28.1`.
