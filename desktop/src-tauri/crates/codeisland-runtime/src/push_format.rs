use codeisland_domain::{
    AgentEvent, AgentKind, PendingRequest, PushKind, SessionSnapshot, push_text::clean_push_text,
};

pub(crate) struct PushWords {
    permission: &'static str,
    question: &'static str,
    completion: &'static str,
    error: &'static str,
    pub reminder: &'static str,
    pub minutes: &'static str,
    secret: &'static str,
    more_options: &'static str,
}

pub(crate) fn push_title(
    kind: PushKind,
    agent: AgentKind,
    project: Option<&str>,
    reminder: bool,
) -> String {
    let marker = if reminder {
        "⏰"
    } else {
        match kind {
            PushKind::Approval => "🔐",
            PushKind::Question | PushKind::DisplayOnlyWait => "❓",
            PushKind::Completion => "✅",
            PushKind::Error => "❌",
        }
    };
    let title = format!("{marker} {}", agent.display_name());
    match project {
        Some(project) => format!("{title} · {}", clean_push_text(project, 80, false)),
        None => title,
    }
}

pub(crate) fn push_content(
    kind: PushKind,
    request: Option<&PendingRequest>,
    session: Option<&SessionSnapshot>,
    event: Option<&AgentEvent>,
    words: &PushWords,
    limit: usize,
) -> (String, String) {
    let text = || {
        event
            .and_then(|event| event.text.as_deref())
            .or_else(|| session.and_then(|session| session.last_message.as_deref()))
            .unwrap_or_default()
    };
    match kind {
        PushKind::Approval => {
            let tool = request
                .and_then(|request| request.tool.as_deref())
                .or_else(|| event.and_then(|event| event.tool.as_deref()));
            let headline = match tool {
                Some(tool) => format!(
                    "{}: {}",
                    words.permission,
                    clean_push_text(tool, 100, false)
                ),
                None => words.permission.to_owned(),
            };
            let detail = request
                .and_then(|request| request.body.as_deref())
                .unwrap_or_else(text);
            let mut lines = detail
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty());
            let mut body = clean_push_text(lines.next().unwrap_or_default(), 300, false);
            if lines.next().is_some() && !body.ends_with('…') {
                body.push_str(" …");
            }
            (headline, body)
        }
        PushKind::Question | PushKind::DisplayOnlyWait => {
            let headline = words.question.to_owned();
            let Some(request) = request else {
                return (headline, String::new());
            };
            if request.questions.iter().any(|question| question.is_secret) {
                return (headline, words.secret.to_owned());
            }
            if request.questions.is_empty() {
                let text = request
                    .body
                    .as_deref()
                    .or(request.title.as_deref())
                    .unwrap_or_default();
                return (headline, numbered_question(text, &request.options, words));
            }
            let mut body = Vec::new();
            for (index, question) in request.questions.iter().take(4).enumerate() {
                let text = numbered_question(&question.question, &question.options, words);
                if request.questions.len() > 1 {
                    body.push(format!(
                        "({}/{}) {text}",
                        index + 1,
                        request.questions.len()
                    ));
                } else {
                    body.push(text);
                }
            }
            (headline, body.join("\n"))
        }
        PushKind::Completion => (
            words.completion.to_owned(),
            clean_push_text(text(), limit, true),
        ),
        PushKind::Error => {
            let error = event
                .and_then(|event| event.payload.as_ref())
                .and_then(|payload| {
                    payload["error"]
                        .as_str()
                        .or_else(|| payload["error"]["name"].as_str())
                });
            let headline = match error {
                Some(error) => format!("{} ({})", words.error, clean_push_text(error, 80, true)),
                None => words.error.to_owned(),
            };
            (headline, clean_push_text(text(), limit, true))
        }
    }
}

fn numbered_question(text: &str, options: &[String], words: &PushWords) -> String {
    let mut lines = vec![clean_push_text(text, 300, true)];
    for (index, option) in options
        .iter()
        .filter(|option| !option.trim().is_empty())
        .take(9)
        .enumerate()
    {
        lines.push(format!(
            "{}. {}",
            index + 1,
            clean_push_text(option, 80, true)
        ));
    }
    if options.len() > 9 {
        lines.push(
            words
                .more_options
                .replace("%d", &(options.len() - 9).to_string()),
        );
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use codeisland_domain::{AgentEventType, SessionStore};
    use serde_json::json;

    #[test]
    fn secret_question_does_not_leave_the_device_in_a_push() {
        let mut event = AgentEvent::new("session", AgentKind::Codex, AgentEventType::Question);
        event.waits = true;
        event.title = Some("Secret title".into());
        event.text = Some("Secret prompt".into());
        event.payload = Some(json!({
            "questions": [{"question": "Secret prompt", "isSecret": true, "options": [{"label":"Secret option"}]}]
        }));
        let mut store = SessionStore::default();
        store.apply(&event);
        let (headline, text) = push_content(
            PushKind::Question,
            store.get_pending(&event.id),
            store.get("session"),
            Some(&event),
            &EN,
            200,
        );
        assert_eq!(headline, EN.question);
        assert_eq!(text, EN.secret);
        assert!(!text.contains("Secret option"));
    }
}

// The UI and runtime share the same language setting.
pub(crate) fn push_words(language: &str) -> &'static PushWords {
    match language {
        "de-DE" | "de" => &DE,
        "zh-CN" | "zh" => &ZH,
        "zh-TW" | "zh-Hant" => &ZH_HANT,
        "ja-JP" | "ja" => &JA,
        "ko-KR" | "ko" => &KO,
        "tr-TR" | "tr" => &TR,
        _ => &EN,
    }
}

const EN: PushWords = PushWords {
    permission: "Needs approval",
    question: "Has a question",
    completion: "Finished",
    error: "Stopped on an error",
    reminder: "Still waiting",
    minutes: "%d min",
    secret: "Sensitive prompt — answer it on your PC.",
    more_options: "+%d more",
};

const DE: PushWords = PushWords {
    permission: "Braucht Freigabe",
    question: "Hat eine Frage",
    completion: "Fertig",
    error: "Mit Fehler abgebrochen",
    reminder: "Wartet noch",
    minutes: "%d Min.",
    secret: "Vertrauliche Eingabe – bitte am Mac beantworten.",
    more_options: "+%d weitere",
};

const ZH: PushWords = PushWords {
    permission: "需要审批",
    question: "提问",
    completion: "已完成",
    error: "出错停止",
    reminder: "仍在等待",
    minutes: "%d 分钟",
    secret: "敏感输入，请在 Mac 上作答。",
    more_options: "另有 %d 项",
};

const ZH_HANT: PushWords = PushWords {
    permission: "需要審批",
    question: "提問",
    completion: "已完成",
    error: "出錯停止",
    reminder: "仍在等待",
    minutes: "%d 分鐘",
    secret: "敏感輸入，請在 Mac 上作答。",
    more_options: "另有 %d 項",
};

const JA: PushWords = PushWords {
    permission: "承認が必要",
    question: "質問があります",
    completion: "完了",
    error: "エラーで停止",
    reminder: "まだ待機中",
    minutes: "%d 分",
    secret: "機密性の高い入力です。PC で回答してください。",
    more_options: "他 %d 件",
};

const KO: PushWords = PushWords {
    permission: "승인 필요",
    question: "질문이 있습니다",
    completion: "완료",
    error: "오류로 중단됨",
    reminder: "아직 대기 중",
    minutes: "%d분",
    secret: "민감한 입력입니다. PC에서 답변하세요.",
    more_options: "외 %d개",
};

const TR: PushWords = PushWords {
    permission: "Onay gerekiyor",
    question: "Bir sorusu var",
    completion: "Tamamlandı",
    error: "Hatayla durdu",
    reminder: "Hâlâ bekliyor",
    minutes: "%d dk",
    secret: "Hassas istem — Mac'inizde yanıtlayın.",
    more_options: "+%d daha",
};
