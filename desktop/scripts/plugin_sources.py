"""Build the installed extensions from the original agent resources and Windows I/O."""
from pathlib import Path
import sys


def replace_region(source, start, end, replacement):
    lines = source.splitlines(keepends=True)
    def boundary(prefix):
        found = [i for i, line in enumerate(lines) if line.startswith(prefix)]
        if len(found) != 1:
            raise ValueError(f"expected one plugin boundary {prefix!r}, found {len(found)}")
        return found[0]
    first, last = boundary(start), boundary(end)
    if first >= last:
        raise ValueError(f"out-of-order plugin boundaries {start!r}, {end!r}")
    return "".join(lines[:first]) + replacement + "".join(lines[last:])


def build(root, output):
    resources = root / "Sources/CodeIsland/Resources"
    transport = (root / "desktop/scripts/plugin-transport.js").read_text(encoding="utf-8")
    for name, filename in (("opencode", "codeisland-opencode.js"), ("pi", "codeisland-pi.ts"),
                           ("omp", "codeisland-omp.ts"), ("openclaw", "codeisland-openclaw/index.ts")):
        text = (resources / filename).read_text(encoding="utf-8")
        if name == "opencode":
            text = replace_region(text, 'import { connect }', 'import { homedir }',
                                  'import { execFile } from "node:child_process";\n'
                                  'import { readFileSync, writeFileSync } from "node:fs";\n')
            text = replace_region(text, "const SOCKET =", "/// OpenCode's", transport + "\n")
            text = replace_region(text, "    let detectedTty =", "    function collectEnv",
                                  "    const detectedTty = null;\n\n")
        else:
            # Preserve the agent SDK types. Remove only Unix socket/process imports.
            text = "\n".join(line for line in text.split("\n") if not line.startswith((
                'import { existsSync, lstatSync }', 'import { connect }', 'import { getuid }',
                'import type { ChildProcess }')))
            text = text.replace("import { execFile, execFileSync }", "import { execFile }")
            text = replace_region(text, "const userId =", "const ENV_KEYS", "")
            next_section = "// ── Lane resolution" if name == "openclaw" else "// ── Event builders"
            text = replace_region(text, "function detectTty()", next_section,
                                  "function detectTty(): null { return null; }\n" + transport + "\n")
        # JSDoc keeps the JavaScript adapter runnable in OpenCode. Add types when
        # the host loads a TypeScript extension under noImplicitAny.
        types = {"/*payload*/": ": object", "/*millis*/": ": number", "/*env*/": ": NodeJS.ProcessEnv | undefined",
                 "/*child*/": ": import('node:child_process').ChildProcess | undefined",
                 "/*result*/": "<Record<string, unknown> | null>"}
        text = text.replace("__CODEISLAND_WAIT_MS__", "300000" if name == "opencode" else "30000")
        for token, annotation in types.items():
            text = text.replace(token, "" if name == "opencode" else annotation)
        (output / (name + Path(filename).suffix)).write_bytes(text.encode("utf-8"))


if __name__ == "__main__":
    build(Path(__file__).resolve().parents[2], Path(sys.argv[1]))
