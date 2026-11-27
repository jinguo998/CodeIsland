use std::collections::HashSet;

use chrono::{Duration, Utc};
use codeisland_domain::*;

fn event(session: &str, kind: AgentEventType) -> AgentEvent {
    AgentEvent::new(session, AgentKind::Claude, kind)
}

#[test]
fn successful_claude_checklist_hooks_apply_provider_task_changes() {
    let mut store = SessionStore::default();
    for (name, raw, input, response) in [
        ("TaskCreate", "PostToolUse", serde_json::json!({"subject": "Run tests"}),
            serde_json::json!({"task": {"id": "7", "subject": "Run tests"}})),
        ("TaskUpdate", "PostToolUse", serde_json::json!({"taskId": "7", "status": "in_progress"}),
            serde_json::json!({"success": true, "taskId": "7", "statusChange": {"from": "pending", "to": "in_progress"}})),
        ("TaskUpdate", "PostToolUseFailure", serde_json::json!({"taskId": "7", "status": "deleted"}),
            serde_json::json!({"success": false})),
    ] {
        let mut event = event("checklist", AgentEventType::ToolEnd);
        event.tool = Some(name.to_owned());
        event.raw_event_name = Some(raw.to_owned());
        event.payload = Some(serde_json::json!({"tool_input": input, "tool_response": response}));
        assert!(store.apply(&event).applied);
    }
    let tasks = &store.get("checklist").unwrap().tasks;
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].id, "7");
    assert_eq!(tasks[0].title, "Run tests");
    assert_eq!(tasks[0].status, TaskStatus::InProgress);
}

#[test]
fn history_setting_limits_new_restored_and_existing_entries() {
    let mut store = SessionStore::new(8, 100);
    for index in 0..110 {
        let mut update = event("history", AgentEventType::Heartbeat);
        update.text = Some(index.to_string());
        assert!(store.apply(&update).applied);
    }
    let history = &store.get("history").unwrap().history;
    assert_eq!(history.len(), 100);
    assert_eq!(history[0].text.as_deref(), Some("10"));

    let mut restored = SessionStore::new(8, 20);
    restored.restore(store.persisted());
    let history = &restored.get("history").unwrap().history;
    assert_eq!(history.len(), 20);
    assert_eq!(history[0].text.as_deref(), Some("90"));

    store.apply_history_limit(10);
    let history = &store.get("history").unwrap().history;
    assert_eq!(history.len(), 10);
    assert_eq!(history[0].text.as_deref(), Some("100"));
    assert_eq!(history[9].text.as_deref(), Some("109"));
}

#[test]
fn codex_rollout_turn_recovery_preserves_requests_completion_and_newer_state() {
    let mut store = SessionStore::default();
    let modified = Utc::now();
    let path = "C:/work/rollout.jsonl";
    for (id, kind, timestamp) in [
        ("codexapp:waiting", AgentEventType::PermissionRequest, modified),
        ("codexapp:completed", AgentEventType::TurnEnd, modified),
        ("codexapp:newer", AgentEventType::UserPrompt, modified + Duration::seconds(1)),
    ] {
        let mut event = AgentEvent::new(id, AgentKind::Codex, kind);
        event.timestamp = timestamp;
        event.transcript_path = Some(path.to_owned());
        assert!(store.apply(&event).applied);
    }

    for running in [false, true] {
        assert!(!store.restore_codex_rollout_turn("codexapp:waiting", path, modified, running));
        assert!(!store.restore_codex_rollout_turn("codexapp:completed", path, modified, running));
    }
    assert!(!store.restore_codex_rollout_turn("codexapp:newer", path, modified, false));
    assert_eq!(store.get("codexapp:waiting").unwrap().status, SessionStatus::WaitingPermission);
    assert_eq!(store.get("codexapp:completed").unwrap().status, SessionStatus::Completed);
    assert_eq!(store.get("codexapp:newer").unwrap().status, SessionStatus::Running);
}

#[test]
fn wsl_process_evidence_does_not_use_windows_pid_or_replace_newer_activity() {
    let mut store = SessionStore::default();
    let path = r"\\wsl.localhost\Ubuntu\home\user\.claude\projects\p\example.jsonl";
    let mut prompt = event("example", AgentEventType::UserPrompt);
    prompt.transcript_path = Some(path.to_owned());
    assert!(store.apply(&prompt).applied);
    let process = WslProcess {
        distro: "Ubuntu".into(),
        pid: 72000,
        start_ticks: 735,
        tty: Some("/dev/pts/7".into()),
        windows_terminal_session: None,
        wezterm_pane: None,
    };
    assert!(!store.reconcile_wsl_process("example", Some("wrong"), Some(process.clone()), None, false));
    assert!(store.reconcile_wsl_process(
        "example",
        Some(path),
        Some(process.clone()),
        Some((SessionStatus::Idle, prompt.timestamp - Duration::seconds(1))),
        false,
    ));
    let running = store.get("example").unwrap();
    assert_eq!(running.status, SessionStatus::Running);
    assert_eq!(running.wsl_process.as_ref(), Some(&process));
    assert_eq!(running.pid, None);
    assert!(!store.settle_unmonitored_claude(prompt.timestamp + Duration::hours(1), None));

    let mut restored = SessionStore::default();
    restored.restore(store.persisted());
    assert!(restored.get("example").unwrap().wsl_process.is_none());

    assert!(store.reconcile_wsl_process("example", Some(path), None, None, true));
    let settled = store.get("example").unwrap();
    assert_eq!(settled.status, SessionStatus::Idle);
    assert!(settled.wsl_process.is_none());
}

#[test]
fn codex_hook_preview_is_transient_and_latched_to_the_active_turn() {
    let mut store = SessionStore::default();
    let codex = |kind| AgentEvent::new("remote-codex", AgentKind::Codex, kind);
    let mut prompt = codex(AgentEventType::UserPrompt);
    prompt.payload = Some(serde_json::json!({"last_assistant_message": "previous turn"}));
    store.apply(&prompt);
    assert_eq!(store.get("remote-codex").unwrap().live_codex_output, None);

    let mut tool = codex(AgentEventType::ToolStart);
    tool.payload = Some(serde_json::json!({"last_assistant_message": "  **Working**  "}));
    store.apply(&tool);
    let active = store.get("remote-codex").unwrap();
    assert_eq!(active.live_codex_output.as_deref(), Some("**Working**"));
    assert_eq!(active.last_message, None);
    let wire = serde_json::to_value(active).unwrap();
    assert_eq!(wire["live_codex_output"], "**Working**");
    let disk = serde_json::to_value(store.persisted()).unwrap();
    assert!(disk["sessions"]["remote-codex"].get("live_codex_output").is_none());

    store.apply(&codex(AgentEventType::Interrupt));
    let mut late = codex(AgentEventType::Message);
    late.payload = Some(serde_json::json!({"last_assistant_message": "stale"}));
    store.apply(&late);
    let cancelled = store.get("remote-codex").unwrap();
    assert_eq!(cancelled.status, SessionStatus::Cancelled);
    assert_eq!(cancelled.live_codex_output, None);
}

#[test]
fn permission_request_roundtrip() {
    let mut store = SessionStore::default();
    store.apply(&event("s1", AgentEventType::SessionStart));
    let mut tool = event("s1", AgentEventType::ToolStart);
    tool.tool = Some("Bash".into());
    store.apply(&tool);

    let mut request = event("s1", AgentEventType::PermissionRequest);
    request.tool = Some("Bash".into());
    request.text = Some("Allow npm test?".into());
    let outcome = store.apply(&request);

    assert!(outcome.applied);
    assert_eq!(store.get("s1").unwrap().status, SessionStatus::WaitingPermission);
    assert_eq!(store.pending_count(), 1);

    let decision = store
        .resolve(&request.id, UserAction::Approve, None)
        .expect("decision");
    assert_eq!(decision.action, UserAction::Approve);
    assert_eq!(store.pending_count(), 0);
    assert_eq!(store.get("s1").unwrap().status, SessionStatus::Running);
}

#[test]
fn permission_replay_preserves_queue_slot_and_dismissal_but_parallel_inputs_wait() {
    let mut store = SessionStore::default();
    let make_permission = |path: &str| {
        let mut request = event("s1", AgentEventType::PermissionRequest);
        request.tool = Some("Read".into());
        request.payload = Some(serde_json::json!({
            "tool_use_id": "shared",
            "tool_input": { "path": path }
        }));
        request
    };
    let original = make_permission("a");
    let unrelated = event("other", AgentEventType::PermissionRequest);
    store.apply(&original);
    store.apply(&unrelated);
    store.resolve(&original.id, UserAction::Hide, None);

    let replay = make_permission("a");
    store.apply(&replay);
    assert!(store.get_pending(&original.id).is_none());
    assert_eq!(store.pending().get_index(0).unwrap().0, &replay.id);
    assert!(store.get_pending(&replay.id).unwrap().hidden);

    let parallel = make_permission("b");
    store.apply(&parallel);
    assert_eq!(store.pending_count(), 3);
    assert!(!store.get_pending(&replay.id).unwrap().hidden);
}

#[test]
fn activity_retires_only_matching_request_and_never_resolves_a_question_as_approval() {
    let mut store = SessionStore::default();
    let mut question = event("shared", AgentEventType::Question);
    question.agent_id = Some("parent-agent".into());
    store.apply(&question);
    assert!(store.resolve(&question.id, UserAction::Approve, None).is_none());

    let mut sibling_tool = event("shared", AgentEventType::ToolStart);
    sibling_tool.agent_id = Some("child-agent".into());
    store.apply(&sibling_tool);
    assert!(store.get_pending(&question.id).is_some());

    let mut own_tool = event("shared", AgentEventType::ToolStart);
    own_tool.agent_id = question.agent_id;
    store.apply(&own_tool);
    assert!(store.get_pending(&question.id).is_none());
}

#[test]
fn always_allow_is_session_scoped() {
    let mut store = SessionStore::default();
    store.apply(&event("s1", AgentEventType::SessionStart));
    let mut request = event("s1", AgentEventType::PermissionRequest);
    request.agent = AgentKind::Gemini;
    request.tool = Some("Bash".into());
    store.apply(&request);
    store.resolve(&request.id, UserAction::AlwaysAllow, None);
    assert_eq!(
        store.get("s1").unwrap().always_allowed_tools,
        vec!["Bash".to_string()]
    );

    store.apply(&event("s1", AgentEventType::SessionStart));
    assert!(store.get("s1").unwrap().always_allowed_tools.is_empty());
}

#[test]
fn codex_always_uses_exact_displayed_prefixes_and_drains_covered_queue() {
    let mut store = SessionStore::default();
    let request = |session, script| {
        let mut event = AgentEvent::new(session, AgentKind::Codex, AgentEventType::PermissionRequest);
        event.tool = Some("Bash".into());
        event.payload = Some(serde_json::json!({ "tool_input": { "command": script } }));
        event
    };
    let initial = request("codex", "git status");
    let matching = request("codex", "git status --short");
    let unrelated = request("codex", "git push");
    let other_session = request("other", "git status");
    let scope = ApprovalAlwaysScope::CodexRules { patterns: vec![vec!["git".into(), "status".into()]] };
    store.apply_with_approval_scope(&initial, scope.clone(), None);
    for event in [&matching, &unrelated, &other_session] {
        store.apply(event);
    }

    let resolved = store.resolve_with_coverage(&initial.id, UserAction::AlwaysAllow, None).unwrap();
    assert_eq!(resolved.covered.len(), 1);
    assert_eq!(resolved.covered[0].event_id, matching.id);
    assert_eq!(resolved.covered[0].action, UserAction::Approve);
    assert!(store.get_pending(&unrelated.id).is_some());
    assert!(store.get_pending(&other_session.id).is_some());
    assert!(store.session_always_allows(&matching));
    assert!(!store.session_always_allows(&unrelated));
    assert!(!store.session_always_allows(&other_session));
    assert!(store.get("codex").unwrap().always_allowed_tools.is_empty());
}

#[test]
fn provider_owned_claude_always_keeps_asking_about_forwarded_exceptions() {
    let mut store = SessionStore::default();
    let mut first = event("claude", AgentEventType::PermissionRequest);
    first.tool = Some("Bash".into());
    store.apply(&first);
    store.resolve(&first.id, UserAction::AlwaysAllow, None);

    let mut exception = event("claude", AgentEventType::PermissionRequest);
    exception.tool = Some("Bash".into());
    assert!(!store.session_always_allows(&exception));
}

#[test]
fn question_options_and_answer() {
    let mut store = SessionStore::default();
    store.apply(&event("s1", AgentEventType::SessionStart));
    let mut question = event("s1", AgentEventType::Question);
    question.text = Some("Which environment?".into());
    question.payload = Some(serde_json::json!({
        "questions": [{ "question": "Which environment?", "options": [
            { "label": "staging" }, { "label": "production" }
        ]}]
    }));
    store.apply(&question);

    let pending = store.pending().values().next().unwrap();
    assert_eq!(pending.options, vec!["staging", "production"]);
    assert_eq!(store.get("s1").unwrap().status, SessionStatus::WaitingAnswer);

    let decision = store
        .resolve(&question.id, UserAction::Answer, Some("staging".into()))
        .expect("decision");
    assert_eq!(decision.text.as_deref(), Some("staging"));
}

#[test]
fn duplicate_and_stale_events_are_dropped() {
    let mut store = SessionStore::default();
    let start = event("s1", AgentEventType::SessionStart);
    assert!(store.apply(&start).applied);
    assert!(!store.apply(&start).applied);

    let mut first = event("s1", AgentEventType::UserPrompt);
    first.seq = Some(2);
    assert!(store.apply(&first).applied);
    let mut stale = event("s1", AgentEventType::Message);
    stale.seq = Some(1);
    stale.text = Some("stale".into());
    assert!(!store.apply(&stale).applied);
    assert!(store.get("s1").unwrap().last_message.is_none());
}

#[test]
fn needs_you_sessions_sort_first() {
    let mut store = SessionStore::default();
    store.apply(&event("a", AgentEventType::SessionStart));
    store.apply(&event("b", AgentEventType::SessionStart));
    let mut request = event("b", AgentEventType::PermissionRequest);
    request.tool = Some("Read".into());
    store.apply(&request);

    assert_eq!(store.ordered_ids().first().map(String::as_str), Some("b"));
}

#[test]
fn manual_order_breaks_ties_among_active_sessions() {
    let mut store = SessionStore::default();
    let base = Utc::now();
    let mut start_a = event("a", AgentEventType::SessionStart);
    start_a.timestamp = base;
    store.apply(&start_a);
    let mut prompt_a = event("a", AgentEventType::UserPrompt);
    prompt_a.text = Some("a".into());
    store.apply(&prompt_a);
    let mut start_b = event("b", AgentEventType::SessionStart);
    start_b.timestamp = base + Duration::seconds(1);
    store.apply(&start_b);
    let mut prompt_b = event("b", AgentEventType::UserPrompt);
    prompt_b.text = Some("b".into());
    store.apply(&prompt_b);

    // The newest working session wins until the user drags one ahead.
    assert_eq!(store.current_session().unwrap().session_id, "b");
    assert!(store.move_before("a", "b"));
    assert_eq!(store.current_session().unwrap().session_id, "a");
}

#[test]
fn expired_sessions_are_removed_but_waiting_ones_survive() {
    let mut store = SessionStore::default();
    let mut old = event("old", AgentEventType::SessionStart);
    old.timestamp = Utc::now() - Duration::hours(2);
    store.apply(&old);
    store.apply(&event("fresh", AgentEventType::SessionStart));
    let mut request = event("waiting", AgentEventType::PermissionRequest);
    request.timestamp = Utc::now() - Duration::hours(2);
    store.apply(&request);

    let removed = store.remove_expired(Utc::now() - Duration::hours(1));
    assert_eq!(removed, 1);
    assert!(store.get("old").is_none());
    assert!(store.get("fresh").is_some());
    assert!(store.get("waiting").is_some());
}

#[test]
fn restore_drops_pending_requests() {
    let mut store = SessionStore::default();
    store.apply(&event("s1", AgentEventType::SessionStart));
    let mut request = event("s1", AgentEventType::PermissionRequest);
    request.tool = Some("Bash".into());
    store.apply(&request);
    let persisted = store.persisted();

    let mut restored = SessionStore::default();
    restored.restore(persisted);
    assert_eq!(restored.pending_count(), 0);
    assert_eq!(restored.get("s1").unwrap().status, SessionStatus::Idle);
}

#[test]
fn attention_suppresses_children_and_quiet_hours() {
    let mut store = SessionStore::default();
    let mut start = event("child", AgentEventType::SessionStart);
    start.parent_session_id = Some("parent".into());
    let outcome = store.apply(&start);
    let subject = AttentionSubject {
        is_child: true,
        interrupted: false,
    };
    let intents = intents_for_event(&start, &outcome, subject);
    assert!(intents.sound.is_none());

    let mut context = AttentionContext {
        audible_cues: HashSet::from(["Stop".to_string()]),
        ..AttentionContext::default()
    };
    let end = event("s1", AgentEventType::TurnEnd);
    let outcome = store.apply(&end);
    let decision = decide(&end, &outcome, AttentionSubject::default(), &context);
    assert_eq!(decision.sound.as_deref(), Some("Stop"));

    context.quiet_hours = true;
    let decision = decide(&end, &outcome, AttentionSubject::default(), &context);
    assert!(decision.sound.is_none());
}

#[test]
fn completion_style_off_hides_completion_card() {
    let mut store = SessionStore::default();
    store.apply(&event("s1", AgentEventType::SessionStart));
    let end = event("s1", AgentEventType::TurnEnd);
    let outcome = store.apply(&end);
    let context = AttentionContext {
        completion_style: CompletionStyle::Off,
        ..AttentionContext::default()
    };
    let decision = decide(&end, &outcome, AttentionSubject::default(), &context);
    assert_eq!(
        decision.completion.unwrap().presentation,
        CompletionPresentation::StatusOnly
    );
    assert!(decision.follow_up.is_none());
}

#[test]
fn settings_are_clamped_and_shortcuts_normalized() {
    let settings = AppSettings {
        max_visible_sessions: 100,
        session_cleanup_minutes: 0,
        toggle_shortcut: "ctrl+shift+i".into(),
        display_mode: "invalid".into(),
        ..AppSettings::default()
    }
    .validate();
    assert_eq!(settings.max_visible_sessions, 20);
    // 0 means "never expire"; it is not clamped up to one minute.
    assert_eq!(settings.session_cleanup_minutes, 0);
    assert_eq!(settings.toggle_shortcut, "Ctrl+Shift+I");
    assert_eq!(settings.display_mode, "primary");
}

#[test]
fn session_limit_keeps_unlimited_and_legacy_values() {
    for (stored, effective) in [(0, 1), (7, 7), (20, 20), (99, 99), (100, 20)] {
        let settings = AppSettings { max_visible_sessions: stored, ..AppSettings::default() }.validate();
        assert_eq!(settings.max_visible_sessions, effective);
    }
    let defaults: AppSettings = serde_json::from_str("{}").unwrap();
    assert_eq!(defaults.max_visible_sessions, 5);
}

#[test]
fn quiet_hours_handle_midnight_windows() {
    use chrono::TimeZone;
    let at = |hour: u32| {
        chrono::Local
            .with_ymd_and_hms(2026, 11, 10, hour, 30, 0)
            .unwrap()
    };
    assert!(in_quiet_hours(at(23), "22:00", "08:00"));
    assert!(in_quiet_hours(at(2), "22:00", "08:00"));
    assert!(!in_quiet_hours(at(12), "22:00", "08:00"));
}

fn task(id: &str) -> AgentTask {
    AgentTask {
        id: id.to_owned(),
        title: format!("task {id}"),
        status: TaskStatus::InProgress,
    }
}

#[test]
fn subagent_events_do_not_disturb_the_parent() {
    let mut store = SessionStore::default();
    store.apply(&event("parent", AgentEventType::SessionStart));
    let mut prompt = event("parent", AgentEventType::UserPrompt);
    prompt.text = Some("do the thing".into());
    store.apply(&prompt);
    let mut tool = event("parent", AgentEventType::ToolStart);
    tool.tool = Some("Bash".into());
    store.apply(&tool);
    let mut request = event("parent", AgentEventType::PermissionRequest);
    request.agent = AgentKind::Gemini;
    request.tool = Some("Bash".into());
    store.apply(&request);
    store.resolve(&request.id, UserAction::AlwaysAllow, None);

    // Tasks are not carried by a hook event yet; seed one via persistence.
    let mut persisted = store.persisted();
    persisted
        .sessions
        .get_mut("parent")
        .unwrap()
        .tasks
        .push(task("t1"));
    let mut store = SessionStore::default();
    store.restore(persisted);
    // Restore clears tool chrome and always-allowed; put them back.
    let mut prompt = event("parent", AgentEventType::UserPrompt);
    prompt.text = Some("continue".into());
    store.apply(&prompt);
    let mut tool = event("parent", AgentEventType::ToolStart);
    tool.tool = Some("Bash".into());
    store.apply(&tool);
    let mut request = event("parent", AgentEventType::PermissionRequest);
    request.agent = AgentKind::Gemini;
    request.tool = Some("Bash".into());
    store.apply(&request);
    store.resolve(&request.id, UserAction::AlwaysAllow, None);

    let mut start = event("child-1", AgentEventType::SubagentStart);
    start.parent_session_id = Some("parent".into());
    start.agent_id = Some("agent-1".into());
    store.apply(&start);

    let parent = store.get("parent").unwrap();
    assert_eq!(parent.status, SessionStatus::Running);
    assert_eq!(parent.active_tool.as_deref(), Some("Bash"));
    assert_eq!(parent.always_allowed_tools, vec!["Bash".to_string()]);
    assert_eq!(parent.tasks.len(), 1);
    assert_eq!(parent.child_session_ids, vec!["child-1".to_string()]);

    let mut end = event("child-1", AgentEventType::SubagentEnd);
    end.parent_session_id = Some("parent".into());
    store.apply(&end);

    let parent = store.get("parent").unwrap();
    assert_eq!(parent.status, SessionStatus::Running);
    assert_eq!(parent.active_tool.as_deref(), Some("Bash"));
    assert_eq!(parent.always_allowed_tools, vec!["Bash".to_string()]);
    assert_eq!(parent.tasks.len(), 1);

    let child = store.get("child-1").unwrap();
    assert_eq!(child.status, SessionStatus::Completed);
    assert_eq!(child.parent_session_id.as_deref(), Some("parent"));
    assert_eq!(child.agent_id.as_deref(), Some("agent-1"));
}

#[test]
fn subagent_event_without_parent_is_a_noop() {
    let mut store = SessionStore::default();
    let outcome = store.apply(&event("orphan", AgentEventType::SubagentStart));
    assert!(outcome.applied);
    assert!(store.get("orphan").is_none());
    assert_eq!(store.session_count(), 0);

    // A self-referential parent is just as meaningless.
    let mut end = event("orphan", AgentEventType::SubagentEnd);
    end.parent_session_id = Some("orphan".into());
    store.apply(&end);
    assert!(store.get("orphan").is_none());
}

#[test]
fn orphan_subagent_end_is_silent() {
    let mut store = SessionStore::default();
    let end = event("orphan", AgentEventType::SubagentEnd);
    let outcome = store.apply(&end);
    let context = AttentionContext {
        audible_cues: HashSet::from(["Stop".to_string()]),
        push_enabled: true,
        ..AttentionContext::default()
    };
    let decision = decide(&end, &outcome, AttentionSubject::default(), &context);
    assert_eq!(decision, AttentionDecision::default());
}

#[test]
fn interrupted_turn_cancels_and_late_tool_events_are_ignored() {
    let mut store = SessionStore::default();
    store.apply(&event("s1", AgentEventType::SessionStart));
    let mut prompt = event("s1", AgentEventType::UserPrompt);
    prompt.text = Some("run it".into());
    store.apply(&prompt);
    let mut tool = event("s1", AgentEventType::ToolStart);
    tool.tool = Some("Bash".into());
    store.apply(&tool);

    let mut stop = event("s1", AgentEventType::TurnEnd);
    stop.stop_reason = Some("user".into());
    let outcome = store.apply(&stop);
    assert!(outcome.ended_turn);
    let session = store.get("s1").unwrap();
    assert_eq!(session.status, SessionStatus::Cancelled);
    assert!(session.interrupted);

    // Late hooks from the abandoned turn must not revive the card.
    let mut late_tool = event("s1", AgentEventType::ToolStart);
    late_tool.tool = Some("Read".into());
    store.apply(&late_tool);
    let mut late_end = event("s1", AgentEventType::ToolEnd);
    late_end.tool = Some("Read".into());
    store.apply(&late_end);
    let mut late_message = event("s1", AgentEventType::Message);
    late_message.text = Some("stale reply".into());
    store.apply(&late_message);
    let mut heartbeat = event("s1", AgentEventType::Heartbeat);
    heartbeat.tool = Some("background".into());
    store.apply(&heartbeat);

    let session = store.get("s1").unwrap();
    assert_eq!(session.status, SessionStatus::Cancelled);
    assert_eq!(session.active_tool, None);
    assert!(!session.is_executing_tool);
    assert_eq!(session.last_message, None);

    // A new prompt revives it.
    let mut next = event("s1", AgentEventType::UserPrompt);
    next.text = Some("again".into());
    store.apply(&next);
    let session = store.get("s1").unwrap();
    assert_eq!(session.status, SessionStatus::Running);
    assert!(!session.interrupted);
}

#[test]
fn interrupted_failed_turn_is_cancelled_not_failed() {
    let mut store = SessionStore::default();
    store.apply(&event("s1", AgentEventType::SessionStart));
    let mut failed = event("s1", AgentEventType::TurnFailed);
    failed.stop_reason = Some("aborted".into());
    failed.text = Some("Interrupted by user".into());
    let outcome = store.apply(&failed);
    assert!(outcome.ended_turn);
    assert!(!outcome.turn_failed);
    let session = store.get("s1").unwrap();
    assert_eq!(session.status, SessionStatus::Cancelled);
    assert!(session.interrupted);
    assert!(!session.last_turn_failed);
    assert_eq!(session.error, None);
}

#[test]
fn interrupted_turn_has_no_completion_attention() {
    let mut store = SessionStore::default();
    store.apply(&event("s1", AgentEventType::SessionStart));
    let mut stop = event("s1", AgentEventType::TurnEnd);
    stop.stop_reason = Some("interrupted".into());
    let outcome = store.apply(&stop);
    let context = AttentionContext {
        audible_cues: HashSet::from(["Stop".to_string(), "PermissionRequest".to_string()]),
        ..AttentionContext::default()
    };
    let decision = decide(&stop, &outcome, AttentionSubject::default(), &context);
    assert_eq!(decision, AttentionDecision::default());
}

#[test]
fn session_end_produces_no_attention() {
    let mut store = SessionStore::default();
    store.apply(&event("s1", AgentEventType::SessionStart));
    let end = event("s1", AgentEventType::SessionEnd);
    let outcome = store.apply(&end);
    assert!(outcome.applied);
    assert!(store.get("s1").is_none());
    let context = AttentionContext {
        audible_cues: HashSet::from(["Stop".to_string()]),
        push_enabled: true,
        ..AttentionContext::default()
    };
    let decision = decide(&end, &outcome, AttentionSubject::default(), &context);
    assert_eq!(decision, AttentionDecision::default());
}

#[test]
fn restore_lands_every_session_idle() {
    let mut store = SessionStore::default();
    store.apply(&event("parent", AgentEventType::SessionStart));
    let mut prompt = event("parent", AgentEventType::UserPrompt);
    prompt.text = Some("hello".into());
    store.apply(&prompt);
    let mut message = event("parent", AgentEventType::Message);
    message.text = Some("working on it".into());
    store.apply(&message);
    let mut tool = event("parent", AgentEventType::ToolStart);
    tool.tool = Some("Bash".into());
    store.apply(&tool);
    let mut request = event("parent", AgentEventType::PermissionRequest);
    request.tool = Some("Bash".into());
    store.apply(&request);
    store.resolve(&request.id, UserAction::AlwaysAllow, None);

    let mut child = event("child", AgentEventType::SessionStart);
    child.parent_session_id = Some("parent".into());
    store.apply(&child);
    let mut child_prompt = event("child", AgentEventType::UserPrompt);
    child_prompt.text = Some("subtask".into());
    store.apply(&child_prompt);

    let mut completed = event("done", AgentEventType::SessionStart);
    completed.timestamp = Utc::now() - Duration::minutes(5);
    store.apply(&completed);
    store.apply(&event("done", AgentEventType::TurnEnd));

    let persisted = store.persisted();
    let mut restored = SessionStore::default();
    restored.restore(persisted);

    assert_eq!(restored.pending_count(), 0);
    for session in restored.sessions().values() {
        assert_eq!(session.status, SessionStatus::Idle);
        assert_eq!(session.active_tool, None);
        assert_eq!(session.tool_use_id, None);
        assert!(!session.is_executing_tool);
        assert!(session.always_allowed_tools.is_empty());
        assert!(session.child_session_ids.is_empty());
        assert!(session.error.is_none());
        assert!(!session.interrupted);
        assert!(!session.last_turn_failed);
    }
    let parent = restored.get("parent").unwrap();
    assert_eq!(parent.last_prompt.as_deref(), Some("hello"));
    assert_eq!(parent.last_message.as_deref(), Some("working on it"));
    // Children stay as cards; only the parent's live list is cleared.
    assert!(restored.get("child").is_some());
    assert_eq!(
        restored.get("child").unwrap().parent_session_id.as_deref(),
        Some("parent")
    );
    assert!(restored.get("done").is_some());
}

#[test]
fn session_start_rebuilds_but_keeps_tasks() {
    let mut store = SessionStore::default();
    store.apply(&event("s1", AgentEventType::SessionStart));
    let mut prompt = event("s1", AgentEventType::UserPrompt);
    prompt.text = Some("old prompt".into());
    store.apply(&prompt);
    let mut message = event("s1", AgentEventType::Message);
    message.text = Some("old reply".into());
    store.apply(&message);
    let mut tool = event("s1", AgentEventType::ToolStart);
    tool.tool = Some("Bash".into());
    tool.payload = Some(serde_json::json!({ "tool_use_id": "tool-1" }));
    store.apply(&tool);
    let mut request = event("s1", AgentEventType::PermissionRequest);
    request.tool = Some("Bash".into());
    store.apply(&request);
    store.resolve(&request.id, UserAction::AlwaysAllow, None);
    let mut failed = event("s1", AgentEventType::Error);
    failed.text = Some("boom".into());
    store.apply(&failed);

    // Tasks are not carried by a hook event yet; seed one via persistence.
    let mut persisted = store.persisted();
    persisted.sessions.get_mut("s1").unwrap().tasks.push(task("t1"));
    let mut store = SessionStore::default();
    store.restore(persisted);
    // Rebuild the transient state restore intentionally clears.
    let mut prompt = event("s1", AgentEventType::UserPrompt);
    prompt.text = Some("new prompt".into());
    store.apply(&prompt);
    let mut message = event("s1", AgentEventType::Message);
    message.text = Some("new reply".into());
    store.apply(&message);
    let mut tool = event("s1", AgentEventType::ToolStart);
    tool.tool = Some("Bash".into());
    tool.payload = Some(serde_json::json!({ "tool_use_id": "tool-1" }));
    store.apply(&tool);
    let mut request = event("s1", AgentEventType::PermissionRequest);
    request.tool = Some("Bash".into());
    store.apply(&request);
    store.resolve(&request.id, UserAction::AlwaysAllow, None);
    let mut failed = event("s1", AgentEventType::Error);
    failed.text = Some("boom".into());
    store.apply(&failed);
    let mut child = event("child", AgentEventType::SessionStart);
    child.parent_session_id = Some("s1".into());
    store.apply(&child);

    let before = store.get("s1").unwrap();
    assert!(!before.history.is_empty());
    assert_eq!(before.tasks.len(), 1);
    assert_eq!(before.child_session_ids, vec!["child".to_string()]);

    store.apply(&event("s1", AgentEventType::SessionStart));
    let session = store.get("s1").unwrap();
    assert_eq!(session.status, SessionStatus::Idle);
    assert_eq!(session.last_prompt, None);
    assert_eq!(session.last_message, None);
    assert_eq!(session.active_tool, None);
    assert_eq!(session.tool_use_id, None);
    assert_eq!(session.error, None);
    assert!(!session.interrupted);
    assert!(!session.last_turn_failed);
    assert!(!session.is_executing_tool);
    assert!(session.child_session_ids.is_empty());
    assert!(session.history.is_empty());
    assert!(session.always_allowed_tools.is_empty());
    assert_eq!(session.tasks.len(), 1);
}

#[test]
fn new_session_start_is_idle_not_starting() {
    let mut store = SessionStore::default();
    store.apply(&event("fresh", AgentEventType::SessionStart));
    assert_eq!(store.get("fresh").unwrap().status, SessionStatus::Idle);
}

#[test]
fn needs_you_beats_manually_prioritized_active_session() {
    let mut store = SessionStore::default();
    store.apply(&event("active", AgentEventType::SessionStart));
    let mut prompt = event("active", AgentEventType::UserPrompt);
    prompt.text = Some("work".into());
    store.apply(&prompt);
    store.apply(&event("waiting", AgentEventType::SessionStart));
    let mut request = event("waiting", AgentEventType::PermissionRequest);
    request.tool = Some("Bash".into());
    store.apply(&request);

    assert!(store.move_before("active", "waiting"));
    assert_eq!(store.current_session().unwrap().session_id, "waiting");
}

#[test]
fn first_of_burst_only_rings_for_the_first_request_of_kind() {
    let mut store = SessionStore::default();
    assert!(first_of_burst(store.pending(), RequestKind::Permission));
    assert!(first_of_burst(store.pending(), RequestKind::Question));

    let mut first = event("s1", AgentEventType::PermissionRequest);
    first.tool = Some("Bash".into());
    store.apply(&first);
    assert!(!first_of_burst(store.pending(), RequestKind::Permission));
    assert!(first_of_burst(store.pending(), RequestKind::Question));

    let mut second = event("s2", AgentEventType::Question);
    second.text = Some("which?".into());
    store.apply(&second);
    assert!(!first_of_burst(store.pending(), RequestKind::Question));

    // Hidden requests do not count: the next visible one is still first.
    store.resolve(&first.id, UserAction::Hide, None);
    assert!(first_of_burst(store.pending(), RequestKind::Permission));
}

#[test]
fn new_settings_fields_default_and_clamp() {
    let defaults = AppSettings::default();
    assert!(defaults.auto_expand_on_permission);
    assert!(defaults.auto_expand_on_question);
    assert!(!defaults.hide_when_no_session);
    assert_eq!(defaults.hover_expand_delay_ms, 500);
    assert_eq!(defaults.sound_volume, 50);
    assert_eq!(defaults.mascot_speed, 100);
    assert_eq!(defaults.default_mascot, "claude");
    assert!(defaults.show_project_name);
    assert!(defaults.show_git_branch);
    assert!(!defaults.show_model_label);
    assert!(defaults.show_task_progress);
    assert!(defaults.show_session_recap);
    assert!(defaults.show_usage_stats);
    assert!(!defaults.show_claude_quota);
    assert!(!defaults.show_agent_details);
    assert!(defaults.show_tool_status);
    assert_eq!(defaults.content_font_size, 11);
    assert_eq!(defaults.ai_message_lines, 1);
    assert!(!defaults.allow_horizontal_drag);
    assert_eq!(defaults.display_choice, "auto");
    assert!(defaults.mute_when_away);
    assert!(!defaults.sound_enabled);
    assert_eq!(defaults.max_visible_sessions, 5);
    assert_eq!(defaults.group_by, "all");
    assert_eq!(defaults.codex_subsession_mode, "separate");

    let stored = serde_json::from_str::<AppSettings>(
        r#"{"content_font_size":13,"ai_message_lines":2,"show_agent_details":true,"show_model_label":true}"#,
    )
    .unwrap()
    .validate();
    assert_eq!(stored.content_font_size, 13);
    assert_eq!(stored.ai_message_lines, 2);
    assert!(stored.show_agent_details);
    assert!(stored.show_model_label);
    assert_eq!(stored.codex_subsession_mode, "separate");

    for mode in ["separate", "merge", "hide"] {
        let saved = serde_json::to_string(&AppSettings {
            codex_subsession_mode: mode.into(),
            ..AppSettings::default()
        }).unwrap();
        let restored = serde_json::from_str::<AppSettings>(&saved).unwrap().validate();
        assert_eq!(restored.codex_subsession_mode, mode);
    }

    let clamped = AppSettings {
        hover_expand_delay_ms: 1,
        sound_volume: 200,
        mascot_speed: 999,
        content_font_size: 30,
        ai_message_lines: 99,
        language: "fr".into(),
        group_by: "bogus".into(),
        codex_subsession_mode: "bogus".into(),
        display_choice: "  ".into(),
        ..AppSettings::default()
    }
    .validate();
    assert_eq!(clamped.hover_expand_delay_ms, 100);
    assert_eq!(clamped.sound_volume, 100);
    assert_eq!(clamped.mascot_speed, 300);
    assert_eq!(clamped.content_font_size, 18);
    assert_eq!(clamped.ai_message_lines, 10);
    assert_eq!(clamped.language, "system");
    assert_eq!(clamped.group_by, "all");
    assert_eq!(clamped.codex_subsession_mode, "separate");
    assert_eq!(clamped.display_choice, "auto");

    let accepted = AppSettings {
        language: "zh-Hant".into(),
        display_choice: "display:MONITOR-1".into(),
        group_by: "none".into(),
        ..AppSettings::default()
    }
    .validate();
    assert_eq!(accepted.language, "zh-Hant");
    assert_eq!(accepted.display_choice, "display:MONITOR-1");
    assert_eq!(accepted.group_by, "none");
}

#[test]
fn notch_animation_speed_matches_mac_defaults_and_validation() {
    for (stored, expected) in [
        (r#"{"sound_volume":37}"#, 1.0),
        (r#"{"sound_volume":37,"notch_animation_speed":null}"#, 1.0),
        (r#"{"sound_volume":37,"notch_animation_speed":"invalid"}"#, 1.0),
        (r#"{"sound_volume":37,"notch_animation_speed":0}"#, 0.5),
        (r#"{"sound_volume":37,"notch_animation_speed":3}"#, 2.0),
        (r#"{"sound_volume":37,"notch_animation_speed":1.37}"#, 1.37),
    ] {
        let settings: AppSettings = serde_json::from_str(stored).unwrap();
        let settings = settings.validate();
        assert_eq!(settings.notch_animation_speed, expected);
        assert_eq!(settings.sound_volume, 37);
    }

    for speed in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let settings = AppSettings {
            notch_animation_speed: speed,
            ..AppSettings::default()
        };
        assert_eq!(settings.validate().notch_animation_speed, 1.0);
    }
}

#[test]
fn notch_geometry_defaults_and_validation_match_mac() {
    let previous: AppSettings = serde_json::from_str(r#"{"sound_volume":37}"#).unwrap();
    assert_eq!(previous.collapsed_width_scale, 100);
    assert_eq!(previous.notch_height_mode, "matchNotch");
    assert_eq!(previous.custom_notch_height, 37.0);

    let invalid = AppSettings {
        collapsed_width_scale: 151,
        notch_height_mode: "invalid".into(),
        custom_notch_height: 0.0,
        ..previous
    }
    .validate();
    assert_eq!(invalid.collapsed_width_scale, 150);
    assert_eq!(invalid.notch_height_mode, "matchNotch");
    assert_eq!(invalid.custom_notch_height, 37.0);
    assert_eq!(invalid.sound_volume, 37);
}

#[test]
fn idle_removal_revalidates_whole_families_and_preserves_new_candidates() {
    let mut store = SessionStore::default();
    store.apply(&event("parent", AgentEventType::SessionStart));
    let mut child = event("child", AgentEventType::SessionStart);
    child.parent_session_id = Some("parent".into());
    store.apply(&child);
    let requested = store.idle_session_removal_groups();
    assert_eq!(requested, vec![vec!["child".to_owned(), "parent".to_owned()]]);

    let mut new_child = event("new-child", AgentEventType::SessionStart);
    new_child.parent_session_id = Some("parent".into());
    store.apply(&new_child);
    store.apply(&event("unrelated", AgentEventType::SessionStart));
    assert_eq!(store.remove_idle_sessions(requested, Utc::now()), 0);

    let requested = store.idle_session_removal_groups();
    let request = event("parent", AgentEventType::PermissionRequest);
    store.apply(&request);
    store.resolve(&request.id, UserAction::Hide, None);
    assert_eq!(store.remove_idle_sessions(requested, Utc::now()), 1);
    assert!(store.get_pending(&request.id).is_some());
    assert!(store.get("parent").is_some());
    assert!(store.get("child").is_some());
    assert!(store.get("new-child").is_some());
}

#[test]
fn explicitly_removed_idle_families_need_fresh_work_to_revive() {
    let mut store = SessionStore::default();
    store.apply(&event("parent", AgentEventType::SessionStart));
    let mut child = event("child", AgentEventType::SessionStart);
    child.parent_session_id = Some("parent".into());
    store.apply(&child);
    let now = Utc::now();
    assert_eq!(store.remove_idle_sessions(store.idle_session_removal_groups(), now), 2);

    let mut passive = event("parent", AgentEventType::Heartbeat);
    passive.timestamp = now + Duration::seconds(1);
    assert!(!store.apply(&passive).applied);

    let mut stale_work = event("child", AgentEventType::UserPrompt);
    stale_work.timestamp = now - Duration::seconds(1);
    assert!(!store.apply(&stale_work).applied);

    let mut work = event("parent", AgentEventType::UserPrompt);
    work.timestamp = now + Duration::seconds(2);
    assert!(store.apply(&work).applied);

    let mut child_returns = event("child", AgentEventType::Heartbeat);
    child_returns.timestamp = now + Duration::seconds(3);
    assert!(store.apply(&child_returns).applied);
}

#[test]
fn publishes_the_original_swift_model_label() {
    let mut store = SessionStore::default();
    let mut event = AgentEvent::new("model-card", AgentKind::Claude, AgentEventType::UserPrompt);
    event.model = Some("claude-sonnet-4-5".into());
    event.reasoning = Some("high".into());
    store.apply(&event);
    assert_eq!(store.snapshot().sessions[0].model_label.as_deref(), Some("Sonnet 4.5 · high"));
}
