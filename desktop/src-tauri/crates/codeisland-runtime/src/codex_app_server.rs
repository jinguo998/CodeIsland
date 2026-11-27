//! Codex Desktop's JSON-RPC client. Only requests originating on this
//! connection can be answered or withdrawn by it.

use std::collections::HashMap;
use std::io;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use codeisland_domain::{
    AgentEvent, AgentEventType, AgentKind, Decision, SessionStatus, UserAction,
    codex_desktop_session_id,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::task::JoinSet;

use crate::Runtime;

const INITIALIZE_ID: i64 = 1;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(untagged)]
enum RequestId {
    Number(i64),
    String(String),
}

impl RequestId {
    fn from_value(value: Option<&Value>) -> Option<Self> {
        match value? {
            Value::Number(number) => number.as_i64().map(Self::Number),
            Value::String(value) => Some(Self::String(value.clone())),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
struct RpcMessage {
    method: Option<String>,
    id: Option<RequestId>,
    params: Value,
    has_result: bool,
}

fn parse_message(line: &str) -> Option<RpcMessage> {
    codeisland_swift::call_bytes(c"codex.rpc", line.as_bytes())
}

struct Question {
    session_id: String,
    event_id: String,
    keys: Vec<String>,
}

struct Connection {
    runtime: Arc<Runtime>,
    stdin: ChildStdin,
    pending: HashMap<RequestId, Question>,
    replies: JoinSet<(RequestId, String, Option<Decision>)>,
}

impl Connection {
    async fn send(&mut self, body: Value) -> io::Result<()> {
        let mut bytes = serde_json::to_vec(&body)?;
        bytes.push(b'\n');
        self.stdin.write_all(&bytes).await?;
        self.stdin.flush().await
    }

    async fn receive(&mut self, envelope: RpcMessage) -> io::Result<()> {
        let Some(method) = envelope.method.as_deref() else { return Ok(()) };
        let params = &envelope.params;
        match (method, envelope.id) {
            ("item/tool/requestUserInput", Some(id)) => {
                if self.pending.contains_key(&id) {
                    return Ok(());
                }
                let Some((event, keys)) = question_event(params) else {
                    return self.answer(id, json!({ "answers": {} })).await;
                };
                let question = Question {
                    session_id: event.session_id.clone(),
                    event_id: event.id.clone(),
                    keys,
                };
                if let Some(reply) = self.runtime.queue_codex_desktop_question(event) {
                    let event_id = question.event_id.clone();
                    self.pending.insert(id.clone(), question);
                    self.replies.spawn(async move { (id, event_id, reply.await.ok()) });
                } else {
                    self.answer(id, json!({ "answers": {} })).await?;
                }
            }
            ("thread/started", None) => {
                let thread = &params["thread"];
                if let Some(id) = thread["id"].as_str() {
                    let mut event = thread_event(id, AgentEventType::SessionStart);
                    event.cwd = owned_string(&thread["cwd"]);
                    event.title = owned_string(&thread["name"]);
                    event.transcript_path = owned_string(&thread["path"]);
                    self.runtime.observe_codex_desktop_thread(
                        &event, thread_status(&thread["status"]), thread["preview"].as_str(),
                    );
                }
            }
            ("thread/status/changed", None) => {
                if let Some(id) = params["threadId"].as_str() {
                    let event = thread_event(id, AgentEventType::Heartbeat);
                    self.runtime.observe_codex_desktop_thread(&event, thread_status(&params["status"]), None);
                }
            }
            ("thread/closed", None) => {
                if let Some(id) = params["threadId"].as_str() {
                    let session_id = codex_desktop_session_id(id);
                    self.withdraw(&session_id, None);
                    self.runtime.close_codex_desktop_thread(&session_id);
                }
            }
            ("serverRequest/resolved", None) => {
                if let Some(id) = params["threadId"].as_str() {
                    let session_id = codex_desktop_session_id(id);
                    let request_id = RequestId::from_value(params.get("requestId"));
                    self.withdraw(&session_id, request_id.as_ref());
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn withdraw(&mut self, session_id: &str, request_id: Option<&RequestId>) {
        let mut cancelled = Vec::new();
        self.pending.retain(|id, question| {
            let keep = question.session_id != session_id || request_id.is_some_and(|wanted| wanted != id);
            if !keep {
                cancelled.push(question.event_id.clone());
            }
            keep
        });
        self.runtime.cancel_codex_desktop_questions(cancelled.iter().map(String::as_str));
    }

    async fn send_reply(
        &mut self, id: RequestId, event_id: &str, decision: Option<Decision>,
    ) -> io::Result<()> {
        if self.pending.get(&id).is_some_and(|question| question.event_id == event_id)
            && let Some(question) = self.pending.remove(&id)
        {
            let result = answer_result(&question.keys, decision.as_ref());
            self.answer(id, result).await?;
        }
        Ok(())
    }

    async fn answer(&mut self, id: RequestId, result: Value) -> io::Result<()> {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "result": result })).await
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.runtime.cancel_codex_desktop_questions(
            self.pending.values().map(|question| question.event_id.as_str()),
        );
    }
}

async fn run_client(
    runtime: Arc<Runtime>,
    mut command: Command,
    host_alive: impl Fn() -> bool,
) -> io::Result<()> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let stdin = child.stdin.take().ok_or_else(|| io::Error::other("Codex stdin missing"))?;
    let stdout = child.stdout.take().ok_or_else(|| io::Error::other("Codex stdout missing"))?;
    let mut lines = BufReader::new(stdout).lines();
    let mut connection = Connection {
        runtime,
        stdin,
        pending: HashMap::new(),
        replies: JoinSet::new(),
    };
    connection.send(json!({
        "jsonrpc": "2.0",
        "id": INITIALIZE_ID,
        "method": "initialize",
        "params": {
            "clientInfo": {
                "name": "CodeIsland",
                "version": env!("CARGO_PKG_VERSION"),
            },
            "capabilities": null,
        },
    })).await?;
    let initialized = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(line) = lines.next_line().await? {
            let Some(message) = parse_message(&line) else { continue };
            if message.id == Some(RequestId::Number(INITIALIZE_ID)) && message.method.is_none() {
                return Ok::<bool, io::Error>(message.has_result);
            }
        }
        Ok(false)
    }).await.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Codex initialize timed out"))??;
    if !initialized {
        return Err(io::Error::other("Codex app-server did not initialize"));
    }
    connection.send(json!({ "jsonrpc": "2.0", "method": "initialized" })).await?;

    let mut probe = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { break };
                if let Some(message) = parse_message(&line) {
                    connection.receive(message).await?;
                }
            }
            reply = connection.replies.join_next(), if !connection.replies.is_empty() => {
                if let Some(Ok((id, event_id, decision))) = reply {
                    connection.send_reply(id, &event_id, decision).await?;
                }
            }
            _ = probe.tick() => {
                if !host_alive() {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn thread_event(thread_id: &str, event_type: AgentEventType) -> AgentEvent {
    let mut event = AgentEvent::new(codex_desktop_session_id(thread_id), AgentKind::Codex, event_type);
    event.terminal_kind = Some("codex".into());
    event
}

fn owned_string(value: &Value) -> Option<String> {
    value.as_str().filter(|text| !text.is_empty()).map(ToOwned::to_owned)
}

fn thread_status(status: &Value) -> Option<SessionStatus> {
    codeisland_swift::call(c"codex.threadStatus", status)
}

fn question_event(params: &Value) -> Option<(AgentEvent, Vec<String>)> {
    let questions: Vec<Value> = codeisland_swift::call(c"codex.questions", params);
    let first = questions.first()?;
    let keys = questions.iter().filter_map(|row| owned_string(&row["answer_key"])).collect();
    let mut event = thread_event(params["threadId"].as_str()?, AgentEventType::Question);
    event.raw_event_name = Some("CodexAppServerQuestion".into());
    event.agent_id = Some("codex-app-server".into());
    event.tool = Some("request_user_input".into());
    event.text = first.get("question").and_then(owned_string);
    event.payload = Some(json!({ "questions": questions }));
    Some((event, keys))
}

fn answer_result(keys: &[String], decision: Option<&Decision>) -> Value {
    let mut answers = HashMap::new();
    if let Some(text) = decision.filter(|decision| decision.action == UserAction::Answer)
        .and_then(|decision| decision.text.as_deref())
    {
        let response = serde_json::from_str::<Map<String, Value>>(text).ok();
        for key in keys {
            let choice = match &response {
                Some(response) => response.get(key).and_then(Value::as_str),
                None if keys.len() == 1 => Some(text),
                None => None,
            };
            if let Some(choice) = choice.filter(|text| !text.is_empty()) {
                answers.insert(key, [choice.to_owned()]);
            }
        }
    }
    codeisland_swift::call(c"codex.questionResult", answers)
}

#[cfg(windows)]
pub(crate) async fn watch(runtime: Arc<Runtime>) {
    use std::time::Instant;

    let mut backoff = Duration::from_millis(500);
    let mut last_root = None;
    loop {
        let root = crate::codex_host::running_root();
        if last_root.as_ref() != Some(&root) {
            if root.is_none() || last_root.as_ref().is_some_and(Option::is_some) {
                runtime.set_codex_desktop_running(false);
            }
            if root.is_some() {
                runtime.set_codex_desktop_running(true);
            }
            last_root = Some(root.clone());
            backoff = Duration::from_millis(500);
        }

        let started = Instant::now();
        if let Some(path) = root.as_deref().and_then(crate::codex_host::executable) {
            let mut command = Command::new(path);
            command.args(["app-server", "--listen", "stdio://"]);
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
            if let Err(error) = run_client(Arc::clone(&runtime), command, || {
                crate::codex_host::running_root() == root
            }).await {
                tracing::debug!(%error, "Codex app-server connection ended");
            }
        }
        if started.elapsed() >= Duration::from_secs(10) {
            backoff = Duration::from_millis(500);
        }
        tokio::time::sleep(backoff).await;
        if root.is_some() {
            backoff = (backoff * 2).min(Duration::from_secs(10));
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use codeisland_domain::AppSettings;
    use std::sync::atomic::{AtomicBool, Ordering};

    const FIXTURE: &str = r#"
        IFS= read -r initialize || exit 1
        case "$initialize" in *'"method":"initialize"'*'"name":"CodeIsland"'*) ;; *) exit 2 ;; esac
        printf '%s\n' '{"id":1,"result":{"userAgent":"fixture"}}'
        IFS= read -r initialized || exit 3
        case "$initialized" in *'"method":"initialized"'*) ;; *) exit 4 ;; esac

        printf '%s\n' '{"method":"thread/started","params":{"thread":{"id":"native","cwd":"C:/work","preview":"Initial prompt","name":"Native thread","path":"C:/native.jsonl","status":{"type":"active"}}}}'
        printf '%s\n' '{"method":"item/tool/requestUserInput","id":"wizard","params":{"threadId":"native","questions":[{"id":"env","header":"Mode","question":"Where?","options":[{"label":"staging","description":"Preview before launch"}]},{"id":"token","header":"Mode","question":"Secret?","isSecret":true}]}}'
        IFS= read -r reply || exit 5
        printf '%s\n' "$reply" > "$1" || exit 6

        printf '%s\n' '{"method":"thread/closed","params":{"threadId":"native"}}'
        printf '%s\n' '{"method":"thread/started","params":{"thread":{"id":"keep","status":{"type":"active","activeFlags":["waitingOnApproval"]}}}}'
        printf '%s\n' '{"method":"item/tool/requestUserInput","id":7,"params":{"threadId":"keep","questions":[{"id":"resolved","question":"Was resolved in Codex?"}]}}'
        printf '%s\n' '{"method":"item/tool/requestUserInput","id":8,"params":{"threadId":"keep","questions":[{"id":"disconnected","question":"Dropped on disconnect?"}]}}'
        printf '%s\n' '{"method":"serverRequest/resolved","params":{"threadId":"keep","requestId":7}}'
        IFS= read -r unexpected && exit 7
        "#;

    async fn until<T>(mut value: impl FnMut() -> Option<T>) -> T {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(value) = value() {
                    break value;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }).await.expect("fixture timed out")
    }

    #[tokio::test]
    async fn initializes_answers_exact_wire_keys_and_withdraws_only_this_connection() {
        let runtime = crate::runtime::tests::runtime_with(AppSettings {
            collapsed: true,
            ..AppSettings::default()
        });
        runtime.set_codex_desktop_running(true);
        runtime.set_terminal_visibility_detector(|_| true);
        let cli = AgentEvent::new("native", AgentKind::Codex, AgentEventType::Question);
        runtime.handle_event(cli.clone());
        runtime.resolve(&cli.id, UserAction::Hide, None);

        let active = Arc::new(AtomicBool::new(true));
        let probe = Arc::clone(&active);
        let fixture = tempfile::tempdir().unwrap();
        let reply_path = fixture.path().join("reply.json");
        let mut command = Command::new("sh");
        command.args(["-c", FIXTURE, "codex-fixture"]).arg(&reply_path);
        let child = tokio::spawn(run_client(Arc::clone(&runtime), command, move || {
            probe.load(Ordering::Relaxed)
        }));

        let native = until(|| runtime.state().store.pending.into_iter()
            .find(|request| request.session_id == "codexapp:native")).await;
        assert_eq!(native.questions[0].answer_key.as_deref(), Some("env"));
        assert_eq!(native.questions[0].option_descriptions, ["Preview before launch"]);
        assert!(native.questions[1].is_secret);
        assert!(matches!(runtime.surface(), crate::IslandSurface::QuestionCard { .. }));
        let session = runtime.state().store.sessions.into_iter()
            .find(|session| session.session_id == "codexapp:native").unwrap();
        assert_eq!(session.title.as_deref(), Some("Native thread"));
        assert_eq!(session.transcript_path.as_deref(), Some("C:/native.jsonl"));
        assert_eq!(session.last_prompt.as_deref(), Some("Initial prompt"));
        assert!(runtime.resolve(
            &native.event_id,
            UserAction::Answer,
            Some(json!({ "env": "staging", "token": "test-only" }).to_string()),
        ).is_some());

        until(|| {
            let state = runtime.state();
            let keep: Vec<_> = state.store.pending.iter()
                .filter(|request| request.session_id == "codexapp:keep")
                .collect();
            (keep.len() == 1
                && keep[0].questions[0].answer_key.as_deref() == Some("disconnected")
                && !state.store.sessions.iter().any(|session| session.session_id == "codexapp:native"))
                .then_some(())
        }).await;
        let reply: Value = serde_json::from_slice(&std::fs::read(reply_path).unwrap()).unwrap();
        assert_eq!(reply, json!({
            "jsonrpc": "2.0", "id": "wizard",
            "result": { "answers": { "env": { "answers": ["staging"] }, "token": { "answers": ["test-only"] } } }
        }));
        assert!(!runtime.can_tail_codex_desktop("codexapp:native"));

        active.store(false, Ordering::Relaxed);
        child.await.unwrap().unwrap();
        let state = runtime.state();
        assert_eq!(state.store.pending.len(), 1);
        assert_eq!(state.store.pending[0].event_id, cli.id);
        assert!(state.store.sessions.iter().any(|session| session.session_id == "codexapp:keep"));
        runtime.set_codex_desktop_running(false);
        assert_eq!(runtime.state().store.sessions.len(), 1);
    }
}
