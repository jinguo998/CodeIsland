import { useId } from "react";
import { useTranslation } from "react-i18next";
import { MaxVisibleSessionsField } from "../MaxVisibleSessionsField";
import { HookBehaviorSettings } from "./HookBehaviorSettings";
import { PushSettings } from "./PushSettings";
import {
  HOVER_EXPAND_DELAY_MAX,
  HOVER_EXPAND_DELAY_MIN,
  HOVER_EXPAND_DELAY_STEP,
  ROTATION_INTERVAL_OPTIONS,
} from "../../lib/defaults";
import {
  NumberField,
  SelectField,
  SettingRow,
  SettingsSection,
  SliderField,
  TimeField,
  Toggle,
  type SettingsTabProps,
} from "../controls";

const TOOL_HISTORY_OPTIONS = [10, 20, 50, 100];

export function BehaviorTab({ settings, onChange }: SettingsTabProps) {
  const { t } = useTranslation();
  const completionId = useId();
  const reminderId = useId();
  const cleanupId = useId();
  const rotationIntervalId = useId();
  const quietId = useId();
  const quietStartId = useId();
  const quietEndId = useId();
  const muteId = useId();
  const smartId = useId();
  const autoExpandPermissionId = useId();
  const autoExpandQuestionId = useId();
  const hideNoSessionId = useId();
  const hoverDelayId = useId();
  const horizontalDragId = useId();
  const toolHistoryId = useId();
  const codexSubsessionsId = useId();
  const hideFullscreenId = useId();
  const collapseId = useId();
  const collapseAfterJumpId = useId();
  const toolHistoryOptions = TOOL_HISTORY_OPTIONS.includes(settings.max_history)
    ? TOOL_HISTORY_OPTIONS
    : [...TOOL_HISTORY_OPTIONS, settings.max_history];

  return (
    <div>
      <SettingsSection>
        <SettingRow
          htmlFor={completionId}
          label={t("settings.behavior.completionStyle")}
        >
          <SelectField
            id={completionId}
            value={settings.completion_style}
            onChange={(value) =>
              onChange({
                completion_style: value as typeof settings.completion_style,
              })
            }
            options={[
              {
                value: "expand",
                label: t("settings.behavior.completionExpand"),
              },
              {
                value: "glance",
                label: t("settings.behavior.completionGlance"),
              },
              { value: "off", label: t("settings.behavior.completionOff") },
            ]}
          />
        </SettingRow>

        <SettingRow
          htmlFor={reminderId}
          label={t("settings.behavior.followUpReminders")}
          description={t("settings.behavior.followUpRemindersHint")}
        >
          <SelectField
            id={reminderId}
            value={String(settings.follow_up_reminder_minutes)}
            onChange={(value) => onChange({ follow_up_reminder_minutes: Number(value) })}
            options={[0, 1, 2, 3, 5].map((minutes) => ({
              value: String(minutes),
              label: minutes === 0
                ? t("settings.behavior.followUpOff")
                : t("settings.behavior.followUpAfterMinutes", { minutes }),
            }))}
          />
        </SettingRow>

        <SettingRow
          htmlFor={autoExpandPermissionId}
          label={t("settings.behavior.autoExpandOnPermission")}
          description={t("settings.behavior.autoExpandOnPermissionHint")}
        >
          <Toggle
            id={autoExpandPermissionId}
            label={t("settings.behavior.autoExpandOnPermission")}
            checked={settings.auto_expand_on_permission}
            onChange={(auto_expand_on_permission) =>
              onChange({ auto_expand_on_permission })
            }
          />
        </SettingRow>

        <SettingRow
          htmlFor={autoExpandQuestionId}
          label={t("settings.behavior.autoExpandOnQuestion")}
          description={t("settings.behavior.autoExpandOnQuestionHint")}
        >
          <Toggle
            id={autoExpandQuestionId}
            label={t("settings.behavior.autoExpandOnQuestion")}
            checked={settings.auto_expand_on_question}
            onChange={(auto_expand_on_question) =>
              onChange({ auto_expand_on_question })
            }
          />
        </SettingRow>

        <SettingRow
          htmlFor={hideNoSessionId}
          label={t("settings.behavior.hideWhenNoSession")}
          description={t("settings.behavior.hideWhenNoSessionHint")}
        >
          <Toggle
            id={hideNoSessionId}
            label={t("settings.behavior.hideWhenNoSession")}
            checked={settings.hide_when_no_session}
            onChange={(hide_when_no_session) => onChange({ hide_when_no_session })}
          />
        </SettingRow>

        <SettingRow
          htmlFor={smartId}
          label={t("settings.behavior.smartSuppress")}
          description={t("settings.behavior.smartSuppressHint")}
        >
          <Toggle
            id={smartId}
            label={t("settings.behavior.smartSuppress")}
            checked={settings.smart_suppress}
            onChange={(smart_suppress) => onChange({ smart_suppress })}
          />
        </SettingRow>

        <SettingRow
          htmlFor={hoverDelayId}
          label={t("settings.behavior.hoverExpandDelay")}
          description={t("settings.behavior.hoverExpandDelayHint")}
        >
          <div className="flex items-center gap-3">
            <SliderField
              id={hoverDelayId}
              value={settings.hover_expand_delay_ms}
              min={HOVER_EXPAND_DELAY_MIN}
              max={HOVER_EXPAND_DELAY_MAX}
              step={HOVER_EXPAND_DELAY_STEP}
              onChange={(hover_expand_delay_ms) =>
                onChange({ hover_expand_delay_ms })
              }
            />
            <span className="w-16 text-right text-[12px] tabular-nums text-white/55">
              {t("settings.behavior.hoverExpandDelayValue", {
                ms: settings.hover_expand_delay_ms,
              })}
            </span>
          </div>
        </SettingRow>

        <SettingRow
          htmlFor={horizontalDragId}
          label={t("settings.behavior.allowHorizontalDrag")}
          description={t("settings.behavior.allowHorizontalDragHint")}
        >
          <Toggle
            id={horizontalDragId}
            label={t("settings.behavior.allowHorizontalDrag")}
            checked={settings.allow_horizontal_drag}
            onChange={(allow_horizontal_drag) =>
              onChange({ allow_horizontal_drag })
            }
          />
        </SettingRow>

        <SettingRow
          htmlFor={collapseId}
          label={t("settings.behavior.collapseOnMouseLeave")}
          description={t("settings.behavior.collapseOnMouseLeaveHint")}
        >
          <Toggle
            id={collapseId}
            label={t("settings.behavior.collapseOnMouseLeave")}
            checked={settings.collapse_on_mouse_leave}
            onChange={(collapse_on_mouse_leave) =>
              onChange({ collapse_on_mouse_leave })
            }
          />
        </SettingRow>

        <SettingRow
          htmlFor={collapseAfterJumpId}
          label={t("settings.behavior.autoCollapseAfterSessionJump")}
          description={t("settings.behavior.autoCollapseAfterSessionJumpHint")}
        >
          <Toggle
            id={collapseAfterJumpId}
            label={t("settings.behavior.autoCollapseAfterSessionJump")}
            checked={settings.auto_collapse_after_session_jump}
            onChange={(auto_collapse_after_session_jump) => onChange({ auto_collapse_after_session_jump })}
          />
        </SettingRow>

        <SettingRow
          htmlFor={hideFullscreenId}
          label={t("settings.behavior.hideFullscreen")}
        >
          <Toggle
            id={hideFullscreenId}
            label={t("settings.behavior.hideFullscreen")}
            checked={settings.hide_in_fullscreen}
            onChange={(hide_in_fullscreen) => onChange({ hide_in_fullscreen })}
          />
        </SettingRow>

        <SettingRow
          htmlFor={muteId}
          label={t("settings.behavior.muteWhenAway")}
          description={t("settings.behavior.muteWhenAwayHint")}
        >
          <Toggle
            id={muteId}
            label={t("settings.behavior.muteWhenAway")}
            checked={settings.mute_when_away}
            onChange={(mute_when_away) => onChange({ mute_when_away })}
          />
        </SettingRow>

        <SettingRow
          htmlFor={quietId}
          label={t("settings.behavior.quietHours")}
        >
          <Toggle
            id={quietId}
            label={t("settings.behavior.quietHours")}
            checked={settings.quiet_hours_enabled}
            onChange={(quiet_hours_enabled) => onChange({ quiet_hours_enabled })}
          />
        </SettingRow>

        <SettingRow
          htmlFor={quietStartId}
          label={t("settings.behavior.quietStart")}
        >
          <TimeField
            id={quietStartId}
            value={settings.quiet_hours_start}
            onChange={(quiet_hours_start) => onChange({ quiet_hours_start })}
          />
        </SettingRow>

        <SettingRow
          htmlFor={quietEndId}
          label={t("settings.behavior.quietEnd")}
        >
          <TimeField
            id={quietEndId}
            value={settings.quiet_hours_end}
            onChange={(quiet_hours_end) => onChange({ quiet_hours_end })}
          />
        </SettingRow>

        <SettingRow
          htmlFor={cleanupId}
          label={t("settings.behavior.cleanup")}
          description={t("settings.behavior.cleanupHint")}
        >
          <NumberField
            id={cleanupId}
            value={settings.session_cleanup_minutes}
            min={0}
            max={1440}
            onChange={(session_cleanup_minutes) =>
              onChange({ session_cleanup_minutes })
            }
          />
        </SettingRow>

        <SettingRow
          htmlFor={rotationIntervalId}
          label={t("settings.behavior.rotationInterval")}
          description={t("settings.behavior.rotationIntervalHint")}
        >
          <SelectField
            id={rotationIntervalId}
            value={String(settings.rotation_interval)}
            onChange={(value) => onChange({ rotation_interval: Number(value) })}
            options={ROTATION_INTERVAL_OPTIONS.map((seconds) => ({
              value: String(seconds),
              label: t(`settings.behavior.rotationSeconds.${seconds}`),
            }))}
          />
        </SettingRow>

        <MaxVisibleSessionsField settings={settings} onChange={onChange} />

        <SettingRow
          htmlFor={toolHistoryId}
          label={t("settings.behavior.toolHistoryLimit")}
          description={t("settings.behavior.toolHistoryLimitHint")}
        >
          <SelectField
            id={toolHistoryId}
            value={String(settings.max_history)}
            onChange={(value) => onChange({ max_history: Number(value) })}
            options={toolHistoryOptions.map((limit) => ({ value: String(limit), label: String(limit) }))}
          />
        </SettingRow>

        <SettingRow
          htmlFor={codexSubsessionsId}
          label={t("settings.behavior.codexSubsessions")}
          description={t("settings.behavior.codexSubsessionsHint")}
        >
          <SelectField
            id={codexSubsessionsId}
            value={settings.codex_subsession_mode}
            onChange={(codex_subsession_mode) => onChange({ codex_subsession_mode })}
            options={["separate", "merge", "hide"].map((value) => ({
              value, label: t(`settings.behavior.codexSubsessionModes.${value}`),
            }))}
          />
        </SettingRow>
      </SettingsSection>
      <HookBehaviorSettings settings={settings} onChange={onChange} />
      <PushSettings settings={settings} onChange={onChange} />
    </div>
  );
}
