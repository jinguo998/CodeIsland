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
    output = bytearray()
    write_type = ctypes.CFUNCTYPE(None, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p)

    @write_type
    def receive(ptr, length, _):
        output.extend(ctypes.string_at(ptr, length))

    call = core.codeisland_core_call_v1
    call.argtypes = [ctypes.c_void_p, ctypes.c_char_p, ctypes.c_char_p, ctypes.c_size_t,
                     write_type, ctypes.c_void_p]
    call.restype = ctypes.c_int
    payload = json.dumps("post_llm_call").encode()
    result = call(None, b"normalizeEvent", payload, len(payload), receive, None)
    if result or json.loads(output) != "AgentTurnSettled":
        raise RuntimeError("The installed Swift core did not normalize the Hermes hook")


if __name__ == "__main__":
    main()
