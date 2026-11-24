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
    try:
        payload = json.dumps({"command": command, "raw_args": raw_args,
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


def register(ctx):
    from functools import partial

    for name in ("link", "unlink", "primary", "relations"):
        ctx.register_command(
            f"fini:{name}", partial(_call, f"fini:{name}"),
            description=f"Fini quest-session {name} (local only).",
            args_hint="[quest-id] [--agent NAME --session ID]", argument_mode="text",
        )
