"""Exercise the Swift C ABI from an extracted Windows installer without the SDK on PATH."""
import ctypes
import json
import os
import sys
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
    finally:
        destroy(context)


if __name__ == "__main__":
    main()
