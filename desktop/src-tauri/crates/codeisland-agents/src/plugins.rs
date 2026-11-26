//! Generated agent plugins/extensions.
//!
//! macOS shipped its OpenCode/Pi/OpenClaw/MiMo integrations as resource files
//! that talked to a Unix socket and the `codeisland-bridge` helper. On Windows
//! the transport is the app's named pipe, reached through the one-shot
//! `codeisland-hook send … --stdin` client, so every plugin spawns that
//! executable and (for blocking requests) reads the decision from stdout.
//!
//! The generated text is deterministic: install/uninstall compare bytes, so a
//! second install is a no-op.

use codeisland_domain::AgentKind;

use crate::definitions::AgentDefinition;

/// Pick the plugin source for a `plugin_js` / `opencode_plugin` definition.
pub fn plugin_source(def: &AgentDefinition, hook_program: &str) -> String {
    match def.agent {
        AgentKind::OpenCode | AgentKind::MiMo => {
            opencode_plugin(&def.source, &def.marker(), hook_program)
        }
        AgentKind::Pi => pi_extension(&def.source, &def.marker(), hook_program),
        AgentKind::OpenClaw => openclaw_plugin(&def.source, &def.marker(), hook_program),
        _ => stub(&def.marker()),
    }
}

fn json_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

fn fill(template: &str, source: &str, marker: &str, hook_program: &str) -> String {
    template
        .replace("__HOOK__", &json_string(hook_program))
        .replace("__SOURCE__", &json_string(source))
        .replace("__MARKER__", marker)
}

pub fn stub(marker: &str) -> String {
    format!(
        "// codeisland-plugin: {marker}\n\
         // Managed by CodeIsland. This file is replaced during app upgrades.\n\
         export default {{}};\n"
    )
}

// ── OpenCode / MiMo ─────────────────────────────────────────────────────────

pub fn opencode_plugin(source: &str, marker: &str, hook_program: &str) -> String {
    fill(OPENCODE_TEMPLATE, source, marker, hook_program)
}

const OPENCODE_TEMPLATE: &str = r####"// codeisland-plugin: __MARKER__
// version: v1
// CodeIsland OpenCode plugin. Managed by CodeIsland; replaced on upgrade.
//
// One file serves OpenCode 1.x's `server()` hooks, OpenCode 2's `setup()`
// event service, and MiMo Code (the same 1.x contract, SOURCE rewritten).
// Blocking permission/question requests spawn `codeisland-hook send … --stdin`
// and wait for its decision.
import { execFile } from "node:child_process";
import { readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const HOOK = __HOOK__;
const SOURCE = __SOURCE__;
const BASE_ARGS = ["send", "--source", "__MARKER__", "--stdin"];

function runHook(event, payload, wait = false) {
  return new Promise((resolve) => {
    let child;
    try {
      child = execFile(
        HOOK,
        [...BASE_ARGS, "--event", event],
        { timeout: wait ? 86400000 : 30000, maxBuffer: 4 * 1024 * 1024 },
        (error, stdout) => {
          if (error) return resolve(null);
          const text = String(stdout || "").trim();
          if (!text) return resolve(null);
          try { resolve(JSON.parse(text)); } catch { resolve(null); }
        },
      );
    } catch {
      resolve(null);
      return;
    }
    try {
      child.stdin.write(JSON.stringify(payload));
      child.stdin.end();
    } catch {
      try { child.kill(); } catch {}
      resolve(null);
    }
  });
}

function prettyToolName(name) {
  const raw = String(name || "");
  return raw.charAt(0).toUpperCase() + raw.slice(1);
}

function permissionInput(action, resources, metadata, message) {
  const items = resources || [];
  const toolInput = { patterns: items, metadata };
  if ((action === "bash" || action === "shell") && items.length > 0) toolInput.command = items.join(" && ");
  if ((action === "edit" || action === "write") && items.length > 0) toolInput.file_path = items[0];
  if (message) toolInput.description = message;
  return toolInput;
}

function mapQuestions(questions) {
  return (questions || []).map((q) => ({
    question: q.question || "",
    header: q.header || "",
    options: (q.options || []).map((o) => ({ label: o.label, description: o.description })),
    multiSelect: q.multiple || false,
  }));
}

function answerKeys(questions) {
  const used = new Set();
  return (questions || []).map((q) => {
    const base = typeof q?.question === "string" ? q.question : "Question";
    let key = base;
    if (used.has(key)) {
      let suffix = 2;
      while (used.has(`${base}_${suffix}`)) suffix += 1;
      key = `${base}_${suffix}`;
    }
    used.add(key);
    return key;
  });
}

function answersFromDecision(questions, decision) {
  const input = decision?.updatedInput || {};
  const answers = input.answers || {};
  const keys = answerKeys(questions);
  const asLabels = (value) => {
    if (Array.isArray(value)) return value.map(String);
    if (value === undefined || value === null || value === "") return [];
    return [String(value)];
  };
  if (!keys.some((key) => key in answers)) {
    return keys.map((_, index) => asLabels(Object.values(answers)[index]));
  }
  return keys.map((key) => asLabels(answers[key]));
}

function permissionReplyFromDecision(decision) {
  const behavior = decision?.behavior;
  if (!behavior) return null;
  const hasUpdatedPerms = decision.updatedPermissions != null;
  if (behavior === "always" || (behavior === "allow" && hasUpdatedPerms)) return "always";
  if (behavior === "allow") return "once";
  return "reject";
}

// ── OpenCode 2 ──────────────────────────────────────────────────────────────

const V2_TOOL_NAMES = { shell: "Bash", subagent: "Task", webfetch: "WebFetch", websearch: "WebSearch" };

function v2ToolName(name) {
  const raw = String(name || "");
  return V2_TOOL_NAMES[raw] || prettyToolName(raw) || "Tool";
}

function v2ToolInput(input) {
  const out = { ...(input || {}) };
  if (out.file_path === undefined) {
    const path = out.filePath ?? out.path;
    if (typeof path === "string") out.file_path = path;
  }
  return out;
}

function v2QuestionsFromForm(form) {
  if (!form || form.metadata?.kind !== "question" || !Array.isArray(form.fields)) return null;
  const questions = form.fields.map((field) => ({
    question: field.description || field.title || "",
    header: field.title || "",
    options: (field.options || []).map((o) => ({ label: o.label ?? o.value, description: o.description })),
    multiSelect: field.type === "multiselect",
  }));
  return questions.length > 0 ? questions : null;
}

function v2FormAnswer(form, answerArrays) {
  const answer = {};
  (form?.fields || []).forEach((field, index) => {
    const picked = answerArrays[index] || [];
    if (picked.length === 0) return;
    answer[field.key] = field.type === "multiselect" ? picked : picked.join(", ");
  });
  return answer;
}

const V2_REPLY_EVENTS = new Set(["permission.replied", "form.replied", "form.cancelled"]);

function createV2Mapper(base) {
  const sessions = new Map();
  const cwdBySession = new Map();
  const toolNames = new Map();
  const sid = (id) => `${SOURCE}-${String(id || "")}`;
  const session = (id) => {
    if (!sessions.has(id)) sessions.set(id, { lastAssistantText: "", pendingTitle: null });
    return sessions.get(id);
  };
  const cwdFor = (id) => cwdBySession.get(id) || "";
  const setCwd = (id, dir) => { if (id && dir) cwdBySession.set(id, dir); };
  const remember = (map, key, value) => {
    map.set(key, value);
    if (map.size > 500) map.delete(map.keys().next().value);
  };

  function map(event) {
    const t = event?.type;
    const d = event?.data || {};
    const id = d.sessionID;
    switch (t) {
      case "session.created":
        if (!id) return null;
        setCwd(id, d.location?.directory);
        if (!isPlaceholderTitle(d.title)) session(id).pendingTitle = d.title;
        return base(sid(id), { hook_event_name: "SessionStart", cwd: cwdFor(id) });
      case "session.moved":
        setCwd(id, d.location?.directory);
        return null;
      case "session.renamed":
        if (id && !isPlaceholderTitle(d.title)) session(id).pendingTitle = d.title;
        return null;
      case "session.deleted":
        if (!id) return null;
        sessions.delete(id);
        cwdBySession.delete(id);
        return base(sid(id), { hook_event_name: "SessionEnd" });
      case "session.inbox.enqueued": {
        if (!id || d.item?.type !== "user") return null;
        const prompt = d.item.payload?.text || "";
        session(id).lastAssistantText = "";
        return base(sid(id), { hook_event_name: "UserPromptSubmit", cwd: cwdFor(id), ...(prompt ? { prompt } : {}) });
      }
      case "session.status": {
        if (!id) return null;
        const type = d.status?.type;
        if (type === "busy" || type === "retry") {
          return base(sid(id), { hook_event_name: "UserPromptSubmit", cwd: cwdFor(id) });
        }
        if (type === "idle") {
          const s = session(id);
          const extra = {
            hook_event_name: "Stop", cwd: cwdFor(id),
            ...(s.lastAssistantText ? { last_assistant_message: s.lastAssistantText } : {}),
          };
          if (s.pendingTitle) { extra.session_title = s.pendingTitle; s.pendingTitle = null; }
          return base(sid(id), extra);
        }
        return null;
      }
      case "session.text.ended":
        if (id && d.text) session(id).lastAssistantText = d.text;
        return null;
      case "session.tool.input.started":
        if (id && d.id) remember(toolNames, `${id}:${d.id}`, v2ToolName(d.name));
        return null;
      case "session.tool.called":
        if (!id) return null;
        return base(sid(id), {
          hook_event_name: "PreToolUse", cwd: cwdFor(id),
          tool_name: toolNames.get(`${id}:${d.id}`) || "Tool", tool_input: v2ToolInput(d.input),
        });
      case "session.tool.success":
      case "session.tool.failed":
        if (!id) return null;
        return base(sid(id), {
          hook_event_name: t === "session.tool.success" ? "PostToolUse" : "PostToolUseFailure",
          cwd: cwdFor(id), tool_name: toolNames.get(`${id}:${d.id}`) || "Tool",
        });
      case "session.compaction.started":
        if (!id) return null;
        return base(sid(id), { hook_event_name: "PreCompact", cwd: cwdFor(id) });
      case "permission.asked":
        if (!id || !d.id) return null;
        return base(sid(id), {
          hook_event_name: "PermissionRequest", cwd: cwdFor(id),
          tool_name: v2ToolName(d.action),
          tool_input: permissionInput(d.action, d.resources, d.metadata, d.message),
          _opencode_request_id: d.id, _opencode_session_id: id,
        });
      case "form.created": {
        const form = d.form;
        const questions = v2QuestionsFromForm(form);
        if (!questions || !form.id || !form.sessionID) return null;
        return base(sid(form.sessionID), {
          hook_event_name: "PermissionRequest", cwd: cwdFor(form.sessionID),
          tool_name: "AskUserQuestion", tool_input: { questions },
          _opencode_request_id: form.id, _opencode_session_id: form.sessionID, _opencode_form: form,
        });
      }
      case "permission.replied":
      case "form.replied":
      case "form.cancelled":
        if (!id) return null;
        return base(sid(id), { hook_event_name: "PostToolUse", cwd: cwdFor(id) });
      default:
        return null;
    }
  }
  map.knows = (id) => cwdBySession.has(id);
  map.adopt = (id, info) => {
    setCwd(id, info?.location?.directory);
    if (!isPlaceholderTitle(info?.title)) session(id).pendingTitle = info.title;
  };
  return map;
}

function v2EventSessionId(event) {
  return event?.data?.sessionID || event?.data?.form?.sessionID;
}

function claimV2Event(eventId) {
  if (!eventId) return true;
  const key = Symbol.for("codeisland.opencode.v2.claimed");
  const claimed = globalThis[key] || (globalThis[key] = new Map());
  if (claimed.has(eventId)) return false;
  claimed.set(eventId, true);
  if (claimed.size > 2000) claimed.delete(claimed.keys().next().value);
  return true;
}

function v2ServiceEndpoint() {
  try {
    const stateHome = process.env.XDG_STATE_HOME
      ? join(process.env.XDG_STATE_HOME, "opencode")
      : process.platform === "win32"
        ? join(process.env.LOCALAPPDATA || homedir(), "opencode")
        : join(homedir(), ".local", "state", "opencode");
    const info = JSON.parse(readFileSync(join(stateHome, "service.json"), "utf8"));
    if (info?.pid !== process.pid || typeof info.url !== "string") return null;
    return { url: info.url, password: info.password };
  } catch { return null; }
}

async function v2FormRequest(method, path, body) {
  const endpoint = v2ServiceEndpoint();
  if (!endpoint) return false;
  const headers = { "Content-Type": "application/json" };
  if (endpoint.password) {
    headers.Authorization = "Basic " + Buffer.from(`opencode:${endpoint.password}`).toString("base64");
  }
  try {
    const res = await fetch(new URL(path, endpoint.url), {
      method, headers, ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
    return res.ok;
  } catch { return false; }
}

function createV2Runtime({ base, send, ask, replyPermission, replyForm, cancelForm, canAnswerForms, lookupSession }) {
  const map = createV2Mapper(base);
  const pendingSessions = new Set();
  const lookedUp = new Set();

  async function adoptUnknownSession(event) {
    const id = v2EventSessionId(event);
    if (!id || !lookupSession || event.type === "session.created" || map.knows(id) || lookedUp.has(id)) return;
    lookedUp.add(id);
    try {
      const result = await lookupSession(id);
      map.adopt(id, result?.data ?? result);
    } catch {}
  }

  async function blocking(mapped, settle) {
    pendingSessions.add(mapped.session_id);
    let response;
    try {
      response = await ask(mapped);
    } finally {
      pendingSessions.delete(mapped.session_id);
    }
    const decision = response?.hookSpecificOutput?.decision;
    if (decision) await settle(decision);
  }

  return async function handle(event) {
    if (!claimV2Event(event?.id)) return;
    await adoptUnknownSession(event);
    const mapped = map(event);
    if (!mapped) return;
    if (mapped.hook_event_name === "PermissionRequest" && mapped._opencode_form) {
      const form = mapped._opencode_form;
      const payload = { ...mapped };
      delete payload._opencode_form;
      if (!canAnswerForms()) {
        await send({ ...payload, hook_event_name: "Notification", message: payload.tool_input.questions[0]?.question || "Question" });
        return;
      }
      blocking(payload, async (decision) => {
        if (decision.behavior === "deny") {
          await cancelForm(form.sessionID, form.id);
          return;
        }
        const answer = v2FormAnswer(form, answersFromDecision(payload.tool_input.questions, decision));
        await replyForm(form.sessionID, form.id, answer);
      }).catch(() => {});
      return;
    }
    if (mapped.hook_event_name === "PermissionRequest") {
      blocking(mapped, async (decision) => {
        const reply = permissionReplyFromDecision(decision);
        if (!reply) return;
        await replyPermission(mapped._opencode_session_id, mapped._opencode_request_id, reply, decision.reason);
      }).catch(() => {});
      return;
    }
    if (!V2_REPLY_EVENTS.has(event.type)
        && pendingSessions.has(mapped.session_id)
        && mapped.hook_event_name !== "SessionStart"
        && mapped.hook_event_name !== "SessionEnd") {
      return;
    }
    await send(mapped);
  };
}

function setupV2(ctx) {
  const base = (sessionId, extra) => ({ session_id: sessionId, _source: SOURCE, ...extra });
  const handle = createV2Runtime({
    base,
    send: (payload) => runHook(payload.hook_event_name, payload),
    ask: (payload) => runHook(payload.hook_event_name, payload, true),
    replyPermission: (sessionID, requestID, decision, message) =>
      ctx.permission.reply({ sessionID, requestID, decision, ...(message ? { message } : {}) }).catch(() => {}),
    replyForm: (sessionID, formID, answer) =>
      v2FormRequest("POST", `/api/session/${encodeURIComponent(sessionID)}/form/${encodeURIComponent(formID)}/reply`, { answer }),
    cancelForm: (sessionID, formID) =>
      v2FormRequest("DELETE", `/api/session/${encodeURIComponent(sessionID)}/form/${encodeURIComponent(formID)}`),
    canAnswerForms: () => v2ServiceEndpoint() !== null,
    lookupSession: (sessionID) => ctx.session.get({ sessionID }),
  });

  const controller = new AbortController();
  (async () => {
    try {
      for await (const event of ctx.event.subscribe({ signal: controller.signal })) {
        try { await handle(event); } catch {}
      }
    } catch {}
  })();
  return () => controller.abort();
}

export default {
  id: "codeisland",
  setup: setupV2,
  server: async ({ serverUrl }) => {
    const port = serverUrl ? Number(serverUrl.port) || 4096 : 4096;
    const endpoint = serverUrl ? String(serverUrl).replace(/\/$/, "") : `http://localhost:${port}`;
    const sessions = new Map();
    const sessionCwd = new Map();
    const msgRoles = new Map();
    const toolNamesByCall = new Map();
    const pendingRequestSessions = new Set();

    function getSession(id) {
      if (!sessions.has(id)) sessions.set(id, { lastAssistantText: "", lastUserText: "" });
      return sessions.get(id);
    }
    function islandSessionId(id) { return `${SOURCE}-${String(id || "")}`; }
    function cwdFor(id) { return sessionCwd.get(String(id || "")) || process.env.PWD || ""; }
    function setCwd(id, cwd) { if (id && cwd) sessionCwd.set(String(id), cwd); }
    function rememberTool(id, callId, name) {
      if (!id || !callId || !name) return;
      toolNamesByCall.set(`${id}:${callId}`, prettyToolName(name));
      if (toolNamesByCall.size > 200) toolNamesByCall.delete(toolNamesByCall.keys().next().value);
    }
    function rememberedTool(id, callId) {
      return toolNamesByCall.get(`${id}:${callId}`) || "Tool";
    }
    function base(sessionId, extra) {
      return { session_id: sessionId, _source: SOURCE, ...extra };
    }

    function mapEvent(ev) {
      const t = ev.type;
      const p = ev.properties || {};
      if (t === "session.created" && p.info) {
        const cwd = p.info.directory || p.info.location?.directory || "";
        setCwd(p.info.id, cwd);
        return base(islandSessionId(p.info.id), { hook_event_name: "SessionStart", cwd });
      }
      if (t === "session.deleted" && p.info) {
        sessions.delete(p.info.id);
        sessionCwd.delete(String(p.info.id));
        return base(islandSessionId(p.info.id), { hook_event_name: "SessionEnd" });
      }
      if (t === "session.updated" && p.info) {
        if (p.info.directory || p.info.location?.directory) {
          setCwd(p.info.id, p.info.directory || p.info.location?.directory);
        }
        if (p.info.time?.archived) {
          sessions.delete(p.info.id);
          sessionCwd.delete(String(p.info.id));
          return base(islandSessionId(p.info.id), { hook_event_name: "SessionEnd" });
        }
        return null;
      }
      if (t === "session.next.moved" && p.sessionID) {
        setCwd(p.sessionID, p.location?.directory || "");
        return base(islandSessionId(p.sessionID), { hook_event_name: "SessionStart", cwd: cwdFor(p.sessionID) });
      }
      if (t === "session.next.prompted" || t === "session.next.prompt.admitted") {
        if (!p.sessionID) return null;
        const prompt = p.prompt?.text || p.text || "";
        if (prompt) getSession(p.sessionID).lastUserText = prompt;
        return base(islandSessionId(p.sessionID), {
          hook_event_name: "UserPromptSubmit", cwd: cwdFor(p.sessionID), ...(prompt ? { prompt } : {}),
        });
      }
      if (t === "session.next.text.delta" && p.sessionID) {
        const s = getSession(p.sessionID);
        s.textParts = s.textParts || new Map();
        const key = p.textID || "default";
        s.textParts.set(key, (s.textParts.get(key) || "") + (p.delta || ""));
        s.lastAssistantText = Array.from(s.textParts.values()).join("\n");
        return null;
      }
      if (t === "session.next.text.ended" && p.sessionID) {
        if (p.text) getSession(p.sessionID).lastAssistantText = p.text;
        return null;
      }
      if (t === "session.next.compaction.started" && p.sessionID) {
        return base(islandSessionId(p.sessionID), { hook_event_name: "PreCompact", cwd: cwdFor(p.sessionID) });
      }
      if (t === "session.next.tool.called" && p.sessionID) {
        const toolName = prettyToolName(p.tool || p.name);
        rememberTool(p.sessionID, p.callID, toolName);
        return base(islandSessionId(p.sessionID), {
          hook_event_name: "PreToolUse", cwd: cwdFor(p.sessionID), tool_name: toolName, tool_input: p.input || {},
        });
      }
      if (t === "session.next.tool.success" && p.sessionID) {
        return base(islandSessionId(p.sessionID), {
          hook_event_name: "PostToolUse", cwd: cwdFor(p.sessionID), tool_name: rememberedTool(p.sessionID, p.callID),
        });
      }
      if (t === "session.next.tool.failed" && p.sessionID) {
        return base(islandSessionId(p.sessionID), {
          hook_event_name: "PostToolUseFailure", cwd: cwdFor(p.sessionID), tool_name: rememberedTool(p.sessionID, p.callID),
        });
      }
      if (t === "session.status" && p.sessionID) {
        const s = getSession(p.sessionID);
        if (p.status?.type === "busy" || p.status?.type === "retry") {
          return base(islandSessionId(p.sessionID), { hook_event_name: "UserPromptSubmit", cwd: cwdFor(p.sessionID) });
        }
        if (p.status?.type === "idle") {
          return base(islandSessionId(p.sessionID), {
            hook_event_name: "Stop", cwd: cwdFor(p.sessionID),
            ...(s.lastAssistantText ? { last_assistant_message: s.lastAssistantText } : {}),
          });
        }
      }
      if (t === "message.updated" && p.info?.id && p.info?.sessionID) {
        msgRoles.set(p.info.id, { role: p.info.role, sessionID: p.info.sessionID });
        if (msgRoles.size > 200) msgRoles.delete(msgRoles.keys().next().value);
        return null;
      }
      if (t === "message.part.updated" && p.part?.type === "text" && p.part?.messageID) {
        const meta = msgRoles.get(p.part.messageID);
        if (!meta) return null;
        const s = getSession(meta.sessionID);
        const text = p.part.text || "";
        if (meta.role === "user" && text) {
          s.lastUserText = text;
          return base(islandSessionId(meta.sessionID), {
            hook_event_name: "UserPromptSubmit", cwd: cwdFor(meta.sessionID), prompt: text,
          });
        }
        if (meta.role === "assistant" && text) s.lastAssistantText = text;
        return null;
      }
      if (t === "message.part.updated" && p.part?.type === "tool" && p.part?.sessionID) {
        const status = p.part.state?.status;
        const toolName = prettyToolName(p.part.tool || p.part.name);
        if (status === "running" || status === "pending") {
          return base(islandSessionId(p.part.sessionID), {
            hook_event_name: "PreToolUse", cwd: cwdFor(p.part.sessionID),
            tool_name: toolName, tool_input: p.part.state?.input || {},
          });
        }
        if (status === "completed" || status === "error") {
          return base(islandSessionId(p.part.sessionID), {
            hook_event_name: status === "error" ? "PostToolUseFailure" : "PostToolUse",
            cwd: cwdFor(p.part.sessionID), tool_name: toolName,
          });
        }
      }
      if (t === "permission.asked" && p.id && p.sessionID) {
        return base(islandSessionId(p.sessionID), {
          hook_event_name: "PermissionRequest", cwd: cwdFor(p.sessionID),
          tool_name: prettyToolName(p.permission),
          tool_input: permissionInput(p.permission, p.patterns, p.metadata), _opencode_request_id: p.id,
        });
      }
      if (t === "permission.replied" && p.sessionID) {
        return base(islandSessionId(p.sessionID), { hook_event_name: "PostToolUse", cwd: cwdFor(p.sessionID) });
      }
      if (t === "permission.v2.asked" && p.id && p.sessionID) {
        return base(islandSessionId(p.sessionID), {
          hook_event_name: "PermissionRequest", cwd: cwdFor(p.sessionID),
          tool_name: prettyToolName(p.action),
          tool_input: permissionInput(p.action, p.resources, p.metadata), _opencode_request_id: p.id,
        });
      }
      if (t === "permission.v2.replied" && p.sessionID) {
        return base(islandSessionId(p.sessionID), { hook_event_name: "PostToolUse", cwd: cwdFor(p.sessionID) });
      }
      if (t === "question.asked" && p.id && p.sessionID) {
        return base(islandSessionId(p.sessionID), {
          hook_event_name: "PermissionRequest", cwd: cwdFor(p.sessionID),
          tool_name: "AskUserQuestion", tool_input: { questions: mapQuestions(p.questions) },
          _opencode_request_id: p.id,
        });
      }
      if ((t === "question.replied" || t === "question.rejected") && p.sessionID) {
        return base(islandSessionId(p.sessionID), { hook_event_name: "PostToolUse", cwd: cwdFor(p.sessionID) });
      }
      if (t === "question.v2.asked" && p.id && p.sessionID) {
        return base(islandSessionId(p.sessionID), {
          hook_event_name: "PermissionRequest", cwd: cwdFor(p.sessionID),
          tool_name: "AskUserQuestion", tool_input: { questions: mapQuestions(p.questions) },
          _opencode_request_id: p.id,
        });
      }
      if ((t === "question.v2.replied" || t === "question.v2.rejected") && p.sessionID) {
        return base(islandSessionId(p.sessionID), { hook_event_name: "PostToolUse", cwd: cwdFor(p.sessionID) });
      }
      return null;
    }

    async function post(path, body) {
      try {
        await fetch(`${endpoint}${path}`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          ...(body ? { body: JSON.stringify(body) } : {}),
        });
      } catch {}
    }

    async function handlePermission(mapped) {
      const id = mapped._opencode_request_id;
      pendingRequestSessions.add(mapped.session_id);
      let response;
      try {
        response = await runHook("PermissionRequest", mapped, true);
      } finally {
        pendingRequestSessions.delete(mapped.session_id);
      }
      const decision = response?.hookSpecificOutput?.decision;
      const reply = permissionReplyFromDecision(decision);
      if (!reply) return;
      await post(`/permission/${id}/reply`, { reply, message: decision.reason });
    }

    async function handleQuestion(mapped) {
      const id = mapped._opencode_request_id;
      pendingRequestSessions.add(mapped.session_id);
      let response;
      try {
        response = await runHook("PermissionRequest", mapped, true);
      } finally {
        pendingRequestSessions.delete(mapped.session_id);
      }
      const decision = response?.hookSpecificOutput?.decision;
      if (!decision) return;
      if (decision.behavior === "deny") {
        await post(`/question/${id}/reject`);
        return;
      }
      if (!decision.updatedInput?.answers) return;
      await post(`/question/${id}/reply`, {
        answers: answersFromDecision(mapped.tool_input.questions, decision),
      });
    }

    return {
      event: async ({ event }) => {
        const isReply = event.type === "permission.replied"
          || event.type === "question.replied" || event.type === "question.rejected"
          || event.type === "permission.v2.replied"
          || event.type === "question.v2.replied" || event.type === "question.v2.rejected";
        const mapped = mapEvent(event);
        if (!mapped) return;
        if (mapped.hook_event_name === "PermissionRequest" && mapped.tool_name === "AskUserQuestion") {
          handleQuestion(mapped).catch(() => {});
          return;
        }
        if (mapped.hook_event_name === "PermissionRequest") {
          handlePermission(mapped).catch(() => {});
          return;
        }
        if (!isReply && pendingRequestSessions.has(mapped.session_id)
            && mapped.hook_event_name !== "SessionStart"
            && mapped.hook_event_name !== "SessionEnd") {
          return;
        }
        await runHook(mapped.hook_event_name, mapped);
      },
    };
  },
};
"####;

// ── Pi / Oh My Pi ───────────────────────────────────────────────────────────

pub fn pi_extension(source: &str, marker: &str, hook_program: &str) -> String {
    fill(PI_TEMPLATE, source, marker, hook_program)
}

const PI_TEMPLATE: &str = r####"// codeisland-plugin: __MARKER__
// CodeIsland pi extension. Managed by CodeIsland; replaced on upgrade.
//
// pi (and Oh My Pi, which loads the same extension from ~/.omp) auto-discovers
// this file. Events are forwarded by spawning the one-shot hook client; a
// dangerous bash command and the `ask` tool are held for CodeIsland's answer.
import { execFile } from "node:child_process";

const HOOK = __HOOK__;
const SOURCE = __SOURCE__;
const BASE_ARGS = ["send", "--source", "__MARKER__", "--stdin"];

function runHook(event, payload, wait = false) {
  return new Promise((resolve) => {
    let child;
    try {
      child = execFile(
        HOOK,
        [...BASE_ARGS, "--event", event],
        { timeout: wait ? 86400000 : 30000, maxBuffer: 4 * 1024 * 1024 },
        (error, stdout) => {
          if (error) return resolve(null);
          const text = String(stdout || "").trim();
          if (!text) return resolve(null);
          try { resolve(JSON.parse(text)); } catch { resolve(null); }
        },
      );
    } catch {
      resolve(null);
      return;
    }
    try {
      child.stdin.write(JSON.stringify(payload));
      child.stdin.end();
    } catch {
      try { child.kill(); } catch {}
      resolve(null);
    }
  });
}

function isDangerous(command) {
  return /\brm\s+-rf\b|\bsudo\b|\bchmod\s+777\b/.test(String(command || ""));
}

function displayToolName(name) {
  const raw = String(name || "tool");
  return raw.charAt(0).toUpperCase() + raw.slice(1);
}

export default function codeislandExtension(pi) {
  const pending = new Set();
  const sessionIdOf = (ctx) => {
    const raw = ctx?.sessionManager?.getSessionId?.() ?? ctx?.sessionId ?? "";
    return `${SOURCE}-${raw}`;
  };
  const payloadFor = (ctx, extra) => ({
    session_id: sessionIdOf(ctx),
    cwd: ctx?.cwd || process.cwd(),
    _source: SOURCE,
    ...extra,
  });

  pi.on("session_start", async (_event, ctx) => {
    await runHook("SessionStart", payloadFor(ctx, { hook_event_name: "SessionStart" }));
  });

  pi.on("session_shutdown", async (_event, ctx) => {
    await runHook("SessionEnd", payloadFor(ctx, { hook_event_name: "SessionEnd" }));
  });

  pi.on("before_agent_start", async (event, ctx) => {
    if (pending.has(sessionIdOf(ctx))) return;
    await runHook("UserPromptSubmit", payloadFor(ctx, {
      hook_event_name: "UserPromptSubmit", prompt: event?.prompt ?? "",
    }));
  });

  // pi 0.80.4+ ends a turn on `agent_settled`; older builds only emit
  // `agent_end`. The first `agent_settled` just switches over (agent_end
  // already ended that turn), so no turn reports twice.
  let settles = false;
  pi.on("agent_end", async (_event, ctx) => {
    if (settles || pending.has(sessionIdOf(ctx))) return;
    await runHook("Stop", payloadFor(ctx, { hook_event_name: "Stop" }));
  });

  pi.on("agent_settled", async (event, ctx) => {
    if (!settles) {
      settles = true;
      return;
    }
    if (pending.has(sessionIdOf(ctx))) return;
    const aborted = event?.aborted === true;
    await runHook(aborted ? "Interrupt" : "Stop", payloadFor(ctx, {
      hook_event_name: aborted ? "Interrupt" : "Stop",
      ...(aborted ? { stop_reason: "interrupted" } : {}),
    }));
  });

  // `ask` tool -> CodeIsland's question card. pi awaits this handler, so the
  // tool can be held until the user answers (or falls through to pi's own
  // dialog when CodeIsland denies/skips or is unreachable).
  async function forwardAsk(event, ctx) {
    const rawQuestions = Array.isArray(event?.input?.questions) ? event.input.questions : [];
    if (rawQuestions.length === 0) return null;
    const questions = rawQuestions.map((q) => ({
      question: typeof q.question === "string" ? q.question : "Question",
      ...(typeof q.id === "string" && q.id ? { header: q.id } : {}),
      multiSelect: q.multi === true,
      options: Array.isArray(q.options)
        ? q.options
            .map((o) => ({
              label: typeof o.label === "string" ? o.label : "",
              ...(typeof o.description === "string" ? { description: o.description } : {}),
            }))
            .filter((o) => o.label)
        : [],
    }));
    // CodeIsland keys answers by question text, deduping repeats with `_2`,
    // `_3`… — reproduce that here so the answers map back to pi's ids.
    const used = new Set();
    const keys = questions.map(({ question }) => {
      let key = question;
      if (used.has(key)) {
        let suffix = 2;
        while (used.has(`${question}_${suffix}`)) suffix += 1;
        key = `${question}_${suffix}`;
      }
      used.add(key);
      return key;
    });

    const sid = sessionIdOf(ctx);
    pending.add(sid);
    let response = null;
    try {
      response = await runHook("PermissionRequest", payloadFor(ctx, {
        hook_event_name: "PermissionRequest",
        tool_name: "AskUserQuestion",
        tool_input: { questions },
      }), true);
    } finally {
      pending.delete(sid);
    }
    const decision = response?.hookSpecificOutput?.decision;
    if (decision?.behavior !== "allow") return null;
    const answers = decision.updatedInput?.answers || {};
    const lines = rawQuestions.map((q, index) => {
      const id = typeof q.id === "string" && q.id ? q.id : `q${index + 1}`;
      const value = answers[keys[index]];
      const text = Array.isArray(value)
        ? value.map(String).join(", ")
        : typeof value === "string" ? value : "";
      return `${id}: ${text || "(no answer)"}`;
    });
    return {
      block: true,
      reason:
        "The user already answered these questions through the CodeIsland desktop app. " +
        "Their answers:\n" + lines.join("\n") +
        "\nDo not ask again — proceed using these answers.",
    };
  }

  pi.on("tool_call", async (event, ctx) => {
    const sid = sessionIdOf(ctx);
    const toolName = displayToolName(event?.toolName);
    if (event?.toolName === "ask") {
      const answered = await forwardAsk(event, ctx);
      if (answered) return answered;
      return undefined;
    }
    const toolInput = { ...(event?.input || {}) };
    if (event?.toolName === "bash" && typeof event.input?.command === "string") {
      toolInput.patterns = [event.input.command];
      if (isDangerous(event.input.command)) {
        pending.add(sid);
        let response = null;
        try {
          response = await runHook("PermissionRequest", payloadFor(ctx, {
            hook_event_name: "PermissionRequest",
            tool_name: toolName,
            tool_input: toolInput,
          }), true);
        } finally {
          pending.delete(sid);
        }
        const decision = response?.hookSpecificOutput?.decision;
        if (decision?.behavior === "deny") {
          return { block: true, reason: "Blocked by CodeIsland" };
        }
      }
    }
    if (pending.has(sid)) return undefined;
    await runHook("PreToolUse", payloadFor(ctx, {
      hook_event_name: "PreToolUse", tool_name: toolName, tool_input: toolInput,
    }));
    return undefined;
  });

  pi.on("tool_result", async (_event, ctx) => {
    if (pending.has(sessionIdOf(ctx))) return;
    await runHook("PostToolUse", payloadFor(ctx, { hook_event_name: "PostToolUse" }));
  });

  pi.on("session_before_compact", async (_event, ctx) => {
    await runHook("PreCompact", payloadFor(ctx, { hook_event_name: "PreCompact" }));
  });

  pi.on("session_compact", async (_event, ctx) => {
    await runHook("PostCompact", payloadFor(ctx, { hook_event_name: "PostCompact" }));
  });
}
"####;

// ── OpenClaw ────────────────────────────────────────────────────────────────

pub fn openclaw_plugin(source: &str, marker: &str, hook_program: &str) -> String {
    fill(OPENCLAW_TEMPLATE, source, marker, hook_program)
}

const OPENCLAW_TEMPLATE: &str = r####"// codeisland-plugin: __MARKER__
// CodeIsland OpenClaw plugin. Managed by CodeIsland; replaced on upgrade.
//
// OpenClaw loads this in-process and calls `register(api)`; every hook is
// registered structurally, so a build that does not know a name simply never
// fires it. Events go to the one-shot hook client. Dangerous shell calls open
// a blocking PermissionRequest on the island and honour its deny decision.
import { execFile } from "node:child_process";

const HOOK = __HOOK__;
const SOURCE = __SOURCE__;
const BASE_ARGS = ["send", "--source", "__MARKER__", "--stdin"];

const DANGEROUS_PATTERNS = [
  /\brm\s+(-rf?|--recursive)/i,
  /\bsudo\b/i,
  /\b(chmod|chown)\b.*777/i,
];
const SHELL_TOOL_NAMES = new Set(["exec", "bash", "shell"]);

function runHook(event, payload, wait = false) {
  return new Promise((resolve) => {
    let child;
    try {
      child = execFile(
        HOOK,
        [...BASE_ARGS, "--event", event],
        { timeout: wait ? 86400000 : 30000, maxBuffer: 4 * 1024 * 1024 },
        (error, stdout) => {
          if (error) return resolve(null);
          const text = String(stdout || "").trim();
          if (!text) return resolve(null);
          try { resolve(JSON.parse(text)); } catch { resolve(null); }
        },
      );
    } catch {
      resolve(null);
      return;
    }
    try {
      child.stdin.write(JSON.stringify(payload));
      child.stdin.end();
    } catch {
      try { child.kill(); } catch {}
      resolve(null);
    }
  });
}

function isDangerous(command) {
  return DANGEROUS_PATTERNS.some((pattern) => pattern.test(command));
}

function displayToolName(name) {
  const raw = String(name || "tool");
  return raw.charAt(0).toUpperCase() + raw.slice(1);
}

// OpenClaw rotates the provider sessionId on /new and /reset while tool hooks
// only carry a sessionKey. Folding onto the agent id keeps one card per agent.
function laneOf(ctx) {
  if (ctx?.agentId) return ctx.agentId;
  const key = ctx?.sessionKey ?? "";
  if (key) {
    const parts = key.split(":");
    if (parts.length >= 2 && parts[0] === "agent" && parts[1]) return parts[1];
    return key;
  }
  return "main";
}

function payloadFor(lane, cwd, extra) {
  return {
    session_id: `${SOURCE}-${lane}`,
    cwd: cwd || process.cwd(),
    _source: SOURCE,
    _ppid: process.pid,
    ...extra,
  };
}

function register(api) {
  if (!api || typeof api.on !== "function") return;
  const on = api.on.bind(api);
  const startedLanes = new Set();
  const pendingPermissionLanes = new Set();
  const laneCwd = new Map();
  const lastPrompt = new Map();

  const cwdOf = (ctx, lane) => {
    const dir = ctx?.workspaceDir;
    if (typeof dir === "string" && dir.length > 0) {
      laneCwd.set(lane, dir);
      return dir;
    }
    return laneCwd.get(lane) ?? process.cwd();
  };

  const ensureSessionStarted = async (lane, cwd) => {
    if (startedLanes.has(lane)) return;
    startedLanes.add(lane);
    await runHook("SessionStart", payloadFor(lane, cwd, { hook_event_name: "SessionStart" }));
  };

  on("session_start", async (_event, ctx) => {
    const lane = laneOf(ctx);
    startedLanes.delete(lane);
    await ensureSessionStarted(lane, cwdOf(ctx, lane));
  });

  on("session_end", async (_event, ctx) => {
    const lane = laneOf(ctx);
    if (!startedLanes.has(lane)) return;
    startedLanes.delete(lane);
    await runHook("SessionEnd", payloadFor(lane, cwdOf(ctx, lane), { hook_event_name: "SessionEnd" }));
  });

  const onAgentRun = async (event, ctx) => {
    const lane = laneOf(ctx);
    const cwd = cwdOf(ctx, lane);
    await ensureSessionStarted(lane, cwd);
    if (pendingPermissionLanes.has(lane)) return;
    const prompt = typeof event?.prompt === "string"
      ? event.prompt
      : (event?.prompt?.text || event?.text || "");
    const previous = lastPrompt.get(lane);
    const now = Date.now();
    if (previous && previous.prompt === prompt && now - previous.at < 2000) return;
    lastPrompt.set(lane, { prompt, at: now });
    await runHook("UserPromptSubmit", payloadFor(lane, cwd, {
      hook_event_name: "UserPromptSubmit",
      prompt,
    }));
  };
  on("before_agent_start", onAgentRun);
  on("before_agent_run", onAgentRun);

  on("agent_end", async (event, ctx) => {
    const lane = laneOf(ctx);
    const cwd = cwdOf(ctx, lane);
    await ensureSessionStarted(lane, cwd);
    if (pendingPermissionLanes.has(lane)) return;
    const messages = Array.isArray(event?.messages) ? event.messages : [];
    let last = "";
    for (let index = messages.length - 1; index >= 0; index--) {
      const message = messages[index];
      if (message && message.role === "assistant" && Array.isArray(message.content)) {
        last = message.content
          .filter((part) => part && part.type === "text")
          .map((part) => part.text)
          .join("")
          .trim();
        if (last) break;
      }
    }
    if (!last && event?.success === false && event?.error) last = `Error: ${event.error}`;
    await runHook("Stop", payloadFor(lane, cwd, {
      hook_event_name: "Stop",
      last_assistant_message: last || undefined,
    }));
  });

  on("before_tool_call", async (event, ctx) => {
    const lane = laneOf(ctx);
    const cwd = cwdOf(ctx, lane);
    await ensureSessionStarted(lane, cwd);
    const rawName = String(event?.toolName ?? ctx?.toolName ?? "tool");
    const toolName = displayToolName(rawName);
    const params = event?.params && typeof event.params === "object" ? event.params : {};
    const toolInput = { ...params };
    const command = typeof params.command === "string"
      ? params.command
      : (typeof params.cmd === "string" ? params.cmd : undefined);
    if (SHELL_TOOL_NAMES.has(rawName) && command) toolInput.patterns = [command];
    if (typeof params.path === "string" && toolInput.file_path === undefined) {
      toolInput.file_path = params.path;
    }

    if (SHELL_TOOL_NAMES.has(rawName) && command && isDangerous(command)) {
      pendingPermissionLanes.add(lane);
      let response = null;
      try {
        response = await runHook("PermissionRequest", payloadFor(lane, cwd, {
          hook_event_name: "PermissionRequest",
          tool_name: toolName,
          tool_input: toolInput,
        }), true);
      } finally {
        pendingPermissionLanes.delete(lane);
      }
      const decision = response?.hookSpecificOutput?.decision;
      if (decision?.behavior === "deny") {
        return { block: true, blockReason: "Blocked by CodeIsland" };
      }
    }

    if (!pendingPermissionLanes.has(lane)) {
      await runHook("PreToolUse", payloadFor(lane, cwd, {
        hook_event_name: "PreToolUse",
        tool_name: toolName,
        tool_input: toolInput,
      }));
    }
    return undefined;
  });

  on("after_tool_call", async (_event, ctx) => {
    const lane = laneOf(ctx);
    const cwd = cwdOf(ctx, lane);
    await ensureSessionStarted(lane, cwd);
    if (pendingPermissionLanes.has(lane)) return;
    await runHook("PostToolUse", payloadFor(lane, cwd, { hook_event_name: "PostToolUse" }));
  });

  on("before_compaction", async (_event, ctx) => {
    const lane = laneOf(ctx);
    const cwd = cwdOf(ctx, lane);
    await ensureSessionStarted(lane, cwd);
    await runHook("PreCompact", payloadFor(lane, cwd, { hook_event_name: "PreCompact" }));
  });

  on("after_compaction", async (_event, ctx) => {
    const lane = laneOf(ctx);
    const cwd = cwdOf(ctx, lane);
    await ensureSessionStarted(lane, cwd);
    await runHook("PostCompact", payloadFor(lane, cwd, { hook_event_name: "PostCompact" }));
  });

  on("gateway_stop", async () => {
    const lanes = [...startedLanes];
    startedLanes.clear();
    await Promise.all(lanes.map((lane) =>
      runHook("SessionEnd", payloadFor(lane, laneCwd.get(lane) ?? process.cwd(), {
        hook_event_name: "SessionEnd",
      }))));
  });
}

export default {
  id: "codeisland",
  name: "CodeIsland",
  description: "Bridges OpenClaw agent activity into the CodeIsland island.",
  register,
};
"####;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::definition_for_source;

    #[test]
    fn opencode_plugin_embeds_the_hook_command_and_source() {
        let def = definition_for_source("opencode").expect("opencode definition");
        let text = plugin_source(def, "C:\\Program Files\\CodeIsland\\codeisland-hook.exe");
        assert!(text.contains("codeisland-plugin: codeisland-opencode"));
        assert!(text.contains("C:\\\\Program Files\\\\CodeIsland\\\\codeisland-hook.exe"));
        assert!(text.contains("__SOURCE__") == false);
        assert!(text.contains("\"opencode\""));
        assert!(text.contains("permission"));
        assert!(!text.contains("export default {}"));
    }

    #[test]
    fn mimo_reuses_the_opencode_plugin_with_its_own_source() {
        let def = definition_for_source("mimo").expect("mimo definition");
        let text = plugin_source(def, "codeisland-hook");
        assert!(text.contains("const SOURCE = \"mimo\";"));
        assert!(text.contains("codeisland-plugin: codeisland-mimo"));
    }

    #[test]
    fn opencode_plugin_supports_the_v2_setup_service() {
        let def = definition_for_source("opencode").expect("opencode definition");
        let text = plugin_source(def, "codeisland-hook");
        assert!(text.contains("setup: setupV2"));
        assert!(text.contains("createV2Runtime"));
        assert!(text.contains("session.tool.called"));
        assert!(text.contains("form.created"));
        assert!(text.contains("ctx.event.subscribe"));
    }

    #[test]
    fn pi_extension_routes_the_ask_tool_to_the_question_card() {
        let def = definition_for_source("pi").expect("pi definition");
        let text = plugin_source(def, "codeisland-hook");
        assert!(text.contains("forwardAsk"));
        assert!(text.contains("event?.toolName === \"ask\""));
        assert!(text.contains("AskUserQuestion"));
    }

    #[test]
    fn pi_and_openclaw_generate_real_extensions() {
        for source in ["pi", "omp", "openclaw"] {
            let def = definition_for_source(source).expect("definition");
            let text = plugin_source(def, "codeisland-hook");
            assert!(text.contains("runHook"), "{source} has no hook bridge");
            assert!(!text.contains("export default {}"), "{source} is still a stub");
        }
    }

    #[test]
    fn openclaw_gates_dangerous_shell_calls_on_the_island() {
        let def = definition_for_source("openclaw").expect("definition");
        let text = plugin_source(def, "codeisland-hook");
        assert!(text.contains("before_tool_call"));
        assert!(text.contains("PermissionRequest"));
        assert!(text.contains("blockReason"));
        assert!(text.contains("DANGEROUS_PATTERNS"));
        assert!(text.contains("pendingPermissionLanes"));
    }
}
