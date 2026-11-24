"""Native Hermes command registration and dispatch through the shared library."""
from concurrent.futures import ThreadPoolExecutor
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from hermes_cli.plugin_command_dispatch import invoke_plugin_command
from hermes_cli.plugins import PluginContext, PluginManager, PluginManifest

SOURCE = Path(__file__).resolve().parents[1] / "__init__.py"
spec = importlib.util.spec_from_file_location("fini_test_plugin", SOURCE)
assert spec is not None and spec.loader is not None
plugin = importlib.util.module_from_spec(spec)
spec.loader.exec_module(plugin)


class SessionCommandsTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="fini-hermes-", dir=os.environ["TMPDIR"])
        self.addCleanup(temporary.cleanup)
        self.home = Path(temporary.name)
        environment = patch.dict(os.environ, {
            "HOME": str(self.home), "HERMES_HOME": str(self.home / "profile-a"),
            "XDG_DATA_HOME": str(self.home / "data"), "XDG_CONFIG_HOME": str(self.home / "config"),
        })
        environment.start()
        self.addCleanup(environment.stop)
        os.environ.pop("FINI_AGENT_SESSIONS_DB", None)
        result = subprocess.run(["fini", "quest", "create", "--title", "Hermes check", "--json"],
                                capture_output=True, text=True, check=True)
        self.quest = json.loads(result.stdout)["id"]
        self.manager = PluginManager()
        self.addCleanup(self.manager.unload)
        plugin.register(PluginContext(
            PluginManifest(name=plugin.PLUGIN_ID, key=plugin.PLUGIN_ID, source="user"), self.manager,
        ))
        self.commands = self.manager._plugin_commands
        self.path = self.home / ".fini" / "agent-sessions.db"

    def call(self, name, args="", sid: str | None = "runtime-a"):
        return json.loads(invoke_plugin_command(
            self.commands[f"fini:{name}"]["handler"], args,
            session_id=sid, session_key="not-the-session", platform="cli", source=None,
        ))

    def test_native_commands_and_runtime_identity(self):
        self.assertEqual(set(self.commands), {"fini:link", "fini:unlink", "fini:primary", "fini:relations"})
        from tools.registry import registry
        self.assertIsNone(registry.get_entry("fini_link_current_session"))
        self.assertFalse(self.path.exists())
        with patch.dict(os.environ, {"HERMES_SESSION_ID": "wrong-parent"}):
            for sid in ("runtime-a", "delegated-child", "resumed"):
                result = self.call("link", self.quest, sid)
                self.assertNotIn("error", result)
                self.assertEqual(result["session_id"], sid)
        self.assertEqual(self.call("relations")["quests"][0]["quest_id"], self.quest)
        self.assertEqual(self.call("primary", self.quest, "resumed")["agents"][0]["primary"], "resumed")
        self.call("unlink", self.quest, "resumed")
        group = self.call("relations", f"--quest {self.quest}")["agents"][0]
        self.assertIsNone(group["primary"])
        self.assertEqual(group["related"], ["delegated-child", "runtime-a"])
        with patch.dict(os.environ, {"HERMES_HOME": str(self.home / "profile-b")}):
            self.assertEqual(len(self.call("relations")["quests"]), 1)
        self.assertEqual(self.call("link", f"{self.quest} --agent other --session ' Exact 001 '", None)["session_id"], " Exact 001 ")
        self.assertEqual(len(self.call("relations", "--agent other --session ' Exact 001 '", None)["quests"]), 1)
        self.call("primary", f"{self.quest} --agent other --clear", None)

    def test_model_tools_use_runtime_session(self):
        from tools.registry import registry

        def tool(name, args, sid="runtime-tool"):
            # The same keyword arguments as the Hermes agent loop gives to each handler.
            return json.loads(registry.dispatch(name, args, scope=self.manager.scope_key, task_id="task",
                                                session_id=sid, conversation_id="conversation", user_task=None))

        plugin._notes["runtime-tool"] = "old note"
        with patch.dict(os.environ, {"HERMES_SESSION_ID": "wrong-parent"}):
            linked = tool("fini_link", {"quest_id": self.quest})
        self.assertEqual((linked["agent"], linked["session_id"], linked["changed"]), ("hermes", "runtime-tool", True))
        self.assertEqual(plugin._notes, {})
        self.assertEqual(tool("fini_relations", {})["quests"][0]["quest_id"], self.quest)
        explicit = tool("fini_link", {"quest_id": self.quest, "agent": "claude",
                                      "session_id": "12345678-1234-1234-1234-123456789abc"})
        self.assertEqual(explicit["session_id"], "12345678-1234-1234-1234-123456789abc")
        self.assertEqual({group["agent"] for group in tool("fini_relations", {"quest_id": self.quest})["agents"]},
                         {"hermes", "claude"})
        self.assertTrue(tool("fini_unlink", {"quest_id": self.quest})["changed"])
        self.assertIn("error", tool("fini_link", {"quest_id": self.quest}, ""))
        self.assertIn("error", tool("fini_link", {"quest_id": self.quest, "agent": "claude"}))

    def test_failures_have_no_environment_identity_fallback(self):
        with patch.dict(os.environ, {"HERMES_SESSION_ID": "wrong-parent"}):
            for sid in (None, "", " "):
                self.assertIn("error", self.call("link", self.quest, sid))
        self.assertIn("error", self.call("link", "missing-quest"))
        self.assertIn("error", self.call("link", f"{self.quest} --agent hermes"))
        self.assertFalse(self.path.exists())

    def test_installed_symlink_and_concurrent_dispatch(self):
        target = self.home / "profile-a" / "plugins" / plugin.PLUGIN_ID
        target.parent.mkdir(parents=True)
        target.symlink_to(SOURCE.parent)
        installed = importlib.util.spec_from_file_location("fini_installed_check", target / "__init__.py")
        assert installed is not None and installed.loader is not None
        module = importlib.util.module_from_spec(installed)
        installed.loader.exec_module(module)
        self.assertEqual(module.CORE, SOURCE.parents[1] / "shared" / "cli.mts")
        manager = PluginManager()
        self.addCleanup(manager.unload)
        module.register(PluginContext(
            PluginManifest(name=plugin.PLUGIN_ID, key=plugin.PLUGIN_ID, source="user"), manager,
        ))
        handler = manager._plugin_commands["fini:link"]["handler"]
        def link(i):
            result = json.loads(invoke_plugin_command(handler, self.quest, session_id=f"parallel-{i}"))
            self.assertNotIn("error", result)
        with ThreadPoolExecutor(max_workers=4) as pool:
            list(pool.map(link, range(8)))
        with sqlite3.connect(self.path) as db:
            self.assertEqual(db.execute("SELECT count(*),sum(is_primary) FROM relations").fetchone(), (8, 1))
        self.assertEqual(len(self.call("relations", f"--quest {self.quest}")["agents"][0]["related"]), 7)


if __name__ == "__main__":
    unittest.main()
