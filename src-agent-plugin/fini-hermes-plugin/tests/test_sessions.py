"""Run with the installed Hermes runtime: hermes --run-module unittest discover -s tests -v."""

from concurrent.futures import ThreadPoolExecutor
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

import hermes_constants
from hermes_cli.plugin_command_dispatch import invoke_plugin_command
from hermes_cli.plugins import PluginContext, PluginManager, PluginManifest

SOURCE = Path(__file__).resolve().parents[1] / "__init__.py"
spec = importlib.util.spec_from_file_location("fini_test_plugin", SOURCE)
assert spec is not None and spec.loader is not None
plugin = importlib.util.module_from_spec(spec)
spec.loader.exec_module(plugin)


class SessionCommandsTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="fini-tests-", dir=os.environ["TMPDIR"])
        self.addCleanup(self.temporary.cleanup)
        self.home = Path(self.temporary.name) / "profile-a"
        self.home.mkdir()
        environment = patch.dict(os.environ, {"HERMES_HOME": str(self.home)})
        environment.start()
        self.addCleanup(environment.stop)
        # Never inherit an operator's explicit live shared-store selection.
        for key in ("FINI_SESSIONS_PATH", "HERMES_PROFILE", "HERMES_DATA_DIR_SUFFIX"):
            os.environ.pop(key, None)
        self.manager = PluginManager()
        plugin.register(PluginContext(
            PluginManifest(name=plugin.PLUGIN_ID, key=plugin.PLUGIN_ID, source="user"),
            self.manager,
        ))
        self.addCleanup(self.manager.unload)
        # Inspect registrations in the real manager, not a substitute plugin context.
        self.commands = self.manager._plugin_commands
        self.path = self.home / "plugin-data" / plugin.PLUGIN_ID / "sessions.json"

    def call(self, command, quest, session_id=None):
        return json.loads(invoke_plugin_command(
            self.commands[command]["handler"], quest,
            session_id=session_id, session_key=session_id, source=None, platform="cli",
        ))

    def tool(self, args, session_id=None, **context):
        # Actual installed model->registry dispatch: context travels separately
        # from model args, just as agent/tool_executor.py supplies it per call.
        from model_tools import handle_function_call
        return json.loads(handle_function_call(
            "fini_link_current_session", args, session_id=session_id, **context,
        ))

    def test_tool_dispatch_runtime_identity_and_literal_quest(self):
        from tools.registry import registry
        entry = registry.get_entry("fini_link_current_session")
        self.assertIsNotNone(entry)
        assert entry is not None
        self.assertEqual(set(entry.schema["parameters"]["properties"]), {"quest_id"})
        self.assertFalse(self.path.parent.exists())
        quest = '  001 / literal "quest"  '
        with patch.dict(os.environ, {"HERMES_SESSION_ID": "wrong-inherited-parent"}):
            for sid in ("root-fixture", "delegated-child-fixture", "resumed-fixture"):
                result = self.tool({"quest_id": quest}, sid, task_id="not-session",
                                   conversation_id="not-the-calling-session")
                self.assertEqual(result["session_id"], sid)
                self.assertEqual(result["quest_id"], quest)
            before = self.path.read_bytes()
            self.assertFalse(self.tool({"quest_id": quest}, "root-fixture")["added"])
            self.assertEqual(self.path.read_bytes(), before)
        self.assertEqual(self.call("fini-sessions", quest)["session_ids"],
                         ["root-fixture", "delegated-child-fixture", "resumed-fixture"])

    def test_tool_dispatch_missing_identity_and_spoofing_do_not_write(self):
        with patch.dict(os.environ, {"HERMES_SESSION_ID": "wrong-inherited-parent"}):
            for sid in (None, "", " "):
                self.assertIn("error", self.tool({"quest_id": "q"}, sid))
            for args in ({}, {"quest_id": ""}, {"quest_id": 42},
                         {"quest_id": "q", "session_id": "forged"},
                         {"quest_id": "q", "conversation_id": "forged"}):
                self.assertIn("error", self.tool(args, "real-fixture"))
        self.assertFalse(self.path.parent.exists())
        self.call("fini-link", "existing", "original-fixture")
        before = self.path.read_bytes()
        self.assertIn("error", self.tool({"quest_id": "q"}))
        self.assertEqual(self.path.read_bytes(), before)

    def test_tool_dispatch_concurrent_sessions_do_not_capture_registration_identity(self):
        from threading import Barrier
        # All calls share one registered handler and one store; each dispatch
        # owns different runtime kwargs despite a conflicting process env ID.
        barrier = Barrier(6)
        def link(index):
            barrier.wait(timeout=10)
            sid = f"concurrent-fixture-{index}"
            result = self.tool({"quest_id": f"q-{index}"}, sid)
            self.assertEqual(result["session_id"], sid)
        with patch.dict(os.environ, {"HERMES_SESSION_ID": "wrong-inherited-parent"}):
            with ThreadPoolExecutor(max_workers=6) as pool:
                list(pool.map(link, range(6)))
        state = json.loads(self.path.read_text())["quests"]
        self.assertEqual(state, {f"q-{i}": {"hermes": [f"concurrent-fixture-{i}"]}
                                 for i in range(6)})

    def test_installed_symlink_dispatch_uses_exact_shared_entrypoint(self):
        plugins = self.home / "plugins"
        plugins.mkdir()
        link = plugins / plugin.PLUGIN_ID
        link.symlink_to(SOURCE.parent)
        spec = importlib.util.spec_from_file_location("fini_installed_smoke", link / "__init__.py")
        self.assertIsNotNone(spec)
        assert spec is not None and spec.loader is not None
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        self.assertEqual(module.CORE, SOURCE.parents[1] / "shared" / "cli.mts")
        manager = PluginManager()
        self.addCleanup(manager.unload)
        module.register(PluginContext(
            PluginManifest(name=module.PLUGIN_ID, key=module.PLUGIN_ID, source="user"), manager,
        ))
        handler = manager._plugin_commands["fini-link"]["handler"]
        result = json.loads(invoke_plugin_command(handler, "installed-smoke", session_id="runtime-owned-smoke"))
        self.assertEqual(result["session_id"], "runtime-owned-smoke")
        self.assertEqual(result["state_path"], str(self.path))
        self.assertEqual(json.loads(self.path.read_text())["quests"]["installed-smoke"],
                         {"hermes": ["runtime-owned-smoke"]})

    def test_bridge_bounded_request_and_invalid_core_output(self):
        with patch.object(hermes_constants, "get_hermes_home", side_effect=FileNotFoundError("missing profile")):
            self.assertIn("error", self.call("fini-link", "001", "fixture-a"))
        self.assertIn("error", self.call("fini-link", "x" * plugin.MAX_BYTES, "fixture-a"))
        with patch.object(plugin, "CORE", self.home / "missing-core.mts"):
            self.assertIn("error", self.call("fini-link", "001", "fixture-a"))
        fake = self.home / "invalid.mts"
        fake.write_text('process.stdout.write("[]");')
        with patch.object(plugin, "CORE", fake):
            self.assertIn("error", self.call("fini-link", "001", "fixture-a"))
        fake.write_text('process.stdout.write("x".repeat(2 * 1024 * 1024));')
        with patch.object(plugin, "CORE", fake):
            self.assertIn("error", self.call("fini-link", "001", "fixture-a"))
        self.assertFalse(self.path.exists())

    def test_real_bridge_deadline_kills_hung_core(self):
        fake = self.home / "hang.mts"
        fake.write_text('setInterval(() => {}, 1000);')
        started = time.monotonic()
        with patch.object(plugin, "CORE", fake):
            result = self.call("fini-link", "001", "fixture-a")
        self.assertIn("timed out", result["error"])
        self.assertLess(time.monotonic() - started, 20)
        self.assertFalse(self.path.exists())

    def test_runtime_registration_and_many_sessions_deduplicated(self):
        self.assertEqual(set(self.commands), {"fini-link", "fini-sessions", "fini-link-claude"})
        self.assertFalse(self.path.parent.exists())  # Registration is side-effect free.
        for sid in ("fixture-a", "fixture-b"):
            self.assertTrue(self.call("fini-link", "001", sid)["added"])
        before = self.path.stat().st_mtime_ns
        self.assertFalse(self.call("fini-link", "001", "fixture-a")["added"])
        self.assertEqual(self.path.stat().st_mtime_ns, before)
        self.assertEqual(self.call("fini-sessions", "001")["session_ids"], ["fixture-a", "fixture-b"])
        self.assertEqual(json.loads(self.path.read_text())["quests"]["001"], {"hermes": ["fixture-a", "fixture-b"]})
        self.assertEqual(self.path.stat().st_mode & 0o777, 0o600)

    def test_literal_quest_identifier_is_never_normalized(self):
        quest = '  0001 / 漢字 "literal" --session-id=not-an-id  '
        result = self.call("fini-link", quest, "actual-context-fixture")
        self.assertEqual(result["quest_id"], quest)
        self.assertEqual(self.call("fini-sessions", quest)["session_ids"], ["actual-context-fixture"])
        self.assertEqual(self.call("fini-sessions", quest.strip())["session_ids"], [])
        self.assertEqual(set(json.loads(self.path.read_text())["quests"]), {quest})

    def test_missing_context_and_empty_quest_fail_without_writes(self):
        for sid in (None, "", " "):
            self.assertIn("error", self.call("fini-link", "001", sid))
        for quest in ("", "   "):
            self.assertIn("error", self.call("fini-link", quest, "fixture-a"))
        self.assertFalse(self.path.parent.exists())

    def test_unknown_quest_and_multiple_quests(self):
        self.assertEqual(self.call("fini-sessions", "unknown")["session_ids"], [])
        self.assertFalse(self.path.exists())
        for quest in ("a", "b"):
            self.call("fini-link", quest, "fixture-a")
        self.assertEqual(set(json.loads(self.path.read_text())["quests"]), {"a", "b"})

    def test_native_claude_and_hermes_share_default_and_explicit_store(self):
        bridge = SOURCE.parents[1] / "fini-claude-plugin" / "bridge.mts"
        quest = '  001 / literal "quest"  '
        ids = ["12345678-1234-1234-1234-123456789abc",
               "87654321-1234-1234-1234-123456789abc"]

        def claude(sid):
            result = subprocess.run(
                ["node", str(bridge)], input=json.dumps({
                    "hook_event_name": "UserPromptExpansion", "expansion_type": "slash_command",
                    "command_name": "fini:link", "command_source": "plugin",
                    "command_args": quest, "session_id": sid,
                }), text=True, capture_output=True, timeout=20, check=True,
            )
            output = json.loads(result.stdout)
            self.assertEqual(output["decision"], "block")
            return json.loads(output["reason"])

        for override in (None, str(self.home / "common" / "sessions.json")):
            env = {"FINI_SESSIONS_PATH": override} if override else {}
            with patch.dict(os.environ, env):
                with ThreadPoolExecutor(max_workers=4) as pool:
                    futures = [pool.submit(claude, sid) for sid in ids]
                    futures += [pool.submit(self.call, "fini-link", quest, sid)
                                for sid in ("hermes-a", "hermes-b")]
                    for future in futures:
                        self.assertNotIn("error", future.result())
                path = override or str(self.path)
                before = Path(path).read_bytes()
                self.assertFalse(claude(ids[0])["added"])
                self.assertEqual(Path(path).read_bytes(), before)
                result = self.call("fini-sessions", quest)
                self.assertEqual(result["state_path"], path)
                self.assertEqual({(s["provider"], s["session_id"]) for s in result["sessions"]},
                                 {("hermes", "hermes-a"), ("hermes", "hermes-b"),
                                  ("claude", ids[0]), ("claude", ids[1])})
        with patch.dict(os.environ, {"FINI_SESSIONS_PATH": "relative.json"}):
            self.assertIn("error", self.call("fini-link", quest, "no-write"))
        self.assertEqual(set(json.loads(self.path.read_text())["quests"][quest]["hermes"]),
                         {"hermes-a", "hermes-b"})

    def test_profiles_are_isolated(self):
        self.call("fini-link", "001", "profile-a-fixture")
        other = Path(self.temporary.name) / "profile-b"
        other.mkdir()
        with patch.dict(os.environ, {"HERMES_HOME": str(other)}):
            self.assertEqual(self.call("fini-sessions", "001")["session_ids"], [])
            self.call("fini-link", "001", "profile-b-fixture")
        self.assertEqual(self.call("fini-sessions", "001")["session_ids"], ["profile-a-fixture"])

    def test_corrupt_or_unsupported_state_is_preserved(self):
        self.path.parent.mkdir(parents=True)
        for text in ('{broken', '[]', '{"version":3,"quests":{}}', '{"version":1,"quests":{"q":[4]}}',
                     '{"version":2,"quests":{"q":{"other":["s"]}}}',
                     '{"version":2,"quests":{"q":{"claude":["bad-uuid"]}}}',
                     '{"version":2,"quests":{"q":["s"]}}'):
            self.path.write_text(text)
            self.assertIn("error", self.call("fini-link", "001", "fixture-a"))
            self.assertIn("error", self.call("fini-sessions", "001"))
            self.assertEqual(self.path.read_text(), text)

    def test_bridge_missing_runtime_returns_json_without_state(self):
        with patch.dict(os.environ, {"PATH": str(self.home)}):
            self.assertIn("error", self.call("fini-link", "001", "fixture-a"))
        self.assertFalse(self.path.exists())

    def test_concurrent_threads(self):
        def link(index):
            result = self.call("fini-link", "threads", f"thread-fixture-{index % 20}")
            self.assertNotIn("error", result)
        with ThreadPoolExecutor(max_workers=8) as pool:
            list(pool.map(link, range(80)))
        self.assertEqual(len(self.call("fini-sessions", "threads")["session_ids"]), 20)

    def claude(self, quest, sid: object = "60eba7ee-1dd7-48fb-969d-e68a263ed0a0"):
        return self.call("fini-link-claude", json.dumps({"quest_id": quest, "session_id": sid}))

    def test_v1_migration_read_only_then_persistent_write_preserves_all(self):
        self.path.parent.mkdir(parents=True)
        original = json.dumps({"version": 1, "quests": {"001": ["old-a", "old-b"], "other": ["old-c"]}})
        self.path.write_text(original)
        self.assertEqual(self.call("fini-sessions", "001")["session_ids"], ["old-a", "old-b"])
        self.assertEqual(self.path.read_text(), original)
        self.assertTrue(self.claude("001")["added"])
        state = json.loads(self.path.read_text())
        self.assertEqual(state["version"], 2)
        self.assertEqual(state["quests"]["other"], {"hermes": ["old-c"]})
        self.assertEqual(state["quests"]["001"]["hermes"], ["old-a", "old-b"])
        self.assertEqual(len(self.call("fini-sessions", "001")["sessions"]), 3)

    def test_v1_duplicate_link_also_migrates(self):
        self.path.parent.mkdir(parents=True)
        self.path.write_text(json.dumps({"version": 1, "quests": {"001": ["existing"]}}))
        self.assertFalse(self.call("fini-link", "001", "existing")["added"])
        self.assertEqual(json.loads(self.path.read_text())["version"], 2)

    def test_claude_mixed_providers_dedup_and_literal_ids(self):
        sid = "60eba7ee-1dd7-48fb-969d-e68a263ed0a0"
        quest = '  literal "quest"  '
        self.assertTrue(self.claude(quest, sid)["added"])
        self.assertTrue(self.call("fini-link", quest, sid)["added"])
        before = self.path.read_bytes()
        self.assertFalse(self.claude(quest, sid)["added"])
        self.assertEqual(self.path.read_bytes(), before)
        result = self.call("fini-sessions", quest)
        self.assertEqual(result["sessions"], [
            {"provider": "hermes", "session_id": sid},
            {"provider": "claude", "session_id": sid},
        ])
        other = Path(self.temporary.name) / "other-profile"
        other.mkdir()
        with patch.dict(os.environ, {"HERMES_HOME": str(other)}):
            self.assertEqual(self.call("fini-sessions", quest)["sessions"], [])
        self.assertEqual(json.loads(self.path.read_text())["quests"][quest], {"hermes": [sid], "claude": [sid]})

    def test_invalid_claude_input_never_writes(self):
        for sid in ("bad", " 60eba7ee-1dd7-48fb-969d-e68a263ed0a0", None, 1):
            self.assertIn("error", self.claude("001", sid))
        for raw in ('not-json', '[]', '{}', '{"quest_id":"001","session_id":"bad","extra":1}'):
            self.assertIn("error", self.call("fini-link-claude", raw))
        self.assertFalse(self.path.parent.exists())

    def test_concurrent_mixed_providers_during_migration(self):
        self.path.parent.mkdir(parents=True)
        self.path.write_text(json.dumps({"version": 1, "quests": {"mixed": ["original"]}}))
        def link(index):
            sid = f"00000000-0000-0000-0000-{index % 12:012x}"
            result = self.claude("mixed", sid) if index % 2 else self.call("fini-link", "mixed", sid)
            self.assertNotIn("error", result)
        with ThreadPoolExecutor(max_workers=8) as pool:
            list(pool.map(link, range(96)))
        state = json.loads(self.path.read_text())
        self.assertEqual(state["version"], 2)
        self.assertEqual(len(state["quests"]["mixed"]["claude"]), 6)
        self.assertEqual(len(state["quests"]["mixed"]["hermes"]), 7)
        self.assertIn("original", state["quests"]["mixed"]["hermes"])

    def test_concurrent_processes(self):
        bootstrap = str(Path(hermes_constants.__file__).resolve().parent)
        code = '''
import importlib.util, json, sys
sys.path.insert(0, sys.argv[1])
import hermes_bootstrap
from hermes_cli.plugin_command_dispatch import invoke_plugin_command
spec = importlib.util.spec_from_file_location("fini_worker", sys.argv[2])
plugin = importlib.util.module_from_spec(spec)
spec.loader.exec_module(plugin)
for i in range(20):
    result = json.loads(invoke_plugin_command(plugin.link_session, "processes", session_id=f"process-fixture-{sys.argv[3]}-{i % 10}"))
    assert "error" not in result, result
'''
        workers = [subprocess.Popen(
            [sys.executable, "-I", "-c", code, bootstrap, str(SOURCE), str(index)],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        ) for index in range(4)]
        try:
            for worker in workers:
                out, err = worker.communicate(timeout=60)
                self.assertEqual(worker.returncode, 0, out + err)
        finally:
            for worker in workers:
                if worker.poll() is None:
                    worker.kill()
                    worker.communicate()
        result = self.call("fini-sessions", "processes")["session_ids"]
        self.assertEqual(len(result), 40)
        self.assertEqual(len(set(result)), 40)


if __name__ == "__main__":
    unittest.main()
