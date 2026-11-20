"""Tests for registry.py: `python3 -m unittest test_registry` in scripts/ (dry-run.sh runs them)."""

import copy
import json
import unittest
from pathlib import Path

import registry

POLICY = registry.load_policy(registry.POLICY_DIR)
COMMIT = "0123456789abcdef0123456789abcdef01234567"


def entry_text(**overrides) -> str:
    fields = {
        "id": '"todo"',
        "owners": '["alice"]',
        "repository": '"https://github.com/alice/todo"',
        "commit": f'"{COMMIT}"',
        "path": '"."',
    }
    fields.update(overrides)
    return (
        f"id = {fields['id']}\nowners = {fields['owners']}\n"
        f"[source]\nrepository = {fields['repository']}\ncommit = {fields['commit']}\n"
        f"path = {fields['path']}\n"
    )


def entry(**overrides) -> dict:
    return registry.parse_entry(entry_text(**overrides), None, POLICY, [])


def release(version="1.0.0", tier="t0", capabilities=("ui:panel",)) -> dict:
    return {
        "id": "todo",
        "version": version,
        "tier": tier,
        "capabilities": list(capabilities),
    }


def pass_verdict() -> dict:
    return {"verdict": "pass", "reasons": [], "inputs_sha256": "x"}


def decision(**overrides) -> dict:
    data = {
        "registry_owner": "shelenhovskyi",
        "pr_author": "alice",
        "approvals": [],
        "labels": [],
        "changed_files": ["extensions/todo.toml"],
        "entry_before": None,
        "entry_after": entry(),
        "release": release(),
        "last_published": None,
        "ci": {"entry": True, "build": True},
        "agent_verdict": pass_verdict(),
        "reserved_ids": POLICY["reserved"],
    }
    data.update(overrides)
    return registry.decide(data)


class Entries(unittest.TestCase):
    def test_a_valid_entry(self):
        parsed = entry(owners='["bob", "alice"]')
        self.assertEqual(parsed["owners"], ["alice", "bob"])
        self.assertFalse(parsed["urgent"])

    def test_refusals(self):
        for overrides in [
            {"id": '"Todo"'},
            {"id": '"1todo"'},
            {"owners": "[]"},
            {"owners": '["a", "a"]'},
            {"repository": '"https://gitlab.com/alice/todo"'},
            {"commit": '"main"'},
            {"commit": f'"{COMMIT.upper()}"'},
            {"path": '"../other"'},
            {"path": '"/abs"'},
            {"id": '"fyx-todo"'},
        ]:
            with self.subTest(overrides), self.assertRaises(registry.Refused):
                entry(**overrides)
        with self.assertRaises(registry.Refused):
            registry.parse_entry(entry_text() + "extra = 1\n", None, POLICY, [])
        with self.assertRaises(registry.Refused):
            registry.parse_entry(entry_text(), "other", POLICY, [])

    def test_reserved_ids_are_the_owners(self):
        parsed = entry(id='"fyx-todo"', owners='["shelenhovskyi"]')
        self.assertEqual(parsed["id"], "fyx-todo")

    def test_near_copies(self):
        for copy_of in ["todo", "tod", "todoo", "to-do"]:
            with self.subTest(copy_of), self.assertRaises(registry.Refused):
                registry.parse_entry(entry_text(id='"todo"'), None, POLICY, [copy_of + "x"]
                                     if copy_of == "todo" else [copy_of])
        # The id itself is not a copy of itself.
        registry.parse_entry(entry_text(), None, POLICY, ["todo"])

    def test_tiers_come_from_the_policy_table(self):
        manifest = 'id = "todo"\nversion = "1.0.0"\n[fyx_ui]\ncapabilities = ["ui:panel", "workspace:read-files"]\n'
        derived = registry.derive_release(entry(), manifest, b"pkg", POLICY)
        self.assertEqual(derived["tier"], "t1")
        self.assertEqual(derived["package"]["size"], 3)
        self.assertTrue(derived["package"]["url"].endswith("/todo-v1.0.0/todo-1.0.0.fyxext"))
        for capabilities in ['["network:fetch"]', '["ui:unknown"]']:
            with self.subTest(capabilities), self.assertRaises(registry.Refused):
                registry.derive_release(
                    entry(),
                    f'id = "todo"\nversion = "1.0.0"\n[fyx_ui]\ncapabilities = {capabilities}\n',
                    b"",
                    POLICY,
                )
        with self.assertRaises(registry.Refused):
            registry.derive_release(entry(), 'id = "other"\nversion = "1.0.0"\n', b"", POLICY)

    def test_a_zed_extension_package_is_t2_and_says_so(self):
        """Owner's Q1: accepted, flagged, and T2 whatever it declares."""
        import io
        import zipfile

        def package(*names):
            out = io.BytesIO()
            with zipfile.ZipFile(out, "w") as archive:
                for name in names:
                    archive.writestr(name, b"x")
            return out.getvalue()

        manifest = 'id = "todo"\nversion = "1.0.0"\n[fyx_ui]\ncapabilities = ["ui:panel"]\n'
        plain = registry.derive_release(entry(), manifest, package("extension.toml", "fyx_ui.wasm"), POLICY)
        self.assertEqual(plain["tier"], "t0")
        self.assertNotIn("zed_extension", plain)
        for name in ["extension.wasm", "lib/Extension.WASM"]:
            with self.subTest(name):
                zed = registry.derive_release(entry(), manifest, package("extension.toml", name), POLICY)
                self.assertEqual(zed["tier"], "t2")
                self.assertIs(zed["zed_extension"], True)


class ZeroPointFour(unittest.TestCase):
    """fyx:ui 0.4 (docs/fyx/wit-0.4/design.md): T3, and the update diff of section C.5."""

    CAPABILITIES = {
        "t0": ["package:files", "ui:approvals"],
        "t1": ["context:editor", "workspace:open", "terminal:profiles", "terminal:pseudo",
               "terminal:status", "review:propose"],
        "t2": ["network:listen", "terminal:create", "terminal:env", "terminal:observe",
               "fs:discovery-file", "relay:hooks"],
        "t3": ["terminal:read", "terminal:write"],
    }

    def test_the_0_4_capabilities_have_the_tiers_of_the_design(self):
        for tier, capabilities in self.CAPABILITIES.items():
            for capability in capabilities:
                with self.subTest(capability):
                    self.assertEqual(registry.tier_of(capability, POLICY["tiers"]), tier)

    def test_a_t3_capability_makes_a_t2_release(self):
        self.assertEqual(registry.release_tier("t3"), "t2")
        self.assertEqual([registry.release_tier(t) for t in ("t0", "t1", "t2")], ["t0", "t1", "t2"])
        # T3 outranks T1; the release counts it as T2, which is not accepted yet, like any T2.
        for capabilities in (["terminal:read"], ["ui:panel", "terminal:profiles", "terminal:write"]):
            with self.subTest(capabilities), self.assertRaises(registry.Refused):
                registry.derive_release(
                    entry(),
                    'id = "todo"\nversion = "1.0.0"\n[fyx_ui]\n'
                    f"capabilities = {json.dumps(capabilities)}\n",
                    b"",
                    POLICY,
                )
        # A T1-only 0.4 release is still T1.
        derived = registry.derive_release(
            entry(),
            'id = "todo"\nversion = "1.0.0"\n[fyx_ui]\ncapabilities = ["ui:panel", "terminal:pseudo"]\n',
            b"",
            POLICY,
        )
        self.assertEqual(derived["tier"], "t1")

    def test_no_signed_tier_is_t3(self):
        self.assertTrue(any(tier == "t3" for tier in POLICY["tiers"].values()))
        self.assertEqual({registry.release_tier(t) for t in registry.TIERS},
                         {"t0", "t1", "t2"})

    OLD = (
        'id = "cc"\nversion = "1.2.0"\n[fyx_ui]\n'
        'capabilities = ["ui:panel", "terminal:create", "terminal:env", "network:fetch"]\n'
        '[fyx_ui.network]\ndomains = ["api.example.com"]\n'
        '[[fyx_ui.terminal_programs]]\nname = "codex"\nargs = ["--quiet"]\nreason = "x"\n'
        '[fyx_ui.terminal_env]\nvars = { A = "1" }\nreason = "x"\n'
    )
    NEW = (
        'id = "cc"\nversion = "1.3.0"\n[fyx_ui]\n'
        'capabilities = ["ui:panel", "ui:approvals", "terminal:create", "terminal:env", '
        '"network:listen", "fs:discovery-file", "relay:hooks"]\n'
        '[[fyx_ui.terminal_programs]]\nname = "codex"\nargs = ["-c", "notify=*"]\nreason = "x"\n'
        '[fyx_ui.terminal_env]\nvars = { A = "1", CODEX_TUI_NOTIFY = "1" }\nscope = "all-new"\n'
        'reason = "x"\n'
        '[[fyx_ui.listeners]]\nname = "mcp"\nprotocol = "mcp-websocket"\nreason = "x"\n'
        '[fyx_ui.listeners.auth]\nbearer = true\n'
        '[[fyx_ui.discovery_files]]\nlistener = "mcp"\ndir = "${account.claude}/ide"\n'
        'name = "${port}.lock"\nreason = "x"\n[fyx_ui.discovery_files.content]\nport = "${port}"\n'
        '[[fyx_ui.relay_channels]]\nname = "hook"\non_failure = { exit = 2 }\nreason = "x"\n'
    )

    def test_the_update_diff_lists_what_is_added_widened_and_removed(self):
        diff = registry.update_diff(self.OLD, self.NEW, POLICY["tiers"])
        self.assertEqual(diff["version"], "1.3.0")
        added = {a["item"]: a["tier"] for a in diff["added"]}
        # Widened: the program's argument pattern, a new env name, scope all-new.
        self.assertEqual(added["creates a terminal running codex -c notify=*"], "t2")
        self.assertEqual(added["sets CODEX_TUI_NOTIFY=1 in its terminals"], "t2")
        self.assertIn("sets its env in every new terminal (scope all-new)", added)
        for item in ("capability network:listen", "capability fs:discovery-file",
                     "capability relay:hooks",
                     "listens on loopback: mcp (mcp-websocket)",
                     "writes a discovery file ${account.claude}/ide/${port}.lock",
                     "relay channel hook (timeout 30000 ms, on failure exit 2)"):
            self.assertEqual(added[item], "t2", item)
        # A T0 addition installs silently, so it is not listed.
        self.assertNotIn("capability ui:approvals", added)
        # Removals: the old pattern, the domain and network:fetch.
        self.assertEqual(
            diff["removed"],
            ["capability network:fetch", "creates a terminal running codex --quiet",
             "fetches from api.example.com"],
        )

    def test_the_update_diff_is_empty_when_nothing_changes_and_absent_when_unreadable(self):
        same = registry.update_diff(self.OLD, self.OLD, POLICY["tiers"])
        self.assertEqual((same["added"], same["removed"]), ([], []))
        self.assertIsNone(registry.update_diff(None, self.NEW, POLICY["tiers"]))
        self.assertIsNone(registry.update_diff("id = [", self.NEW, POLICY["tiers"]))

    def test_the_comment_shows_the_diff_escaped(self):
        diff = registry.update_diff(self.OLD, self.NEW, POLICY["tiers"])
        diff["added"].append({"item": "runs @mallory [click](https://x.example) <b>", "tier": "t2"})
        result = decision(
            entry_before=entry(), last_published=release("1.2.0"), release=release("1.3.0"),
            update_diff=diff,
        )
        body = registry.comment(result)
        self.assertIn("**New in 1.3.0**", body)
        self.assertIn("**Removed:**", body)
        self.assertIn("codex -c notify=\\*", body)
        self.assertNotIn("<b>", body)
        self.assertNotIn("](https://", body)
        self.assertNotIn("@mallory", body)
        # A new extension has no diff; a new version whose manifests were unreadable says so.
        self.assertNotIn("Update diff", registry.comment(decision()))
        unreadable = decision(
            entry_before=entry(), last_published=release("1.2.0"), release=release("1.3.0")
        )
        self.assertIn("Update diff: not available", registry.comment(unreadable))

    def test_decision_input_carries_the_diff(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            before, after = Path(tmp, "before"), Path(tmp, "after")
            for directory in (before, after):
                (directory / "extensions").mkdir(parents=True)
                (directory / "extensions" / "cc.toml").write_text(
                    entry_text(id='"cc"'), encoding="utf-8"
                )
            pr = {"author": {"login": "alice"}, "files": [{"path": "extensions/cc.toml"}],
                  "labels": [], "reviews": []}
            index = {"releases": [{"signed": dict(release("1.2.0"), id="cc")}]}
            new = ('id = "cc"\nversion = "1.3.0"\n[fyx_ui]\n'
                   'capabilities = ["ui:panel", "terminal:pseudo"]\n')
            old = 'id = "cc"\nversion = "1.2.0"\n[fyx_ui]\ncapabilities = ["ui:panel"]\n'
            data = registry.decision_input(pr, None, before, after, new, b"pkg", index, None, None,
                                           POLICY, last_manifest_text=old)
            self.assertEqual(
                data["update_diff"],
                {"version": "1.3.0",
                 "added": [{"item": "capability terminal:pseudo", "tier": "t1"}],
                 "removed": []},
            )
            none = registry.decision_input(pr, None, before, after, new, b"pkg", index, None, None,
                                           POLICY)
            self.assertIsNone(none["update_diff"])


class ConsentWarnings(unittest.TestCase):
    """Review p5-security-0.4: P3-P5 warnings mirrored from the install dialog, P9's rule."""

    MANIFEST = (
        'id = "cc"\nversion = "1.0.0"\n[fyx_ui]\n'
        'capabilities = ["terminal:profiles", "terminal:env", "relay:hooks", "process:run"]\n'
        '[[fyx_ui.processes]]\nname = "git"\nargs = ["status", "--porcelain=v2"]\n'
        '[[fyx_ui.processes]]\nname = "git"\nargs = ["-c", "core.pager=x", "log"]\n'
        '[[fyx_ui.terminal_profiles]]\nid = "friendly"\nname = "Friendly"\nprogram = "bash"\n'
        'args = ["-c", "curl x | sh"]\n'
        '[[fyx_ui.terminal_profiles]]\nid = "tool"\nname = "Tool"\nprogram = "tool"\n'
        'args = ["--plugins", "${package}/p"]\n'
        '[[fyx_ui.terminal_profiles]]\nid = "plain"\nname = "Plain"\nprogram = "codex"\n'
        '[fyx_ui.terminal_env]\nvars = { NO_COLOR = "1", CLAUDE_CODE_PLUGIN_DIRS = "x", '
        'MY_DIR = "${package}/d" }\nreason = "x"\n'
        '[[fyx_ui.relay_channels]]\nname = "hook"\nreason = "x"\n'
    )

    def test_items_that_can_run_code_carry_the_dialogs_warning(self):
        lines = registry.consent_warnings(self.MANIFEST)
        runs = registry.WARNING_RUNS_ANY_CODE
        self.assertEqual(
            lines,
            [
                f"runs git: {runs}",
                f"terminal profile Friendly: bash -c curl x | sh: {runs}",
                f"terminal profile Tool: tool --plugins ${{package}}/p: {runs}",
                "sets CLAUDE_CODE_PLUGIN_DIRS=x in its terminals (makes Claude Code load this "
                f"extension's plugins): {runs}",
                f"sets MY_DIR=${{package}}/d in its terminals (files from this extension): {runs}",
                f"relay channel hook: {runs}; {registry.WARNING_HOOK_ANSWERS}",
            ],
        )
        self.assertEqual(registry.consent_warnings(None), [])
        self.assertEqual(registry.consent_warnings("id = ["), [])

    def test_runs_any_code_matches_the_host(self):
        for name in ("bash", "Python3", "node.exe", "PWSH.EXE", "npx.cmd", "python3.12",
                     "lua-5.4", "uvx"):
            self.assertTrue(registry.runs_any_code(name, None, []), name)
        for name in ("rustfmt", "eslint", "hostname", "pythonic", "clang-format-18", "7z"):
            self.assertFalse(registry.runs_any_code(name, None, []), name)
        self.assertTrue(registry.runs_any_code("x", "tools/python.exe", []))
        self.assertFalse(registry.runs_any_code("git", None, ["-C", "sub", "status", "**"]))
        self.assertTrue(registry.runs_any_code("git", None, ["submodule", "foreach", "**"]))
        self.assertTrue(registry.runs_any_code("git", None, ["--git-dir=tmp/x", "status"]))
        self.assertTrue(registry.runs_any_code("git", None, ["*"]))

    def test_the_comment_shows_the_warnings_escaped(self):
        warnings = registry.consent_warnings(self.MANIFEST)
        body = registry.comment(decision(consent_warnings=warnings))
        self.assertIn("**Install dialog warns:** relay channel hook", body)
        self.assertIn("bash -c curl x \\| sh", body)
        self.assertNotIn("Install dialog warns", registry.comment(decision()))

    def discovery(self, dir_, default_dir) -> str:
        default = f'default_dir = "{default_dir}"\n' if default_dir else ""
        return (
            'id = "cc"\nversion = "1.0.0"\n[fyx_ui]\ncapabilities = ["fs:discovery-file"]\n'
            f'[[fyx_ui.discovery_files]]\nlistener = "ide"\ndir = "{dir_}"\n{default}'
            'name = "${port}.lock"\nreason = "x"\n'
        )

    def test_a_default_dir_outside_the_tools_folder_needs_the_owner(self):
        for dir_, default_dir in (("${account.claude}/ide", "~/.claude/ide"),
                                  ("${account.claude}/ide", "~/.claude"),
                                  ("${tmp}/gemini/ide", None)):
            self.assertEqual(registry.review_flags(self.discovery(dir_, default_dir)), [],
                             default_dir)
        for dir_, default_dir, where in (
            ("${account.claude}/ide", "~/.config/autostart", "outside ~/.claude"),
            ("${account.claude}/ide", "~/.claude-evil/ide", "outside ~/.claude"),
            ("${tmp}/gemini/ide", "~/.gemini/ide", "with no tool folder in dir"),
        ):
            flags = registry.review_flags(self.discovery(dir_, default_dir))
            self.assertEqual(
                flags, [f"discovery file ${{port}}.lock: default_dir {default_dir} is {where}"]
            )
        flag = registry.review_flags(self.discovery("${account.claude}/ide", "~/.config/x"))
        pending = decision(review_flags=flag)
        self.assertEqual(pending["status"], "pending")
        self.assertIn(flag[0], registry.comment(pending).replace("\\", ""))
        approved = decision(review_flags=flag, approvals=["shelenhovskyi"])
        self.assertEqual(approved["status"], "success")


class Decisions(unittest.TestCase):
    def test_a_new_t0_extension_auto_merges(self):
        result = decision()
        self.assertEqual(result["status"], "success")
        self.assertTrue(result["auto_merge"])

    def test_no_reviewer_fails_closed(self):
        result = decision(agent_verdict=None)
        self.assertEqual(result["status"], "pending")
        self.assertFalse(result["auto_merge"])
        self.assertEqual(result["add_labels"], ["needs-owner"])
        self.assertEqual(result["agent_review"], "failure")
        approved = decision(agent_verdict=None, approvals=["shelenhovskyi"])
        self.assertEqual(approved["status"], "success")
        self.assertEqual(approved["agent_review"], "success")

    def test_needs_owner_stays_until_the_owner_approves(self):
        result = decision(labels=["needs-owner"])
        self.assertEqual(result["status"], "pending")
        self.assertEqual(result["agent_review"], "failure")

    def test_a_new_version_needs_an_entry_owner(self):
        before = entry()
        last = release("1.0.0")
        result = decision(
            entry_before=before, last_published=last, release=release("1.1.0"), pr_author="mallory"
        )
        self.assertEqual(result["status"], "pending")
        result = decision(
            entry_before=before,
            last_published=last,
            release=release("1.1.0"),
            pr_author="mallory",
            approvals=["alice"],
        )
        self.assertEqual(result["status"], "success")
        self.assertTrue(result["auto_merge"])

    def test_a_zed_extension_needs_the_owner_and_warns(self):
        flagged = dict(release=release(tier="t2"), zed_extension=["zed/extension.wasm"])
        result = decision(**flagged)
        self.assertEqual(result["status"], "pending")
        self.assertFalse(result["auto_merge"])
        self.assertIn("unrestricted Zed extension", result["required"][-1]["reason"])
        approved = decision(**flagged, approvals=["shelenhovskyi"])
        self.assertEqual(approved["status"], "success")
        self.assertFalse(approved["auto_merge"])
        self.assertIn("**Warning:** this extension runs an unrestricted Zed extension",
                      registry.comment(approved))

    def test_t2_and_urgent_need_the_owner(self):
        result = decision(release=release(tier="t2", capabilities=["network:fetch"]))
        self.assertEqual(result["status"], "pending")
        self.assertFalse(decision(
            release=release(tier="t2"), approvals=["shelenhovskyi"]
        )["auto_merge"])
        urgent = entry()
        urgent["urgent"] = True
        self.assertEqual(decision(entry_after=urgent)["status"], "pending")
        result = decision(entry_after=urgent, approvals=["shelenhovskyi"])
        self.assertEqual(result["status"], "success")
        self.assertFalse(result["auto_merge"])

    def test_owner_changes_need_an_existing_owner(self):
        before = entry()
        after = copy.deepcopy(before)
        after["owners"] = ["alice", "mallory"]
        result = decision(
            entry_before=before,
            entry_after=after,
            last_published=release("1.0.0"),
            release=release("1.1.0"),
            pr_author="mallory",
        )
        self.assertEqual(result["status"], "pending")
        self.assertEqual(result["changes"]["owners_added"], ["mallory"])

    def test_refusals(self):
        self.assertEqual(decision(entry_after=None)["status"], "failure")
        self.assertEqual(
            decision(
                entry_before=entry(), last_published=release("1.0.0"), release=release("1.0.0")
            )["status"],
            "failure",
        )
        self.assertEqual(decision(ci={"entry": True, "build": False})["status"], "failure")
        self.assertEqual(
            decision(changed_files=["extensions/todo.toml", "extensions/lint.toml"])["status"],
            "failure",
        )
        self.assertEqual(
            decision(changed_files=["extensions/todo.toml", "README.md"])["status"], "failure"
        )
        self.assertEqual(decision(blocked_ids=["todo"])["status"], "failure")
        # Approving does not make a refused PR mergeable.
        self.assertEqual(
            decision(entry_after=None, approvals=["shelenhovskyi"])["status"], "failure"
        )

    def test_reserved_ids_need_the_owner(self):
        reserved = entry(id='"fyx-todo"', owners='["shelenhovskyi"]')
        self.assertEqual(decision(entry_after=reserved)["status"], "pending")
        self.assertEqual(
            decision(entry_after=reserved, pr_author="shelenhovskyi")["status"], "success"
        )

    def test_other_prs_are_left_to_code_owners(self):
        result = decision(changed_files=["policy/builder.toml"])
        self.assertEqual(result["kind"], "other")
        self.assertFalse(result["auto_merge"])

    def test_approvals_use_the_latest_review(self):
        reviews = [
            {"author": {"login": "alice"}, "state": "APPROVED"},
            {"author": {"login": "alice"}, "state": "COMMENTED"},
            {"author": {"login": "bob"}, "state": "APPROVED"},
            {"author": {"login": "bob"}, "state": "CHANGES_REQUESTED"},
        ]
        self.assertEqual(registry.approvals(reviews), ["alice"])

    def test_ci_results(self):
        jobs = {"jobs": [
            {"name": "entry (a)", "conclusion": "success"},
            {"name": "build (a)", "conclusion": "failure"},
            {"name": "changes", "conclusion": "success"},
        ]}
        self.assertEqual(registry.ci_results(jobs), {"entry": True, "build": False})
        self.assertEqual(registry.ci_results(None), {"entry": False, "build": False})


class Verdicts(unittest.TestCase):
    def test_only_a_valid_pass_passes(self):
        good = json.dumps(pass_verdict())
        self.assertEqual(registry.checked_verdict(good, "x")["verdict"], "pass")
        for raw, size in [
            (None, 0),
            ("not json", 0),
            (json.dumps({"verdict": "pass"}), 0),
            (json.dumps({**pass_verdict(), "inputs_sha256": "other"}), 0),
            (json.dumps({**pass_verdict(), "verdict": "approve"}), 0),
            (json.dumps({**pass_verdict(), "reasons": [{"category": "nope", "file": "", "detail": ""}]}), 0),
            (good, registry.MAX_REVIEW_INPUT_BYTES + 1),
        ]:
            with self.subTest(raw=raw, size=size):
                self.assertEqual(registry.checked_verdict(raw, "x", size)["verdict"], "concern")
        self.assertEqual(
            registry.checked_verdict(None, "x")["reasons"][0]["detail"], "no reviewer configured"
        )


class Comments(unittest.TestCase):
    def test_reasons_cannot_ping_link_or_inject_html(self):
        text = registry.escape("@owner see https://evil.example <img src=x> [x](y)\nsecond")
        self.assertNotIn("@owner", text)
        self.assertNotIn("https://", text)
        self.assertNotIn("<img", text)
        self.assertNotIn("\n", text)
        verdict = {
            "verdict": "concern",
            "reasons": [{"category": "other", "file": "a.rs", "detail": "@alice http://x <b>"}],
            "inputs_sha256": "x",
        }
        body = registry.comment(decision(agent_verdict=verdict))
        self.assertTrue(body.startswith("<!-- fyx-registry-policy -->"))
        self.assertNotIn("@alice", body)
        self.assertNotIn("<b>", body)


class BlockLists(unittest.TestCase):
    def test_toml_to_unsigned(self):
        text = '[[entry]]\nid = "todo"\nversions = "*"\nreason = "Bad."\nurgent = true\n'
        unsigned = registry.unsigned_blocklist(text, 3, 100)
        self.assertEqual(unsigned["sequence"], 3)
        self.assertEqual(unsigned["expires_at"], 100 + 48 * 3600)
        self.assertEqual(unsigned["entries"][0]["versions"], "*")
        with self.assertRaises(registry.Refused):
            registry.unsigned_blocklist(text.replace("urgent", "extra"), 1, 1)
        template = (registry.POLICY_DIR.parent / "blocklist" / "blocklist.toml").read_text()
        self.assertEqual(registry.unsigned_blocklist(template, 1, 1)["entries"], [])

    def test_revocations_skip_block_list_keys(self):
        keyset = {"keys": [
            {"key_id": "rel", "roles": ["release"]},
            {"key_id": "bl", "roles": ["blocklist"]},
            {"key_id": "rot", "roles": ["snapshot", "blocklist-revocation"]},
        ]}
        snapshot = {"signed": {"revoked_keys": ["rel", "bl", "rot", "gone"]}}
        self.assertEqual(registry.carried_revocations(snapshot, keyset), ["gone", "rel"])


class Packages(unittest.TestCase):
    def test_package_layout_is_reproducible(self):
        import tempfile
        import zipfile

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "ext" / "assets").mkdir(parents=True)
            (root / "ext" / "extension.toml").write_text(
                'id = "x"\n[fyx_ui]\ncomponent = "fyx_ui.wasm"\n'
            )
            (root / "ext" / "fyx_ui.wasm").write_bytes(b"\0asm")
            (root / "ext" / "assets" / "logo.svg").write_text("<svg/>")
            registry.package_dir(root / "ext", root / "a.fyxext")
            registry.package_dir(root / "ext", root / "b.fyxext")
            self.assertEqual((root / "a.fyxext").read_bytes(), (root / "b.fyxext").read_bytes())
            with zipfile.ZipFile(root / "a.fyxext") as archive:
                self.assertEqual(
                    archive.namelist(), ["extension.toml", "fyx_ui.wasm", "assets/logo.svg"]
                )

    def test_a_zed_extension_makes_the_release_t2(self):
        import io
        import zipfile

        manifest = 'id = "todo"\nversion = "1.0.0"\n[fyx_ui]\ncapabilities = ["ui:panel"]\n'

        def package(*names: str) -> bytes:
            buffer = io.BytesIO()
            with zipfile.ZipFile(buffer, "w") as archive:
                for name in names:
                    archive.writestr(name, b"\0asm")
            return buffer.getvalue()

        plain = registry.derive_release(
            entry(), manifest, package("extension.toml", "fyx_ui.wasm"), POLICY
        )
        self.assertEqual(plain["tier"], "t0")
        self.assertNotIn("zed_extension", plain)
        for name in ["extension.wasm", "zed/Extension.WASM", "a\\extension.wasm", "x/extension.wasm. "]:
            with self.subTest(name):
                derived = registry.derive_release(
                    entry(), manifest, package("extension.toml", "fyx_ui.wasm", name), POLICY
                )
                self.assertEqual(derived["tier"], "t2")
                self.assertIs(derived["zed_extension"], True)
        # The trusted decision has no package, only the source file list.
        from_source = registry.derive_release(
            entry(), manifest, b"", POLICY, ["src/lib.rs", "zed/extension.wasm"]
        )
        self.assertEqual(from_source["tier"], "t2")
        self.assertEqual(
            registry.zed_extension_files(b"", ["src/lib.rs", "zed/extension.wasm"]),
            ["zed/extension.wasm"],
        )


def zip_of(*names: str) -> bytes:
    import io
    import zipfile

    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        for name in names:
            archive.writestr(name, b"\0asm")
    return buffer.getvalue()


class BuildOnlyZedExtensions(unittest.TestCase):
    """integration-6 N3: an `extension.wasm` that only the build makes must not pass as T0."""

    BUILT = 'id = "todo"\nversion = "1.0.0"\n[fyx_ui]\ncomponent = "extension.wasm"\ncapabilities = ["ui:panel"]\n'
    PLAIN = 'id = "todo"\nversion = "1.0.0"\n[fyx_ui]\ncomponent = "fyx_ui.wasm"\ncapabilities = ["ui:panel"]\n'

    def test_the_decision_reads_the_component(self):
        # A gitignored component: in no source file list, only named by extension.toml.
        for component in ["extension.wasm", "target/wasm32-wasip2/release/Extension.WASM. "]:
            with self.subTest(component):
                manifest = self.BUILT.replace("extension.wasm", component)
                self.assertEqual(registry.manifest_component(manifest), [component])
                derived = registry.derive_release(entry(), manifest, b"", POLICY, ["src/lib.rs"])
                self.assertEqual(derived["tier"], "t2")
                self.assertIs(derived["zed_extension"], True)
        self.assertEqual(registry.manifest_component(self.PLAIN), ["fyx_ui.wasm"])
        self.assertEqual(registry.manifest_component("not toml ["), [])
        self.assertEqual(registry.manifest_component(None), [])

    def test_decision_input_flags_the_component(self):
        import tempfile

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "after" / "extensions").mkdir(parents=True)
            (root / "after" / "extensions" / "todo.toml").write_text(entry_text())
            data = registry.decision_input(
                {"author": {"login": "alice"}, "files": [{"path": "extensions/todo.toml"}]},
                None, root / "before", root / "after", self.BUILT, None, None, None, None,
                POLICY, ["src/lib.rs", "Cargo.toml"],
            )
        self.assertEqual(data["zed_extension"], ["extension.wasm"])
        self.assertEqual(data["release"]["tier"], "t2")
        result = registry.decide(data)
        self.assertFalse(result["auto_merge"])
        self.assertFalse(result["owner_approved"])
        self.assertEqual(
            registry.tier_status(result), "tier=t2 zed_extension=true owner_approved=false"
        )

    def test_pr_build_fails_on_a_build_only_extension_wasm(self):
        package = zip_of("extension.toml", "fyx_ui.wasm", "assets/extension.wasm")
        build_only = registry.zed_check(package, self.PLAIN, ["extension.toml", "src/lib.rs"])
        self.assertTrue(build_only["build_only"])
        self.assertEqual(build_only["package"], ["assets/extension.wasm"])
        # Seen by the decision through the source list or the component: a warning, not a failure.
        for manifest, files in [
            (self.PLAIN, ["assets/extension.wasm"]),
            (self.BUILT, ["src/lib.rs"]),
        ]:
            with self.subTest(files):
                self.assertFalse(registry.zed_check(package, manifest, files)["build_only"])
        plain = registry.zed_check(zip_of("extension.toml", "fyx_ui.wasm"), self.PLAIN, [])
        self.assertEqual(plain, {"package": [], "trusted": [], "build_only": False})

    def test_zed_check_exit_code(self):
        import contextlib
        import io
        import tempfile

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "p.fyxext").write_bytes(zip_of("extension.toml", "extension.wasm"))
            (root / "extension.toml").write_text(self.PLAIN)
            (root / "files.txt").write_text("src/lib.rs\n")
            args = ["zed-check", "--package", str(root / "p.fyxext"),
                    "--manifest", str(root / "extension.toml"),
                    "--source-files", str(root / "files.txt")]
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(registry.main(args), 1)
                (root / "files.txt").write_text("src/lib.rs\nextension.wasm\n")
                self.assertEqual(registry.main(args), 0)


class PublishChecks(unittest.TestCase):
    """integration-6 N3: publish.yml signs a T2 or Zed-extension release only after a T2
    decision with the owner's approval."""

    OWNER = "shelenhovskyi"
    COMMIT = "c" * 40

    def pr(self, author="alice", reviews=None, merged=None):
        return {
            "number": 7,
            "author": {"login": author},
            "reviews": reviews if reviews is not None else [
                {"author": {"login": self.OWNER}, "state": "APPROVED"}
            ],
            "mergeCommit": {"oid": merged or self.COMMIT},
        }

    def status(self, description, login="github-actions[bot]"):
        return {"context": "policy-tier", "description": description, "creator": {"login": login}}

    def check(self, release_fields, pr="default", statuses=None):
        rel = dict(release(tier=release_fields.pop("tier", "t0")), **release_fields)
        if pr == "default":
            pr = self.pr()
        return registry.publish_check(rel, self.COMMIT, pr, statuses, self.OWNER)

    APPROVED_ZED = "tier=t2 zed_extension=true owner_approved=true"

    def test_t0_and_t1_need_nothing(self):
        self.assertEqual(self.check({"tier": "t0"}, pr=None), [])
        self.assertEqual(self.check({"tier": "t1"}, pr=None), [])

    def test_an_approved_t2_decision_publishes(self):
        zed = {"tier": "t2", "zed_extension": True}
        self.assertEqual(self.check(dict(zed), statuses=[self.status(self.APPROVED_ZED)]), [])
        # The owner's own PR counts as the approval.
        self.assertEqual(
            self.check(dict(zed), pr=self.pr(author=self.OWNER, reviews=[]),
                       statuses=[self.status(self.APPROVED_ZED)]),
            [],
        )

    def test_a_build_only_zed_extension_rated_t0_is_refused(self):
        errors = self.check(
            {"tier": "t2", "zed_extension": True},
            statuses=[self.status("tier=t0 zed_extension=false owner_approved=false")],
        )
        self.assertTrue(any("rated it t0" in e for e in errors), errors)
        self.assertTrue(any("did not see" in e for e in errors), errors)
        self.assertTrue(any("owner's approval" in e for e in errors), errors)
        # zed_extension alone is enough, whatever the tier field says.
        self.assertNotEqual(self.check({"tier": "t0", "zed_extension": True}, statuses=[]), [])

    def test_refusals(self):
        zed = {"tier": "t2", "zed_extension": True}
        cases = {
            "no PR": dict(pr=None, statuses=[self.status(self.APPROVED_ZED)]),
            "no status": dict(statuses=[]),
            "a status from someone else": dict(
                statuses=[self.status(self.APPROVED_ZED, login="mallory")]
            ),
            "the latest status is not approved": dict(statuses=[
                self.status("tier=t2 zed_extension=true owner_approved=false"),
                self.status(self.APPROVED_ZED),
            ]),
            "the decision missed the Zed extension": dict(
                statuses=[self.status("tier=t2 zed_extension=false owner_approved=true")]
            ),
            "the owner's approval was withdrawn": dict(
                pr=self.pr(reviews=[
                    {"author": {"login": self.OWNER}, "state": "APPROVED"},
                    {"author": {"login": self.OWNER}, "state": "DISMISSED"},
                ]),
                statuses=[self.status(self.APPROVED_ZED)],
            ),
            "a different merge commit": dict(
                pr=self.pr(merged="d" * 40), statuses=[self.status(self.APPROVED_ZED)]
            ),
        }
        for name, kwargs in cases.items():
            with self.subTest(name):
                self.assertNotEqual(self.check(dict(zed), **kwargs), [])

    def test_a_plain_t2_needs_no_zed_flag(self):
        self.assertEqual(
            self.check({"tier": "t2"},
                       statuses=[self.status("tier=t2 zed_extension=false owner_approved=true")]),
            [],
        )

    def test_publish_check_command_refuses(self):
        import contextlib
        import io
        import tempfile

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "release.json").write_text(
                json.dumps(dict(release(tier="t2"), zed_extension=True))
            )
            (root / "pr.json").write_text(json.dumps(self.pr()))
            (root / "statuses.json").write_text(
                json.dumps([self.status("tier=t0 zed_extension=false owner_approved=false")])
            )
            args = ["publish-check", str(root / "release.json"), "--commit", self.COMMIT,
                    "--pr", str(root / "pr.json"), "--statuses", str(root / "statuses.json")]
            with contextlib.redirect_stdout(io.StringIO()), \
                    contextlib.redirect_stderr(io.StringIO()) as err:
                self.assertEqual(registry.main(args), 2)
            self.assertIn("rated it t0", err.getvalue())
            (root / "statuses.json").write_text(json.dumps([self.status(self.APPROVED_ZED)]))
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(registry.main(args), 0)
            # No PR found (an empty file): refused.
            (root / "pr.json").write_text("")
            with contextlib.redirect_stdout(io.StringIO()), \
                    contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(registry.main(args), 2)


if __name__ == "__main__":
    unittest.main()
