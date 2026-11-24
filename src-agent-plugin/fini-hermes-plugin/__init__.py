"""Hermes registration/context bridge; all quest/session logic lives in shared/*.mts."""

import json
import os
from pathlib import Path
import selectors
import signal
import subprocess
import time

PLUGIN_ID = "fini-hermes-plugin"
CORE = Path(__file__).resolve().parents[1] / "shared" / "cli.mts"
MAX_BYTES = 1024 * 1024


def _call(command, raw_args, session_id=None):
    from hermes_constants import get_hermes_home

    # Same profile-local convention as plugin_data_dir(), without creating state
    # before the core validates the request. Resolve on every call, never cache.
    try:
        explicit = os.environ.get("FINI_SESSIONS_PATH")
        path = explicit if explicit is not None else str(
            get_hermes_home() / "plugin-data" / PLUGIN_ID / "sessions.json"
        )
        payload = json.dumps({"command": command, "raw_args": raw_args,
                              "session_id": session_id, "state_path": str(path)}).encode()
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
            ["node", str(CORE)], stdin=input_stream, stdout=subprocess.PIPE,
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
                    raise ValueError("Quest session core failed; check Node 24.12+ and util-linux flock.")
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


def link_session(raw_args, *, session_id=None):
    return _call("fini-link", raw_args, session_id)


def link_claude_session(raw_args):
    return _call("fini-link-claude", raw_args)


def list_sessions(raw_args):
    return _call("fini-sessions", raw_args)


def link_current_session(args, *, session_id=None):
    # Runtime kwargs and model arguments are separate channels. Never accept an
    # identity from args, environment, registration state or a parent session.
    if not isinstance(args, dict) or set(args) != {"quest_id"}:
        return json.dumps({"error": "Expected only quest_id; session identity is runtime-owned."})
    return _call("fini-link", args["quest_id"], session_id)


def register(ctx):
    import shutil

    ctx.register_tool(
        name="fini_link_current_session", toolset="fini",
        schema={
            "name": "fini_link_current_session",
            "description": (
                "Link the actual calling Hermes session to an explicitly user-selected quest ID locally. "
                "Use when asked to connect this/current session. In a delegated agent this links the "
                "child, not its parent; the parent must call for itself. No session ID argument."
            ),
            "parameters": {
                "type": "object", "properties": {"quest_id": {"type": "string"}},
                "required": ["quest_id"], "additionalProperties": False,
            },
        },
        handler=link_current_session,
        check_fn=lambda: CORE.is_file() and bool(shutil.which("node")) and bool(shutil.which("flock")),
    )
    ctx.register_command(
        "fini-link-claude", link_claude_session,
        description="Explicitly link a supplied Claude UUID to a quest (local only).",
        args_hint='<JSON: {"quest_id":"...","session_id":"..."}>', argument_mode="text",
    )
    ctx.register_command(
        "fini-link", link_session,
        description="Explicitly link this Hermes session to a quest ID (local only).",
        args_hint="<quest-id>", argument_mode="text",
    )
    ctx.register_command(
        "fini-sessions", list_sessions,
        description="List sessions explicitly linked to a quest in this profile.",
        args_hint="<quest-id>", argument_mode="text",
    )
