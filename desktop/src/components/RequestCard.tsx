import { useState } from "react";
import { useTranslation } from "react-i18next";
import { agentLabelKey } from "../lib/agents";
import { useIslandStore } from "../lib/store";
import type { PendingRequest } from "../lib/types";
import { ApprovalCard } from "./ApprovalCard";
import { CliIcon } from "./CliIcon";
import { PixelButton } from "./PixelButton";

export interface RequestCardProps {
  request: PendingRequest;
  standalone?: boolean;
  position?: number;
  total?: number;
}

interface QuestionDraft {
  answer: string;
  selected: number[];
  other: boolean;
  text: string;
}

export function RequestCard(props: RequestCardProps) {
  return props.request.kind === "permission"
    ? <ApprovalCard {...props} />
    : <QuestionCard {...props} />;
}

function QuestionCard({
  request,
  standalone = true,
  position = 1,
  total = 1,
}: RequestCardProps) {
  const { t } = useTranslation();
  const resolveRequest = useIslandStore((state) => state.resolveRequest);
  const hideRequest = useIslandStore((state) => state.hideRequest);
  const session = useIslandStore((state) =>
    state.ui?.store.sessions.find((entry) => entry.session_id === request.session_id),
  );
  const [answer, setAnswer] = useState("");
  const [questionIndex, setQuestionIndex] = useState(0);
  const [collected, setCollected] = useState<QuestionDraft[]>([]);
  const [selected, setSelected] = useState<number[]>([]);
  const [other, setOther] = useState(false);

  const questions = request.questions ?? [];
  const current = questions[questionIndex];
  const title = current?.question ?? request.title ?? t("request.questionTitle");
  const options = current?.options ?? request.options;
  const multiSelect = current?.multi_select === true;
  const project = session?.project ?? session?.cwd?.split(/[\\/]/).filter(Boolean).pop();
  const queueProgress = total > 1 ? `${position}/${total}` : null;

  const resetDraft = () => {
    setAnswer("");
    setSelected([]);
    setOther(false);
  };

  const submit = (raw: string, indices = selected) => {
    const value = raw.trim();
    if (!value) return;
    if (questions.length === 0) {
      void resolveRequest(request.event_id, "answer", value);
      return;
    }

    const next = [...collected];
    next[questionIndex] = { answer: value, selected: indices, other, text: answer };
    resetDraft();
    if (questionIndex + 1 < questions.length) {
      setCollected(next);
      setQuestionIndex(questionIndex + 1);
      return;
    }
    const entries = questions.map((item, index) => [
      item.answer_key || item.header?.trim() || item.question || "answer",
      next[index]?.answer ?? "",
    ]);
    void resolveRequest(request.event_id, "answer", JSON.stringify(Object.fromEntries(entries)));
  };

  const submitAnswer = () => {
    if (!multiSelect) {
      submit(answer);
      return;
    }
    const choices = options.filter((_, index) => selected.includes(index));
    if (other && answer.trim()) choices.push(answer.trim());
    submit(choices.join(", "));
  };

  const goBack = () => {
    const previous = questionIndex - 1;
    const draft = collected[previous];
    setAnswer(draft?.text ?? "");
    setSelected(draft?.selected ?? []);
    setOther(draft?.other ?? false);
    setQuestionIndex(previous);
  };

  const hide = (
    <PixelButton
      variant="ghost"
      aria-label={t("request.hide")}
      onClick={() => void hideRequest(request.event_id)}
    >
      {t("common.hide")}
    </PixelButton>
  );

  const field = (
    <div className="request-answer-row">
      <span className="answer-prompt" aria-hidden="true">&gt;</span>
      <input
        type={current?.is_secret ? "password" : "text"}
        autoComplete={current?.is_secret ? "off" : undefined}
        autoFocus={other}
        value={answer}
        onChange={(event) => setAnswer(event.currentTarget.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") submitAnswer();
        }}
        placeholder={t("request.answerPlaceholder")}
        aria-label={t("request.answerPlaceholder")}
      />
    </div>
  );

  const canSubmit = multiSelect ? selected.length > 0 || (other && Boolean(answer.trim())) : Boolean(answer.trim());
  const content = (
    <>
      <div className="request-head">
        <span className="request-question-mark" aria-hidden="true">?</span>
        {current?.header && <span className="request-kind-tag">{current.header}</span>}
        <span className="request-title" title={title}>{title}</span>
        {questions.length > 1 && (
          <span className="request-progress">{questionIndex + 1}/{questions.length}</span>
        )}
      </div>
      {options.length > 0 ? (
        <div className="request-option-list" aria-label={t("request.options")}>
          {options.map((option, index) => (
            <button
              type="button"
              className="request-option"
              key={`${index}-${option}`}
              aria-pressed={selected.includes(index)}
              onClick={() => {
                if (multiSelect) {
                  setSelected((items) => items.includes(index)
                    ? items.filter((item) => item !== index)
                    : [...items, index]);
                } else {
                  submit(option, [index]);
                }
              }}
            >
              <span aria-hidden="true">{index + 1}</span>
              <span className="request-option-content">
                {option}
                {current?.option_descriptions?.[index] && (
                  <small>{current.option_descriptions[index]}</small>
                )}
              </span>
            </button>
          ))}
          <button
            type="button"
            className="request-option"
            aria-expanded={other}
            onClick={() => {
              setOther(!other);
              if (other) setAnswer("");
            }}
          >
            <span aria-hidden="true">+</span>{t("request.other")}
          </button>
          {other && field}
        </div>
      ) : field}
      <div className="request-actions">
        {questionIndex > 0 && <PixelButton variant="ghost" onClick={goBack}>{t("request.back")}</PixelButton>}
        <span className="footer-spacer" />
        {hide}
        <PixelButton variant="ghost" onClick={() => void resolveRequest(request.event_id, "skip")}>
          {t("request.skip")}
        </PixelButton>
        {(multiSelect || options.length === 0 || other) && (
          <PixelButton variant="primary" disabled={!canSubmit} onClick={submitAnswer}>
            {t(multiSelect ? "request.confirm" : "request.submit")}
          </PixelButton>
        )}
      </div>
    </>
  );

  return (
    <div className="request-card" data-request-kind={request.kind} data-standalone={standalone}>
      {standalone && (project || queueProgress) && (
        <div className="request-context">
          <CliIcon source={request.agent} />
          <span>{t("request.fromSession", { agent: t(agentLabelKey(request.agent)), project: project ?? "" })}</span>
          {queueProgress && <span className="request-progress">{queueProgress}</span>}
        </div>
      )}
      {content}
    </div>
  );
}
