"""Hermes registration/context bridge; all quest/session logic lives in shared/*.mts."""

import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import time

PLUGIN_ID = "fini-hermes-plugin"
CORE = Path(__file__).resolve().parent / "shared" / "cli.mts"
MAX_BYTES = 1024 * 1024


def _call(command, args, session_id=None):
    """`args` is command text (raw_args) or tool fields (input)."""
    try:
        payload = json.dumps({"command": command, "raw_args" if isinstance(args, str) else "input": args,
                              "current": {"agent": "hermes", "session_id": session_id}}).encode()
        if len(payload) > MAX_BYTES:
            raise ValueError("Core request exceeds 1 MiB.")
        return _transport(payload)
    except (OSError, ValueError, subprocess.SubprocessError) as exc:
        return json.dumps({"error": str(exc)})


def _transport(payload):
    # Fixed reviewed executable/entrypoint; identifiers travel only over stdin.
    # Temporary input avoids a pipe write blocking before the deadline is polled.
    import tempfile
    with tempfile.TemporaryFile(dir=os.environ.get("TMPDIR")) as input_stream:
        input_stream.write(payload)
        input_stream.seek(0)
        with subprocess.Popen(
            ["node", "--disable-warning=ExperimentalWarning", str(CORE)], stdin=input_stream, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, start_new_session=True,
        ) as process:
            try:
                output = bytearray()
                total = 0
                deadline = time.monotonic() + 15
                assert process.stdout is not None and process.stderr is not None
                with selectors.DefaultSelector() as selector:
                    selector.register(process.stdout.fileno(), selectors.EVENT_READ, True)
                    selector.register(process.stderr.fileno(), selectors.EVENT_READ, False)
                    while selector.get_map():
                        remaining = deadline - time.monotonic()
                        if remaining <= 0:
                            raise ValueError("Quest session core timed out; read state before retrying.")
                        for key, _ in selector.select(min(remaining, 0.1)):
                            chunk = os.read(key.fd, 65536)
                            if not chunk:
                                selector.unregister(key.fileobj)
                                continue
                            total += len(chunk)
                            if total > MAX_BYTES:
                                raise ValueError("Core output exceeds 1 MiB.")
                            if key.data:
                                output.extend(chunk)
                process.wait(timeout=max(0.01, deadline - time.monotonic()))
                if process.returncode:
                    raise ValueError("Quest session core failed; check Node and plugin dependencies.")
                result = json.loads(output)
                if not isinstance(result, dict):
                    raise ValueError("Invalid core response.")
                return json.dumps(result)
            finally:
                # Also clean descendants if the parent exited before closing their pipes.
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()


# The quest note per session. A Fini command clears it, so the next turn reads it again.
_notes = {}


def _command(command, raw_args, session_id=None):
    result = _call(command, raw_args, session_id)
    _notes.clear()
    return result


def _context(session_id=None, **_):
    if not session_id:
        return None
    if session_id not in _notes:
        result = json.loads(_call("fini:context", "", session_id))
        _notes[session_id] = result.get("text") or ""
    return {"context": _notes[session_id]} if _notes[session_id] else None


# Model tools. Hermes gives the handler the current session_id at dispatch;
# the model can only give an explicit target.
_TARGET = {
    "agent": {"type": "string", "description": "Agent of an explicit target session. Give it with session_id, or give neither."},
    "session_id": {"type": "string", "description": "ID of an explicit target session. Leave out agent and session_id for the current session."},
}
_QUEST = {"type": "string", "description": "Fini quest ID."}
_PRIMARY = {"type": "boolean", "description": "Also make this session the primary session of its agent for the quest. "
            "Use only when the user asks. The former primary stays linked."}
_TOOLS = (
    ("fini_link", "fini:link", "Link a Fini quest to the current agent session, or to an explicit agent session. "
     "Only link when the user asks or agrees. Quest content changes stay with the fini CLI.", True),
    ("fini_unlink", "fini:unlink", "Remove one link between a Fini quest and the current agent session, "
     "or an explicit agent session.", True),
    ("fini_relations", "fini:relations", "Show the Fini quests of the current agent session, "
     "or with quest_id the agent sessions of that quest.", False),
)


def _tool(command, args, session_id=None, **_):
    # Only the schema fields go to the core; the session ID comes from Hermes.
    fields = args if isinstance(args, dict) else {}
    result = _call(command, {key: fields[key] for key in ("quest_id", "agent", "session_id", "primary") if key in fields}, session_id)
    if command != "fini:relations":
        _notes.clear()
    return result


def register(ctx):
    from functools import partial

    ctx.register_hook("pre_llm_call", _context)
    for name, command, description, target in _TOOLS:
        properties = {"quest_id": _QUEST, **(_TARGET if target else {}),
                      **({"primary": _PRIMARY} if name == "fini_link" else {})}
        ctx.register_tool(
            name=name, toolset="fini", handler=partial(_tool, command), description=description,
            schema={"name": name, "description": description, "parameters": {
                "type": "object", "properties": properties, "additionalProperties": False,
                **({"required": ["quest_id"]} if target else {})}},
        )
    for name in ("link", "unlink", "relations"):
        ctx.register_command(
            f"fini:{name}", partial(_command, f"fini:{name}"),
            description=f"Fini quest-session {name} (local only).",
            args_hint="[quest-id] [--primary] [--agent NAME --session ID]", argument_mode="text",
        )
