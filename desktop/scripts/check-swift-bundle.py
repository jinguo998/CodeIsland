"""Exercise the Swift C ABI from an extracted Windows installer without the SDK on PATH."""
import ctypes
import json
import os
import sqlite3
import sys
import tempfile
from pathlib import Path


def main():
    # Restrict dependency resolution to the bundle and the operating system.
    directory = Path(sys.argv[1]).resolve(strict=True)
    os.environ["PATH"] = str(Path(os.environ["SystemRoot"]) / "System32")
    core = ctypes.CDLL(str(directory / "CodeIslandDesktopCore.dll"), winmode=0x100 | 0x800)
    exercise(core)


def exercise(core):
    output = bytearray()
    write_type = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p)

    @write_type
    def receive(ptr, length, _):
        output.extend(ctypes.string_at(ptr, length))

    call = core.codeisland_core_call_v1
    call.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p, ctypes.c_size_t,
                     write_type, ctypes.c_void_p]
    call.restype = ctypes.c_int
    def invoke(context, operation, value):
        output.clear()
        payload = json.dumps(value).encode()
        if call(context, operation, payload, len(payload), receive, None):
            raise RuntimeError(output.decode())
        return json.loads(output)

    assert invoke(None, b"normalizeEvent", "post_llm_call") == "AgentTurnSettled"
    # The IPC decoder can reorder keys, including a nested type ahead of the
    # outer one. Only public assistant output belongs on the card.
    row = {"payload": {"content": [{"type": "input_text", "text": "private"},
                                   {"type": "output_text", "text": "done"}],
                       "role": "assistant", "type": "message"}, "type": "response_item"}
    assert invoke(None, b"transcript.kind", row)["publicOutput"] == "done"
    with tempfile.TemporaryDirectory() as temporary:
        store = Path(temporary) / "closed #1.sqlite"
        db = sqlite3.connect(store)
        try:
            db.execute("PRAGMA journal_mode=WAL")
            db.execute("CREATE TABLE threads(id, rollout_path, cwd, updated_at, archived, source)")
            db.execute("INSERT INTO threads VALUES ('a','rollout.jsonl','work',1000,0,'appServer')")
            db.commit()
            args = {"path": str(store), "now": 1000}
            assert invoke(None, b"codex.storeRows", args)[0]["id"] == "a"
        finally:
            db.close()
        before = store.read_bytes()
        assert invoke(None, b"codex.storeRows", args)[0]["id"] == "a"
        assert list(store.parent.iterdir()) == [store] and store.read_bytes() == before
    create = core.codeisland_core_create_v1
    create.argtypes = []
    create.restype = ctypes.c_void_p
    destroy = core.codeisland_core_destroy_v1
    destroy.argtypes = [ctypes.c_void_p]
    destroy.restype = None
    context = create()
    try:
        card = {"status": "idle", "executing": False, "interrupted": False, "failed": False, "tasks": []}
        args = {"card": "gateway", "source": "hermes", "event": "post_llm_call", "previous": card,
                "fresh": True, "limit": 20, "payload": {"session_id": "gateway", "extra": {
                    "platform": "telegram", "user_message": "status?", "assistant_response": "finished"}}}
        reply = invoke(context, b"session.reduce", args)
        assert reply["prompt"] == "status?" and reply["reply"] == "finished" and not reply["ended"]
        args.update(event="on_session_end", fresh=False, previous=reply)
        assert invoke(context, b"session.reduce", args)["ended"]
        for root in ("C:/Users/me", "//server/share"):
            directory = root + "/.cursor/projects/active"
            path = (directory + "/agent-transcripts/stream.jsonl").replace("/", chr(92))
            args = {"card": root, "source": "cursor", "event": "PreToolUse", "previous": card,
                    "fresh": True, "limit": 20, "payload": {"session_id": root, "transcript_path": path}}
            actual = invoke(context, b"session.reduce", args).get("cwd")
            assert actual == directory, (directory, actual)
    finally:
        destroy(context)


if __name__ == "__main__":
    main()
