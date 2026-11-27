import Foundation

/// Notification strings used by the app, its settings and its push channels.
public enum PushLocalization {
    public static func strings(_ lookup: (String) -> String) -> PushStrings {
        PushStrings(
            permission: lookup("push_msg_permission"),
            question: lookup("push_msg_question"),
            completion: lookup("push_msg_completion"),
            error: lookup("push_msg_error"),
            reminder: lookup("push_msg_reminder"),
            waitingMinutes: lookup("push_msg_waiting_minutes"),
            secretQuestion: lookup("push_msg_secret_question"),
            moreOptions: lookup("push_msg_more_options"),
            testHeadline: lookup("push_msg_test_headline"),
            testBody: lookup("push_msg_test_body"),
            answerIn: lookup("push_msg_answer_in"),
            answerOnMac: lookup("push_msg_answer_on_mac")
        )
    }

    public static func table(for language: String) -> [String: String] {
        switch language {
        case "de": return de
        case "zh": return zh
        case "zh-Hant": return zhHant
        case "ja": return ja
        case "ko": return ko
        case "tr": return tr
        default: return en
        }
    }

    private static let en: [String: String] = [
        "push_msg_permission": "Needs approval",
        "push_msg_question": "Has a question",
        "push_msg_completion": "Finished",
        "push_msg_error": "Stopped on an error",
        "push_msg_reminder": "Still waiting",
        "push_msg_waiting_minutes": "%d min",
        "push_msg_secret_question": "Sensitive prompt — answer it on your Mac.",
        "push_msg_more_options": "+%d more",
        "push_msg_test_headline": "Test notification",
        "push_msg_test_body": "If you can read this, CodeIsland can reach you here.",
        "push_msg_answer_in": "Respond in %@.",
        "push_msg_answer_on_mac": "Respond on your Mac.",
    ]

    private static let de: [String: String] = [
        "push_msg_permission": "Braucht Freigabe",
        "push_msg_question": "Hat eine Frage",
        "push_msg_completion": "Fertig",
        "push_msg_error": "Mit Fehler abgebrochen",
        "push_msg_reminder": "Wartet noch",
        "push_msg_waiting_minutes": "%d Min.",
        "push_msg_secret_question": "Vertrauliche Eingabe – bitte am Mac beantworten.",
        "push_msg_more_options": "+%d weitere",
        "push_msg_test_headline": "Testbenachrichtigung",
        "push_msg_test_body": "Wenn du das liest, kann CodeIsland dich hier erreichen.",
        "push_msg_answer_in": "Bitte in %@ erledigen.",
        "push_msg_answer_on_mac": "Bitte am Mac erledigen.",
    ]

    private static let zh: [String: String] = [
        "push_msg_permission": "需要审批",
        "push_msg_question": "提问",
        "push_msg_completion": "已完成",
        "push_msg_error": "出错停止",
        "push_msg_reminder": "仍在等待",
        "push_msg_waiting_minutes": "%d 分钟",
        "push_msg_secret_question": "敏感输入，请在 Mac 上作答。",
        "push_msg_more_options": "另有 %d 项",
        "push_msg_test_headline": "测试通知",
        "push_msg_test_body": "看到这条消息，说明 CodeIsland 能通过这里联系到你。",
        "push_msg_answer_in": "请在 %@ 中处理。",
        "push_msg_answer_on_mac": "请在 Mac 上处理。",
    ]

    private static let zhHant: [String: String] = [
        "push_msg_permission": "需要審批",
        "push_msg_question": "提問",
        "push_msg_completion": "已完成",
        "push_msg_error": "出錯停止",
        "push_msg_reminder": "仍在等待",
        "push_msg_waiting_minutes": "%d 分鐘",
        "push_msg_secret_question": "敏感輸入，請在 Mac 上作答。",
        "push_msg_more_options": "另有 %d 項",
        "push_msg_test_headline": "測試通知",
        "push_msg_test_body": "看到這則訊息，代表 CodeIsland 能透過這裡聯絡到你。",
        "push_msg_answer_in": "請在 %@ 中處理。",
        "push_msg_answer_on_mac": "請在 Mac 上處理。",
    ]

    private static let ja: [String: String] = [
        "push_msg_permission": "承認が必要",
        "push_msg_question": "質問があります",
        "push_msg_completion": "完了",
        "push_msg_error": "エラーで停止",
        "push_msg_reminder": "まだ待機中",
        "push_msg_waiting_minutes": "%d 分",
        "push_msg_secret_question": "機密性の高い入力です。Mac で回答してください。",
        "push_msg_more_options": "他 %d 件",
        "push_msg_test_headline": "テスト通知",
        "push_msg_test_body": "これが届いていれば、CodeIsland はここに通知できます。",
        "push_msg_answer_in": "%@ で対応してください。",
        "push_msg_answer_on_mac": "Mac で対応してください。",
    ]

    private static let ko: [String: String] = [
        "push_msg_permission": "승인 필요",
        "push_msg_question": "질문이 있습니다",
        "push_msg_completion": "완료",
        "push_msg_error": "오류로 중단됨",
        "push_msg_reminder": "아직 대기 중",
        "push_msg_waiting_minutes": "%d분",
        "push_msg_secret_question": "민감한 입력입니다. Mac에서 답변하세요.",
        "push_msg_more_options": "외 %d개",
        "push_msg_test_headline": "테스트 알림",
        "push_msg_test_body": "이 메시지가 보이면 CodeIsland가 여기로 알림을 보낼 수 있습니다.",
        "push_msg_answer_in": "%@에서 처리해 주세요.",
        "push_msg_answer_on_mac": "Mac에서 처리해 주세요.",
    ]

    private static let tr: [String: String] = [
        "push_msg_permission": "Onay gerekiyor",
        "push_msg_question": "Bir sorusu var",
        "push_msg_completion": "Tamamlandı",
        "push_msg_error": "Hatayla durdu",
        "push_msg_reminder": "Hâlâ bekliyor",
        "push_msg_waiting_minutes": "%d dk",
        "push_msg_secret_question": "Hassas istem — Mac'inizde yanıtlayın.",
        "push_msg_more_options": "+%d daha",
        "push_msg_test_headline": "Test bildirimi",
        "push_msg_test_body": "Bunu okuyabiliyorsanız CodeIsland size buradan ulaşabilir.",
        "push_msg_answer_in": "Lütfen %@ üzerinden yanıt verin.",
        "push_msg_answer_on_mac": "Lütfen Mac'inizden yanıt verin.",
    ]
}
