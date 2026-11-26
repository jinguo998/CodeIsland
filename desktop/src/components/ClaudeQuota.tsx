import { useTranslation } from "react-i18next";
import { useLayoutEffect, useRef, type CSSProperties } from "react";
import type { ClaudePlanQuota, ClaudeQuotaLimit, ClaudeUsageTotals, QuotaSnapshot } from "../lib/ipc";
import { useNow } from "../lib/useNow";

const WINDOW_ORDER = ["session", "weekly_all", "weekly_scoped"];
const clamp = (n: number) => Math.max(0, Math.min(n, 1));
const rounded = (n: number) => Math.sign(n) * Math.round(Math.abs(n));
const percent = (n: number) => `${rounded(n)}%`;
const windowSeconds = (limit: ClaudeQuotaLimit) => (limit.kind === "session" ? 5 * 3600 : 7 * 86400);
const remaining = (limit: ClaudeQuotaLimit, now: number) =>
  limit.resets_at ? (Date.parse(limit.resets_at) - now) / 1000 : null;

function elapsedFraction(limit: ClaudeQuotaLimit, now: number) {
  const left = remaining(limit, now);
  return left === null ? null : clamp(1 - left / windowSeconds(limit));
}

const paceDelta = (limit: ClaudeQuotaLimit, now: number) =>
  limit.percent / 100 - (elapsedFraction(limit, now) ?? 0);
const hasSurplus = (limit: ClaudeQuotaLimit, now: number) =>
  limit.kind !== "session" && (remaining(limit, now) ?? 0) > 0
  && (elapsedFraction(limit, now) ?? 0) >= 0.5 && paceDelta(limit, now) <= -0.25;

function pickLimit(limits: ClaudeQuotaLimit[], mode: string, now: number) {
  if (mode === "off") return;
  const session = limits.find(({ kind }) => kind === "session");
  const weeklies = limits.filter(({ kind }) => kind !== "session");
  if (mode === "session") return session;
  if (mode === "weeklyAll") return weeklies.find(({ kind }) => kind === "weekly_all");
  if (mode === "weeklyScoped") return weeklies.find(({ kind }) => kind === "weekly_scoped");
  if (session && session.percent > 50 && paceDelta(session, now) > 0.1) return session;
  const pressing = weeklies.filter((limit) => limit.percent >= 80 || paceDelta(limit, now) > 0);
  const surplus = weeklies.filter((limit) => hasSurplus(limit, now));
  if (pressing.length) return pressing.sort((a, b) => paceDelta(b, now) - paceDelta(a, now))[0];
  if (surplus.length) return surplus.sort((a, b) => paceDelta(a, now) - paceDelta(b, now))[0];
  return weeklies.sort((a, b) => b.percent - a.percent)[0] ?? session;
}

function duration(seconds: number) {
  const minutes = Math.ceil(Math.max(1, Math.floor(Math.abs(seconds))) / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h${minutes % 60 ? `${String(minutes % 60).padStart(2, "0")}m` : ""}`;
  return `${Math.floor(hours / 24)}d${hours % 24 ? ` ${hours % 24}h` : ""}`;
}

function quotaTone(limit: ClaudeQuotaLimit, now: number, footer = false) {
  const severity = limit.severity.toLowerCase();
  if (limit.percent >= 100) return "critical";
  if (["warning", "warn", "elevated"].includes(severity)) return "warning";
  if (severity !== "normal" && severity !== "") return "critical";
  if (footer && limit.percent >= 80) return "warning";
  return !footer && hasSurplus(limit, now) ? "surplus" : "normal";
}

function pace(limit: ClaudeQuotaLimit, now: number) {
  const elapsed = elapsedFraction(limit, now);
  if (elapsed === null || elapsed < 0.1 || (remaining(limit, now) ?? 0) <= 0) return null;
  const points = limit.percent - elapsed * 100;
  const projected = limit.percent / elapsed;
  const tone = Math.abs(rounded(points)) <= 5 ? "neutral" : points > 0 ? "ahead" : "behind";
  const exhaustsIn = limit.percent < 100 && projected > 100
    ? (100 - limit.percent) * elapsed * windowSeconds(limit) / limit.percent
    : null;
  return { points, tone, projected, exhaustsIn, seconds: points / 100 * windowSeconds(limit) };
}

function ordered(limits: ClaudeQuotaLimit[]) {
  return [...limits].sort((a, b) => WINDOW_ORDER.indexOf(a.kind) - WINDOW_ORDER.indexOf(b.kind));
}

function shorten(text: string, length: number) {
  const characters = Array.from(text);
  return characters.length > length ? characters.slice(0, length - 1).join("").trimEnd() + "…" : text;
}

function useQuotaText() {
  const { t } = useTranslation();
  const label = (limit: ClaudeQuotaLimit) => limit.kind === "session" ? "5h" : limit.scope_label ?? t("quota.week");
  const tooltip = (limits: ClaudeQuotaLimit[], stale: boolean, now: number) => {
    const lines = ordered(limits).map((limit) => {
      const parts = [`${label(limit)} ${percent(limit.percent)}`];
      const left = remaining(limit, now);
      if (left !== null && left >= 1) parts.push(t("quota.resetsIn", { duration: duration(left) }));
      const p = pace(limit, now);
      if (p) {
        parts.push(t(`quota.${p.tone}`, {
          points: Math.abs(rounded(p.points)),
          duration: duration(p.seconds),
        }));
        if (p.exhaustsIn !== null) parts.push(t("quota.exhaustsIn", { duration: duration(p.exhaustsIn) }));
        else parts.push(t("quota.projected", { percent: percent(p.projected) }));
      }
      return parts.join(" · ");
    });
    if (stale) lines.push(t("quota.stale"));
    return lines.join("\n");
  };
  return { t, label, tooltip };
}

function formatTokens(count: number) {
  if (count < 1_000) return String(count);
  const [value, unit] = count < 999_950 ? [count / 1_000, "K"]
    : count < 999_950_000 ? [count / 1_000_000, "M"] : [count / 1_000_000_000, "B"];
  return `${Number(value.toFixed(1))}${unit}`;
}

function useUsageText(usage: QuotaSnapshot | null) {
  const { t } = useTranslation();
  if (!usage?.last_5h || !usage.today || !usage.last_5h.message_count && !usage.today.message_count) {
    return null;
  }
  const span = (label: string, totals: ClaudeUsageTotals) => t("quota.usageSpan", {
    label,
    totals: t("quota.usageInOut", {
      input: formatTokens(totals.input_tokens + totals.cache_creation_tokens),
      output: formatTokens(totals.output_tokens),
    }),
  });
  const breakdown = (totals: ClaudeUsageTotals) => t("quota.usageBreakdown", {
    input: formatTokens(totals.input_tokens),
    writes: formatTokens(totals.cache_creation_tokens),
    reads: formatTokens(totals.cache_read_tokens),
  });
  const lastFive = span(t("quota.usageLastFive"), usage.last_5h);
  const today = span(t("quota.usageToday"), usage.today);
  const details = [
    t("quota.usageTitle"), lastFive, breakdown(usage.last_5h), today, breakdown(usage.today),
  ].join("\n");
  return { lastFive, today, details };
}

function UsageSparkline({ buckets }: { buckets: number[] }) {
  const peak = Math.max(...buckets, 1);
  return (
    <span className="island-usage-sparkline" aria-hidden="true">
      {buckets.map((value, index) => (
        <span key={index} data-zero={value === 0} style={{ height: Math.max(1.5, value / peak * 10) }} />
      ))}
    </span>
  );
}

export function ClaudeUsageFooter({ usage }: { usage: QuotaSnapshot }) {
  const text = useUsageText(usage);
  if (!text) return null;
  return (
    <footer className="island-footer island-usage-footer" title={text.details} aria-label={text.details}>
      <span className="island-usage-spans">
        <span><strong>Claude</strong> · {text.lastFive}</span>
        <span>{text.today}</span>
      </span>
      <UsageSparkline buckets={usage.hourly_output_tokens} />
    </footer>
  );
}

export function ClaudeQuotaChip({
  state, mode, showsPace, onWidth,
}: { state: ClaudePlanQuota; mode: string; showsPace: boolean; onWidth?: (width: number) => void }) {
  const now = useNow(60_000);
  const { label, tooltip } = useQuotaText();
  const limits = state.snapshot?.limits ?? [];
  const limit = pickLimit(limits, mode, now);
  const element = useRef<HTMLSpanElement>(null);
  const shown = Boolean(limit);
  useLayoutEffect(() => {
    const chip = element.current;
    if (!chip || !onWidth) return;
    const measure = () => onWidth(chip.offsetWidth);
    measure();
    if (typeof ResizeObserver === "undefined") return () => onWidth(0);
    const observer = new ResizeObserver(measure);
    observer.observe(chip);
    return () => {
      observer.disconnect();
      onWidth(0);
    };
  }, [shown, onWidth]);
  if (!limit) return null;
  const p = showsPace ? pace(limit, now) : null;
  const stale = state.error !== null;
  return (
    <span
      ref={element}
      className="island-quota-chip"
      data-tone={quotaTone(limit, now)}
      data-stale={stale}
      title={tooltip(limits, stale, now)}
    >
      <span className="island-quota-chip-label">{shorten(label(limit), 8)}</span>
      <span className="island-quota-ring" style={{ "--quota-used": `${clamp(limit.percent / 100) * 100}%` } as CSSProperties} />
      <span className="island-quota-chip-number" data-pace={p?.tone}>
        {percent(limit.percent)}
      </span>
    </span>
  );
}

export function ClaudeQuotaFooter({ state, usage }: { state: ClaudePlanQuota; usage: QuotaSnapshot | null }) {
  const now = useNow(60_000);
  const { t, label, tooltip } = useQuotaText();
  const localUsage = useUsageText(usage);
  const limits = state.snapshot?.limits;
  if (!limits) {
    const needsLogin = state.error === "no_credential" || state.error === "unauthorized";
    return state.error ? (
      <div className="island-footer island-quota-message">
        {t(needsLogin ? "quota.loginNeeded" : "quota.unreachable")}
      </div>
    ) : null;
  }
  const stale = state.error !== null;
  const details = [tooltip(limits, stale, now), localUsage?.details].filter(Boolean).join("\n\n");
  return (
    <footer className="island-footer island-quota-footer" title={details} aria-label={details}>
      {ordered(limits).map((limit) => {
        const elapsed = elapsedFraction(limit, now);
        const left = remaining(limit, now);
        return (
          <span key={limit.kind} className="island-quota-window" data-tone={quotaTone(limit, now, true)}>
            <span className="island-quota-window-label">{shorten(label(limit), 10)}</span>
            <span className="island-quota-meter">
              <span style={{ width: `${clamp(limit.percent / 100) * 100}%` }} />
              {elapsed !== null && <i style={{ left: `${elapsed * 100}%` }} />}
            </span>
            <strong>{percent(limit.percent)}</strong>
            {left !== null && left >= 1 && <span className="island-quota-countdown">{duration(left)}</span>}
          </span>
        );
      })}
      {stale && <span className="island-quota-stale" aria-hidden="true">⚠</span>}
      {localUsage && usage && <UsageSparkline buckets={usage.hourly_output_tokens} />}
    </footer>
  );
}
