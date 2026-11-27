// @vitest-environment node
import { test, expect } from "vitest";
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

test("the original provider callbacks deliver events through the installed sidecar interface", () => {
  const dir = mkdtempSync(join(tmpdir(), "CodeIsland plugin test "));
  try {
    const builder = fileURLToPath(new URL("../scripts/plugin_sources.py", import.meta.url));
    execFileSync(process.platform === "win32" ? "python" : "python3", [builder, dir]);
    for (const name of ["opencode", "pi", "omp", "openclaw"]) {
      const js = name === "opencode";
      const source = readFileSync(join(dir, name + (js ? ".js" : ".ts")), "utf8")
        .replaceAll("__CODEISLAND_EXECUTABLE__", JSON.stringify(process.execPath))
        .replaceAll("__CODEISLAND_SOURCE__", JSON.stringify("codeisland-" + name));
      const body = js ? source : ts.transpileModule(source, {
        compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
      }).outputText;
      writeFileSync(join(dir, name + ".mjs"), body);
    }
    // Node is the test sidecar. Its first argument is the "send" subcommand.
    writeFileSync(join(dir, "send"), [
      'const fs = require("node:fs");',
      'const payload = JSON.parse(fs.readFileSync(0, "utf8"));',
      'fs.appendFileSync("frames", JSON.stringify({args:process.argv.slice(2), payload}) + "\\n");',
      'process.stdout.write("{}");',
    ].join("\n"));
    writeFileSync(join(dir, "host.mjs"), [
      'import opencode from "./opencode.mjs";',
      'import pi from "./pi.mjs";',
      'import omp from "./omp.mjs";',
      'import openclaw from "./openclaw.mjs";',
      'const hooks = await opencode.server({client:{}, serverUrl:new URL("http://localhost")});',
      'await hooks.event({event:{type:"session.created",properties:{info:{id:"s",directory:"/example"}}}});',
      'const schema = {optional() { return this; }, min() { return this; }, refine() { return this; }};',
      'const zod = Object.fromEntries(["string","number","boolean","object","array"].map(k => [k, () => schema]));',
      'for (const register of [pi, omp, openclaw.register]) {',
      '  const handlers = new Map();',
      '  register({zod, pi:{settings:{}, AskTool:class {}}, on:(name, fn) => handlers.set(name, fn), registerTool:() => {}, getSessionName:() => undefined});',
      '  await handlers.get("session_start")({}, {cwd:"/example", workspaceDir:"/example", sessionKey:"s",',
      '    sessionManager:{getSessionId:() => "s"}});',
      '}',
    ].join("\n"));
    execFileSync(process.execPath, ["host.mjs"], { cwd: dir, timeout: 15_000 });
    const frames = readFileSync(join(dir, "frames"), "utf8").trim().split("\n").map(JSON.parse);
    expect(frames).toHaveLength(4);
    for (const name of ["opencode", "pi", "omp", "openclaw"]) {
      const frame = frames.find(({args}) => args[1] === "codeisland-" + name);
      expect(frame.args).toContain("--stdin");
      expect(frame.args[3]).toBe("SessionStart");
      expect(frame.payload).toMatchObject({hook_event_name:"SessionStart", cwd:"/example"});
    }
  } finally {
    rmSync(dir, {recursive:true, force:true});
  }
}, 25_000);
