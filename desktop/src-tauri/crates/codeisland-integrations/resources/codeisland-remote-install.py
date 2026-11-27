# The desktop Swift build supplies the common functions from RemoteInstaller.configureRemoteHooksScript.
# These operations add per-host ownership, locking, atomic updates and reversible installation.
import json
import fcntl
import pathlib
import shlex
import shutil
import os
import re
import stat
import sys
import tempfile

home = pathlib.Path.home()
request = json.loads(sys.argv[1])
hook_path = pathlib.Path(request["hook_path"])
plugin_path = pathlib.Path(request["plugin_path"])
manifest_path = pathlib.Path(request["manifest_path"])
host_id = request["host_id"]
host_name = request["host_name"]
custom_clis = request.get("custom_clis", [])
unsupported_custom_clis = request.get("unsupported_custom_clis", [])

lock = manifest_path.with_suffix(".lock").open("a")
fcntl.flock(lock, fcntl.LOCK_EX)
manifest = json.loads(manifest_path.read_text(encoding="utf-8")) if manifest_path.exists() else {}
if manifest and manifest["host_id"] != host_id:
    sys.exit("CodeIsland remote installation belongs to a different host")
socket_path = request.get("socket_path") or manifest.get("socket_path", "")
installed_configs = manifest.get("configs", [])

# @codeisland-swift-remote-helpers@

def ensure_json(path):
    try:
        data = json.loads(path.read_text(encoding="utf-8")) if path.exists() else {}
    except (OSError, UnicodeError, ValueError):
        return None
    return data if isinstance(data, dict) else None

def write_json(path, data):
    write_text_atomic(path, json.dumps(data, indent=2, sort_keys=True) + "\n")

def write_text_atomic(path, text):
    path = path.resolve()
    path.parent.mkdir(parents=True, exist_ok=True)
    mode = stat.S_IMODE(path.stat().st_mode) if path.exists() else 0o600
    fd, tmp = tempfile.mkstemp(prefix="." + path.name + ".", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8", newline="") as stream:
            os.fchmod(stream.fileno(), mode)
            stream.write(text)
        os.replace(tmp, path)
    finally:
        try:
            os.unlink(tmp)
        except FileNotFoundError:
            pass

def save_manifest():
    write_json(manifest_path, {
        "host_id": host_id, "socket_path": socket_path, "configs": installed_configs,
    })

def record_config(path, format, source, key="hooks"):
    entry = {"path": str(path.resolve()), "format": format, "source": source, "key": key}
    if entry not in installed_configs:
        installed_configs.append(entry)
        save_manifest()

def command_for(source):
    # The agent runs this line in a shell: every value is quoted for it, so
    # a host named `$(...)` or with a backtick is a name, never a command run
    # on this host (double quotes, as json.dumps gave, let both run).
    assignments = [
        "CODEISLAND_SOCKET_PATH=" + shlex.quote(socket_path),
        "CODEISLAND_REMOTE_HOST_ID=" + shlex.quote(host_id),
        "CODEISLAND_REMOTE_HOST_NAME=" + shlex.quote(host_name),
        "CODEISLAND_SOURCE=" + shlex.quote(source),
    ]
    return " ".join(assignments + ["python3", shlex.quote(str(hook_path))])

def owns_command(command, source):
    if not isinstance(command, str):
        return False
    try:
        args = shlex.split(command)
    except ValueError:
        return False
    return (
        len(args) >= 2 and args[-2:] == ["python3", str(hook_path)]
        and "CODEISLAND_REMOTE_HOST_ID=" + host_id in args[:-2]
        and "CODEISLAND_SOCKET_PATH=" + socket_path in args[:-2]
        and "CODEISLAND_SOURCE=" + source in args[:-2]
    )

def remove_our_hooks(hooks, source):
    changed = False
    for event in list(hooks.keys()):
        entries = hooks.get(event)
        if not isinstance(entries, list):
            continue
        next_entries = []
        for entry in entries:
            if not isinstance(entry, dict):
                next_entries.append(entry)
                continue
            if any(owns_command(entry.get(key), source) for key in ("command", "bash")):
                continue
            children = entry.get("hooks")
            if isinstance(children, list):
                remaining = [
                    child for child in children if not isinstance(child, dict)
                    or not owns_command(child.get("command"), source)
                ]
                if len(remaining) != len(children):
                    if not remaining:
                        continue
                    entry = {**entry, "hooks": remaining}
            next_entries.append(entry)
        if next_entries == entries:
            continue
        changed = True
        if next_entries:
            hooks[event] = next_entries
        else:
            hooks.pop(event, None)
    return changed

TRAECLI_EVENTS = [
    ("session_start", 5),
    ("session_end", 5),
    ("user_prompt_submit", 5),
    ("pre_tool_use", 5),
    ("post_tool_use", 5),
    ("post_tool_use_failure", 5),
    ("permission_request", 86400),
    ("notification", 86400),
    ("subagent_start", 5),
    ("subagent_stop", 5),
    ("stop", 5),
    ("pre_compact", 5),
    ("post_compact", 5),
]

def _remove_managed_traecli_hooks(contents):
    lines = contents.split("\n")
    result = []
    in_hooks = False

    i = 0
    while i < len(lines):
        line = lines[i]
        stripped = line.strip()
        if line == stripped and ":" in line and not line.startswith("#"):
            in_hooks = line.startswith("hooks:")
        prefix = "- type: command"
        if in_hooks and stripped.startswith(prefix) and (stripped == prefix or stripped[len(prefix):].startswith((" ", "\t", "#"))):
            indent = len(line) - len(line.lstrip(" "))
            j = i + 1
            cmd_value = None
            while j < len(lines):
                nxt = lines[j]
                nxt_stripped = nxt.strip()
                nxt_indent = len(nxt) - len(nxt.lstrip(" "))
                if nxt_indent == indent and nxt_stripped.startswith("- "):
                    break
                if nxt_indent < indent and nxt_stripped != "":
                    break
                if nxt_stripped.startswith("command:"):
                    cmd_value = _parse_yaml_scalar(nxt_stripped.split(":", 1)[1])
                j += 1

            if owns_command(cmd_value, "traecli"):
                result.extend(line for line in lines[i:j] if line.lstrip().startswith("#"))
                i = j
                continue

            result.extend(lines[i:j])
            i = j
            continue

        result.append(line)
        i += 1
    # Trim trailing empty lines (keep one newline at end)
    while len(result) >= 2 and (result[-1] == "") and (result[-2] == ""):
        result.pop()
    return "\n".join(result)

def _merge_traecli_hooks(contents, cmd):
    normalized = _normalize_traecli_hooks_list_indentation(contents)
    cleaned = _remove_managed_traecli_hooks(normalized)
    lines = cleaned.split("\n")
    hooks_index = None
    hooks_scalar = None
    for i, line in enumerate(lines):
        stripped = line.strip()
        if line != stripped:
            continue
        if not stripped.startswith("hooks:"):
            continue
        tail = stripped[len("hooks:"):]
        before_comment = tail.split("#", 1)[0].strip()
        if before_comment in ("", "[]", "{}", "null", "~"):
            hooks_index = i
            hooks_scalar = before_comment
            break
    if hooks_index is not None:
        indent = _detect_traecli_hook_item_indent(lines, hooks_index)
        managed_lines = _render_managed_traecli_hooks(cmd, indent=indent).split("\n")
        if hooks_scalar and hooks_scalar != "":
            lines[hooks_index] = "hooks:"
        lines[hooks_index+1:hooks_index+1] = managed_lines
    else:
        if any(line.startswith("hooks:") for line in lines):
            raise ValueError("unsupported Trae CLI hooks value")
        managed_lines = _render_managed_traecli_hooks(cmd, indent=2).split("\n")
        while lines and lines[-1] == "":
            lines.pop()
        if lines:
            lines.append("")
        lines.append("hooks:")
        lines.extend(managed_lines)
    merged = "\n".join(lines)
    if not merged.endswith("\n"):
        merged += "\n"
    return merged

def install_claude():
    configured_root = _claude_config_dir()
    claude_root = configured_root or home / ".claude"
    # One guard for both cases, no early return: with $CLAUDE_CONFIG_DIR set, a host
    # where Claude Code is on PATH but has never run has no config dir yet, and it
    # must still get hooks (#271).
    if not claude_root.exists() and shutil.which("claude") is None:
        if configured_root is None:
            return "Claude skipped"
        return "Claude skipped (config dir not found: " + _display_path(claude_root) + ")"

    settings_path = claude_root / "settings.json"
    data = ensure_json(settings_path)
    if data is None or not isinstance(data.get("hooks", {}), dict):
        return "Claude config unreadable"
    hooks = data.get("hooks", {})
    remove_our_hooks(hooks, "claude")

    cmd = command_for("claude")
    without_matcher = [{"hooks": [{"type": "command", "command": cmd, "timeout": 60}]}]
    with_matcher = [{"matcher": "*", "hooks": [{"type": "command", "command": cmd, "timeout": 60}]}]
    with_long_timeout = [{"matcher": "*", "hooks": [{"type": "command", "command": cmd, "timeout": 86400}]}]
    precompact = [
        {"matcher": "auto", "hooks": [{"type": "command", "command": cmd, "timeout": 60}]},
        {"matcher": "manual", "hooks": [{"type": "command", "command": cmd, "timeout": 60}]},
    ]
    append_our_hooks(hooks, "UserPromptSubmit", without_matcher)
    append_our_hooks(hooks, "PermissionRequest", with_long_timeout)
    append_our_hooks(hooks, "Notification", with_matcher)
    append_our_hooks(hooks, "Stop", without_matcher)
    append_our_hooks(hooks, "SessionStart", without_matcher)
    append_our_hooks(hooks, "SessionEnd", without_matcher)
    append_our_hooks(hooks, "PreCompact", precompact)
    data["hooks"] = hooks
    record_config(settings_path, "json", "claude")
    write_json(settings_path, data)
    if configured_root is None:
        return "Claude ok"
    # Name the dir: it is the only way to see that $CLAUDE_CONFIG_DIR actually
    # reached the non-interactive login shell this script runs in.
    return "Claude ok (" + _display_path(claude_root) + ")"

def install_qoder():
    qoder_root = home / ".qoder"
    if not qoder_root.exists() and shutil.which("qodercli") is None and shutil.which("qoder") is None:
        return "Qoder skipped"

    settings_path = qoder_root / "settings.json"
    data = ensure_json(settings_path)
    if data is None or not isinstance(data.get("hooks", {}), dict):
        return "Qoder config unreadable"
    hooks = data.get("hooks", {})
    remove_our_hooks(hooks, "qoder")

    cmd = command_for("qoder")
    without_matcher = [{"hooks": [{"type": "command", "command": cmd, "timeout": 60}]}]
    with_matcher = [{"matcher": "*", "hooks": [{"type": "command", "command": cmd, "timeout": 60}]}]
    with_long_timeout = [{"matcher": "*", "hooks": [{"type": "command", "command": cmd, "timeout": 86400}]}]
    precompact = [
        {"matcher": "auto", "hooks": [{"type": "command", "command": cmd, "timeout": 60}]},
        {"matcher": "manual", "hooks": [{"type": "command", "command": cmd, "timeout": 60}]},
    ]
    append_our_hooks(hooks, "UserPromptSubmit", without_matcher)
    append_our_hooks(hooks, "PermissionRequest", with_long_timeout)
    append_our_hooks(hooks, "Notification", with_matcher)
    append_our_hooks(hooks, "Stop", without_matcher)
    append_our_hooks(hooks, "SessionStart", without_matcher)
    append_our_hooks(hooks, "SessionEnd", without_matcher)
    append_our_hooks(hooks, "PreCompact", precompact)
    data["hooks"] = hooks
    record_config(settings_path, "json", "qoder")
    write_json(settings_path, data)
    return "Qoder ok"

# Hermes (Nous Research) is NOT a Claude Code fork. It reads shell hooks from
# ~/.hermes/config.yaml under a `hooks:` MAP keyed by snake_case event names whose
# values are lists of { command, timeout }. settings.json is never parsed (#226).
def hermes_command():
    # Hermes runs a hook command without a shell: shlex.split, then exec
    # (agent/shell_hooks.py). command_for()'s `VAR=value python3 ~/...` form was
    # taken for a program named `CODEISLAND_SOCKET_PATH=...` and never ran, and
    # `~` is never expanded. `env` sets the variables; the hook path is absolute.
    args = [
        "env",
        "CODEISLAND_SOCKET_PATH=" + socket_path,
        "CODEISLAND_REMOTE_HOST_ID=" + host_id,
        "CODEISLAND_REMOTE_HOST_NAME=" + host_name,
        "CODEISLAND_SOURCE=hermes",
        "python3",
        str(hook_path),
    ]
    return " ".join(shlex.quote(a) for a in args)

HERMES_EVENTS = [
    ("pre_tool_call", 5),
    ("post_tool_call", 5),
    ("pre_llm_call", 5),
    ("post_llm_call", 5),
    ("on_session_start", 5),
    ("on_session_end", 5),
    ("on_session_finalize", 5),
    ("subagent_stop", 5),
]

def _merge_hermes_hooks(contents, cmd=None):
    # Event-key-aware merge for Hermes' nested `hooks:` MAP. We parse the existing
    # map into an ordered {event: [item-blocks]} structure, drop our prior managed
    # entries (matched by command), prepend a fresh managed entry under each of our
    # status events, then re-emit a single `hooks:` block. This is idempotent and
    # never produces duplicate event keys (#226).
    normalized = contents.replace("\r\n", "\n")
    lines = normalized.split("\n")

    def _is_top_level_key(line):
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            return False
        if line != stripped:
            return False
        return ":" in stripped

    # Locate a top-level `hooks:` mapping key (empty scalar or block-mapping).
    hooks_index = None
    for i, line in enumerate(lines):
        stripped = line.strip()
        if line != stripped:
            continue
        if stripped == "hooks:" or stripped.startswith("hooks:"):
            tail = stripped[len("hooks:"):].split("#", 1)[0].strip()
            if tail in ("", "{}", "null", "~"):
                hooks_index = i
                break

    # Ordered list of (event, [item_blocks]); item_block is a list of raw lines
    # for one `- ...` list entry, re-indented to 4 spaces on emit.
    order = []
    events = {}
    removed = False

    def _ensure_event(ev):
        if ev not in events:
            events[ev] = []
            order.append(ev)

    head_lines = []
    tail_lines = []
    if hooks_index is not None:
        head_lines = lines[:hooks_index]
        j = hooks_index + 1
        current_event = None
        while j < len(lines):
            line = lines[j]
            stripped = line.strip()
            if stripped and _is_top_level_key(line):
                break  # left the hooks: block
            if stripped == "" or stripped.startswith("#"):
                j += 1
                continue
            indent = len(line) - len(line.lstrip(" "))
            # An event sub-key like "  pre_tool_call:".
            if not stripped.startswith("- ") and stripped.endswith(":") and indent >= 1:
                current_event = stripped[:-1].strip()
                _ensure_event(current_event)
                j += 1
                continue
            # A list item under the current event.
            if stripped.startswith("- ") and current_event is not None:
                item_indent = indent
                k = j + 1
                while k < len(lines):
                    nxt = lines[k]
                    nxt_stripped = nxt.strip()
                    nxt_indent = len(nxt) - len(nxt.lstrip(" "))
                    if nxt_stripped == "":
                        k += 1
                        continue
                    if nxt_indent <= item_indent:
                        break
                    k += 1
                raw_block = [bl for bl in lines[j:k] if bl.strip() != ""]
                cmd_value = None
                for bl in raw_block:
                    bstr = bl.strip()
                    key = bstr[2:].strip() if bstr.startswith("- ") else bstr
                    if key.startswith("command:"):
                        cmd_value = _parse_yaml_scalar(key.split(":", 1)[1])
                        break
                # Re-indent the user item to a canonical 4-space list indent so it
                # composes with our managed entries (avoids mixed-indent YAML).
                base = min(len(bl) - len(bl.lstrip(" ")) for bl in raw_block)
                block = []
                for bl in raw_block:
                    cur = len(bl) - len(bl.lstrip(" "))
                    block.append(" " * (4 + (cur - base)) + bl.lstrip(" "))
                # Drop our prior managed entries; keep user entries.
                if not owns_command(cmd_value, "hermes"):
                    events[current_event].append(block)
                else:
                    removed = True
                j = k
                continue
            j += 1
        tail_lines = lines[j:]
    else:
        if cmd is not None and any(line.startswith("hooks:") for line in lines):
            raise ValueError("unsupported Hermes hooks value")
        head_lines = list(lines)
        while head_lines and head_lines[-1] == "":
            head_lines.pop()

    if cmd is None and not removed:
        return contents
    if cmd is not None:
        escaped = cmd.replace("'", "''")
        for (event, timeout) in HERMES_EVENTS:
            _ensure_event(event)
            events[event].insert(0, [f"    - command: '{escaped}'", f"      timeout: {timeout}"])

    out = list(head_lines)
    if any(events.values()):
        if out and out[-1].strip() != "":
            out.append("")
        out.append("hooks:")
        for event in order:
            if events[event]:
                out.append(f"  {event}:")
                for block in events[event]:
                    out.extend(block)
    out.extend(tail_lines)

    merged = "\n".join(out)
    if not merged.endswith("\n"):
        merged += "\n"
    return merged

def install_hermes():
    hermes_root = home / ".hermes"
    if not hermes_root.exists() and shutil.which("hermes") is None:
        return "Hermes skipped"

    config_path = hermes_root / "config.yaml"
    try:
        original = config_path.read_text(encoding="utf-8") if config_path.exists() else ""
    except Exception:
        return "Hermes read failed"
    cmd = hermes_command()
    merged = _merge_hermes_hooks(original, cmd)
    record_config(config_path, "hermes", "hermes")
    write_text_atomic(config_path, merged)
    return "Hermes ok"

# Codex TOML editing (#354).
def ensure_toml_codex_hooks(path):
    try:
        # newline='' avoids read_text's universal-newline normalization.
        with path.open('r', encoding='utf-8', newline='') as stream:
            content = stream.read()
    except FileNotFoundError:
        content = ''
    except (OSError, UnicodeError):
        return False
    result = _codex_hooks_toml(content)
    if result is None:
        return False
    if result != content:
        try:
            write_text_atomic(path, result)
        except (OSError, UnicodeError, RuntimeError):
            return False
    return True

def install_codex():
    codex_root = _codex_home()
    if not codex_root.exists() and shutil.which("codex") is None:
        return "Codex skipped"

    hooks_path = codex_root / "hooks.json"
    data = ensure_json(hooks_path)
    if data is None or not isinstance(data.get("hooks", {}), dict):
        return "Codex config unreadable"
    hooks = data.get("hooks", {})
    remove_our_hooks(hooks, "codex")

    cmd = command_for("codex")
    entry = [{"hooks": [{"type": "command", "command": cmd, "timeout": 60}]}]
    # PermissionRequest blocks on a human, so it receives the long timeout. The
    # other entries mirror Codex's complete public lifecycle surface, keeping
    # remote status fidelity identical to local sessions.
    blocking_entry = [{"hooks": [{"type": "command", "command": cmd, "timeout": 86400}]}]
    # Codex caps SessionEnd / Interrupt hooks at 3 s and warns about any
    # longer configured timeout, so match the local installer's 3 s there.
    teardown_entry = [{"hooks": [{"type": "command", "command": cmd, "timeout": 3}]}]
    for event in [
        "PreToolUse", "PostToolUse", "PreCompact", "PostCompact",
        "SessionStart", "SubagentStart", "SubagentStop",
        "UserPromptSubmit", "Stop",
    ]:
        append_our_hooks(hooks, event, entry)
    append_our_hooks(hooks, "PermissionRequest", blocking_entry)
    append_our_hooks(hooks, "SessionEnd", teardown_entry)
    append_our_hooks(hooks, "Interrupt", teardown_entry)
    data["hooks"] = hooks
    record_config(hooks_path, "json", "codex")
    write_json(hooks_path, data)
    if not ensure_toml_codex_hooks(codex_root / "config.toml"):
        return "Codex config update failed"
    return "Codex ok"

def install_codebuddy():
    codebuddy_root = home / ".codebuddy"
    if not codebuddy_root.exists() and shutil.which("codebuddy") is None:
        return "CodeBuddy skipped"

    settings_path = codebuddy_root / "settings.json"
    data = ensure_json(settings_path)
    if data is None or not isinstance(data.get("hooks", {}), dict):
        return "CodeBuddy config unreadable"
    hooks = data.get("hooks", {})
    remove_our_hooks(hooks, "codebuddy")

    cmd = command_for("codebuddy")
    without_matcher = [{"hooks": [{"type": "command", "command": cmd, "timeout": 60}]}]
    with_matcher = [{"matcher": "*", "hooks": [{"type": "command", "command": cmd, "timeout": 60}]}]
    with_long_timeout = [{"matcher": "*", "hooks": [{"type": "command", "command": cmd, "timeout": 86400}]}]
    precompact = [
        {"matcher": "auto", "hooks": [{"type": "command", "command": cmd, "timeout": 60}]},
        {"matcher": "manual", "hooks": [{"type": "command", "command": cmd, "timeout": 60}]},
    ]
    append_our_hooks(hooks, "UserPromptSubmit", without_matcher)
    append_our_hooks(hooks, "PermissionRequest", with_long_timeout)
    append_our_hooks(hooks, "Notification", with_matcher)
    append_our_hooks(hooks, "Stop", without_matcher)
    append_our_hooks(hooks, "SessionStart", without_matcher)
    append_our_hooks(hooks, "SessionEnd", without_matcher)
    append_our_hooks(hooks, "PreCompact", precompact)
    data["hooks"] = hooks
    record_config(settings_path, "json", "codebuddy")
    write_json(settings_path, data)
    return "CodeBuddy ok"

def install_traecli():
    traecli_root = home / ".trae"
    if not traecli_root.exists() and shutil.which("traecli") is None:
        return "Traecli skipped"

    config_path = traecli_root / "traecli.yaml"
    try:
        original = config_path.read_text(encoding="utf-8") if config_path.exists() else ""
    except Exception:
        return "Traecli read failed"
    cmd = command_for("traecli")
    merged = _merge_traecli_hooks(original, cmd)
    record_config(config_path, "traecli", "traecli")
    write_text_atomic(config_path, merged)
    return "Traecli ok"

def install_opencode():
    opencode_root = home / ".config" / "opencode"
    if not opencode_root.exists() and shutil.which("opencode") is None:
        return "OpenCode skipped"

    if not plugin_path.is_file():
        return "OpenCode plugin missing"

    target_path = opencode_root / "opencode.jsonc"
    if not target_path.exists():
        target_path = opencode_root / "opencode.json"
    data = ensure_jsonc_object(target_path)
    if data is None or not isinstance(data.get("plugin", []), list):
        return "OpenCode config unreadable"

    plugin_ref = "file://" + str(plugin_path)
    plugins = [plugin for plugin in data.get("plugin", []) if plugin != plugin_ref]
    plugins.append(plugin_ref)
    data["plugin"] = plugins
    data.setdefault("$schema", "https://opencode.ai/config.json")
    record_config(target_path, "opencode", "opencode", "plugin")
    write_opencode_config(target_path, data)
    return "OpenCode ok"

def install_custom():
    results = []
    for cli in custom_clis:
        source = cli["source"]
        # config_path arrives home-relative (or absolute) — the Mac side already
        # stripped `~/` and its own home prefix so this resolves against the REMOTE
        # $HOME (#342).
        config_path = home / cli["config_path"]
        if not config_path.parent.exists() and shutil.which(source) is None:
            results.append(cli["name"] + " skipped (config dir not found: " + _display_path(config_path.parent) + ")")
            continue
        data = ensure_json(config_path)
        if data is None or not isinstance(data.get(cli["config_key"], {}), dict):
            results.append(cli["name"] + " config unreadable")
            continue
        hooks = data.get(cli["config_key"], {})
        remove_our_hooks(hooks, source)
        cmd = command_for(source)
        for ev in cli["events"]:
            event = ev[0]
            timeout = ev[1]
            if cli["format"] == "claude":
                append_our_hooks(hooks, event, [{"matcher": "*", "hooks": [{"type": "command", "command": cmd, "timeout": timeout}]}])
            else:
                append_our_hooks(hooks, event, [{"hooks": [{"type": "command", "command": cmd, "timeout": timeout}]}])
        data[cli["config_key"]] = hooks
        record_config(config_path, "json", source, cli["config_key"])
        write_json(config_path, data)
        results.append(cli["name"] + " ok")
    for name in unsupported_custom_clis:
        results.append(name + " skipped (template not supported remotely)")
    return results

def remove_installed_config(entry):
    path = pathlib.Path(entry["path"])
    if not path.exists():
        return
    key = entry["key"]
    format = entry["format"]
    if format in ("hermes", "traecli"):
        original = path.read_text(encoding="utf-8")
        cleaned = _merge_hermes_hooks(original) if format == "hermes" else _remove_managed_traecli_hooks(original)
        if cleaned != original:
            write_text_atomic(path, cleaned)
        return
    if format == "opencode":
        data = ensure_jsonc_object(path)
        if data is None or not isinstance(data.get(key, []), list):
            raise ValueError("OpenCode config unreadable")
        plugins = data.get(key, [])
        cleaned = [value for value in plugins if value != "file://" + str(plugin_path)]
        if cleaned != plugins:
            data[key] = cleaned
            write_opencode_config(path, data)
        return
    data = ensure_json(path)
    if data is None or not isinstance(data.get(key, {}), dict):
        raise ValueError("hooks config unreadable")
    hooks = data.get(key, {})
    if remove_our_hooks(hooks, entry["source"]):
        if not hooks:
            data.pop(key)
        write_json(path, data)

def uninstall():
    if not manifest:
        report("No managed hooks installed")
        return
    failed = []
    for entry in installed_configs:
        try:
            remove_installed_config(entry)
        except (OSError, UnicodeError, ValueError) as error:
            failed.append(entry)
            print(f'{entry["path"]}: {error}', file=sys.stderr)
    if failed:
        installed_configs[:] = failed
        save_manifest()
        sys.exit(1)
    try:
        plugin_path.unlink()
    except FileNotFoundError:
        pass
    manifest_path.unlink()
    report("Managed hooks removed")

def verify_socket():
    path = pathlib.Path(socket_path)
    folder = path.parent.lstat()
    info = path.lstat()
    if (
        not stat.S_ISDIR(folder.st_mode) or folder.st_uid != os.getuid()
        or stat.S_IMODE(folder.st_mode) != 0o700
        or not stat.S_ISSOCK(info.st_mode) or info.st_uid != os.getuid()
    ):
        sys.exit("CodeIsland reverse socket is not owned by the SSH user")

def install():
    verify_socket()
    parts = []
    for label, installer in [
        ("Claude", install_claude), ("Qoder", install_qoder), ("Hermes", install_hermes),
        ("Codex", install_codex), ("CodeBuddy", install_codebuddy),
        ("Trae CLI", install_traecli), ("OpenCode", install_opencode), ("Custom CLI", install_custom),
    ]:
        try:
            result = installer()
            parts.extend(result if isinstance(result, list) else [result])
        except (OSError, UnicodeError, ValueError) as error:
            parts.append(f"{label} config failed ({error})")
    report(" · ".join(parts))

def report(summary):
    print(json.dumps({"codeisland_summary": summary}))

if request["operation"] == "install":
    install()
elif request["operation"] == "uninstall":
    uninstall()
else:
    sys.exit("Unknown CodeIsland remote installer operation")
