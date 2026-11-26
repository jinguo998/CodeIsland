import { useId } from "react";
import { useTranslation } from "react-i18next";
import { MaxVisibleSessionsField } from "../MaxVisibleSessionsField";
import {
  AI_MESSAGE_LINES_MAX,
  AI_MESSAGE_LINES_MIN,
  COLLAPSED_WIDTH_SCALE_MAX,
  COLLAPSED_WIDTH_SCALE_MIN,
  CUSTOM_NOTCH_HEIGHT_MAX,
  CUSTOM_NOTCH_HEIGHT_MIN,
  CONTENT_FONT_SIZE_MAX,
  CONTENT_FONT_SIZE_MIN,
  NOTCH_ANIMATION_SPEED_MAX,
  NOTCH_ANIMATION_SPEED_MIN,
  NOTCH_ANIMATION_SPEED_STEP,
} from "../../lib/defaults";
import {
  SelectField,
  SettingRow,
  SettingsSection,
  SliderField,
  Toggle,
  type SettingsTabProps,
} from "../controls";

export function AppearanceTab({ settings, onChange }: SettingsTabProps) {
  const { t } = useTranslation();
  const animationSpeedId = useId();
  const animationSpeed = settings.notch_animation_speed;
  const animationSpeedText = `${animationSpeed.toFixed(1)}×`;
  const collapsedWidthId = useId();
  const notchHeightModeId = useId();
  const customNotchHeightId = useId();
  const collapsedWidthText = `${settings.collapsed_width_scale}%`;
  const customNotchHeightText = t("settings.appearance.contentFontSizeValue", {
    size: Math.round(settings.custom_notch_height),
  });
  const densityId = useId();
  const groupId = useId();
  const fontSizeId = useId();
  const messageLinesId = useId();
  const agentDetailsId = useId();
  const toolStatusId = useId();
  const projectNameId = useId();
  const gitBranchId = useId();
  const taskProgressId = useId();
  const sessionRecapId = useId();
  const modelLabelId = useId();
  const usageStatsId = useId();
  const claudeQuotaId = useId();
  const claudeQuotaChipId = useId();

  return (
    <div>
      <SettingsSection title={t("settings.appearance.panel")}>
        <SettingRow
          htmlFor={animationSpeedId}
          label={t("settings.appearance.notchAnimationSpeed")}
          description={t("settings.appearance.notchAnimationSpeedHint")}
        >
          <SliderField
            id={animationSpeedId}
            value={animationSpeed}
            valueText={animationSpeedText}
            min={NOTCH_ANIMATION_SPEED_MIN}
            max={NOTCH_ANIMATION_SPEED_MAX}
            step={NOTCH_ANIMATION_SPEED_STEP}
            onChange={(notch_animation_speed) => onChange({ notch_animation_speed })}
          />
          <span className="w-10 text-right text-[12px] tabular-nums text-white/55">
            {animationSpeedText}
          </span>
        </SettingRow>

        <MaxVisibleSessionsField settings={settings} onChange={onChange} />

        <SettingRow htmlFor={densityId} label={t("settings.appearance.density")}>
          <SelectField
            id={densityId}
            value={settings.density}
            onChange={(density) => onChange({ density })}
            options={[
              {
                value: "comfortable",
                label: t("settings.appearance.densityComfortable"),
              },
              {
                value: "compact",
                label: t("settings.appearance.densityCompact"),
              },
            ]}
          />
        </SettingRow>

        <SettingRow htmlFor={groupId} label={t("settings.appearance.groupBy")}>
          <SelectField
            id={groupId}
            value={settings.group_by}
            onChange={(group_by) => onChange({ group_by })}
            options={[
              { value: "all", label: t("settings.appearance.groupAll") },
              {
                value: "status",
                label: t("settings.appearance.groupStatus"),
              },
              { value: "cli", label: t("settings.appearance.groupCli") },
              { value: "none", label: t("settings.appearance.groupNone") },
            ]}
          />
        </SettingRow>

        <SettingRow
          htmlFor={collapsedWidthId}
          label={t("settings.appearance.collapsedWidthScale")}
          description={t("settings.appearance.collapsedWidthScaleHint")}
        >
          <SliderField
            id={collapsedWidthId}
            value={settings.collapsed_width_scale}
            valueText={collapsedWidthText}
            min={COLLAPSED_WIDTH_SCALE_MIN}
            max={COLLAPSED_WIDTH_SCALE_MAX}
            step={1}
            onChange={(collapsed_width_scale) => onChange({ collapsed_width_scale })}
          />
          <span className="w-10 text-right text-[12px] tabular-nums text-white/55">
            {collapsedWidthText}
          </span>
        </SettingRow>

        <SettingRow
          htmlFor={notchHeightModeId}
          label={t("settings.appearance.notchHeightMode")}
          description={t("settings.appearance.notchHeightModeHint")}
        >
          <SelectField
            id={notchHeightModeId}
            value={settings.notch_height_mode}
            onChange={(notch_height_mode) => onChange({ notch_height_mode })}
            options={[
              {
                value: "matchNotch",
                label: t("settings.appearance.notchHeightMatchNotch"),
              },
              {
                value: "matchMenuBar",
                label: t("settings.appearance.notchHeightMatchMenuBar"),
              },
              {
                value: "custom",
                label: t("settings.appearance.customNotchHeight"),
              },
            ]}
          />
        </SettingRow>

        {settings.notch_height_mode === "custom" && (
          <SettingRow
            htmlFor={customNotchHeightId}
            label={t("settings.appearance.customNotchHeight")}
          >
            <SliderField
              id={customNotchHeightId}
              value={settings.custom_notch_height}
              valueText={customNotchHeightText}
              min={CUSTOM_NOTCH_HEIGHT_MIN}
              max={CUSTOM_NOTCH_HEIGHT_MAX}
              step={1}
              onChange={(custom_notch_height) => onChange({ custom_notch_height })}
            />
            <span className="w-10 text-right text-[12px] tabular-nums text-white/55">
              {customNotchHeightText}
            </span>
          </SettingRow>
        )}
      </SettingsSection>

      <SettingsSection title={t("settings.appearance.content")}>
        <SettingRow
          htmlFor={fontSizeId}
          label={t("settings.appearance.contentFontSize")}
        >
          <div className="flex items-center gap-3">
            <SliderField
              id={fontSizeId}
              value={settings.content_font_size}
              min={CONTENT_FONT_SIZE_MIN}
              max={CONTENT_FONT_SIZE_MAX}
              step={1}
              onChange={(content_font_size) => onChange({ content_font_size })}
            />
            <span className="w-12 text-right text-[12px] tabular-nums text-white/55">
              {t("settings.appearance.contentFontSizeValue", {
                size: settings.content_font_size,
              })}
            </span>
          </div>
        </SettingRow>

        <SettingRow
          htmlFor={messageLinesId}
          label={t("settings.appearance.aiMessageLines")}
          description={t("settings.appearance.aiMessageLinesHint")}
        >
          <div className="flex items-center gap-3">
            <SliderField
              id={messageLinesId}
              value={settings.ai_message_lines}
              min={AI_MESSAGE_LINES_MIN}
              max={AI_MESSAGE_LINES_MAX}
              step={1}
              onChange={(ai_message_lines) => onChange({ ai_message_lines })}
            />
            <span className="w-16 text-right text-[12px] tabular-nums text-white/55">
              {settings.ai_message_lines === 0
                ? t("settings.appearance.aiMessageLinesUnlimited")
                : settings.ai_message_lines}
            </span>
          </div>
        </SettingRow>

        <SettingRow
          htmlFor={agentDetailsId}
          label={t("settings.appearance.showAgentDetails")}
        >
          <Toggle
            id={agentDetailsId}
            label={t("settings.appearance.showAgentDetails")}
            checked={settings.show_agent_details}
            onChange={(show_agent_details) => onChange({ show_agent_details })}
          />
        </SettingRow>

        <SettingRow
          htmlFor={toolStatusId}
          label={t("settings.appearance.showToolStatus")}
        >
          <Toggle
            id={toolStatusId}
            label={t("settings.appearance.showToolStatus")}
            checked={settings.show_tool_status}
            onChange={(show_tool_status) => onChange({ show_tool_status })}
          />
        </SettingRow>

        <SettingRow
          htmlFor={projectNameId}
          label={t("settings.appearance.showProjectName")}
          description={t("settings.appearance.showProjectNameHint")}
        >
          <Toggle
            id={projectNameId}
            label={t("settings.appearance.showProjectName")}
            checked={settings.show_project_name}
            onChange={(show_project_name) => onChange({ show_project_name })}
          />
        </SettingRow>

        <SettingRow
          htmlFor={gitBranchId}
          label={t("settings.appearance.showGitBranch")}
          description={t("settings.appearance.showGitBranchHint")}
        >
          <Toggle
            id={gitBranchId}
            label={t("settings.appearance.showGitBranch")}
            checked={settings.show_git_branch}
            onChange={(show_git_branch) => onChange({ show_git_branch })}
          />
        </SettingRow>

        <SettingRow
          htmlFor={taskProgressId}
          label={t("settings.appearance.showTaskProgress")}
          description={t("settings.appearance.showTaskProgressHint")}
        >
          <Toggle
            id={taskProgressId}
            label={t("settings.appearance.showTaskProgress")}
            checked={settings.show_task_progress}
            onChange={(show_task_progress) => onChange({ show_task_progress })}
          />
        </SettingRow>

        <SettingRow
          htmlFor={sessionRecapId}
          label={t("settings.appearance.showSessionRecap")}
          description={t("settings.appearance.showSessionRecapHint")}
        >
          <Toggle
            id={sessionRecapId}
            label={t("settings.appearance.showSessionRecap")}
            checked={settings.show_session_recap}
            onChange={(show_session_recap) => onChange({ show_session_recap })}
          />
        </SettingRow>

        <SettingRow
          htmlFor={modelLabelId}
          label={t("settings.appearance.showModelLabel")}
          description={t("settings.appearance.showModelLabelHint")}
        >
          <Toggle
            id={modelLabelId}
            label={t("settings.appearance.showModelLabel")}
            checked={settings.show_model_label}
            onChange={(show_model_label) => onChange({ show_model_label })}
          />
        </SettingRow>

        <SettingRow
          htmlFor={usageStatsId}
          label={t("settings.appearance.showUsageStats")}
          description={t("settings.appearance.showUsageStatsHint")}
        >
          <Toggle
            id={usageStatsId}
            label={t("settings.appearance.showUsageStats")}
            checked={settings.show_usage_stats}
            onChange={(show_usage_stats) => onChange({ show_usage_stats })}
          />
        </SettingRow>

        <SettingRow
          htmlFor={claudeQuotaId}
          label={t("settings.appearance.showClaudeQuota")}
          description={t("settings.appearance.showClaudeQuotaHint")}
        >
          <Toggle
            id={claudeQuotaId}
            label={t("settings.appearance.showClaudeQuota")}
            checked={settings.show_claude_quota}
            onChange={(show_claude_quota) => onChange({ show_claude_quota })}
          />
        </SettingRow>
        <SettingRow htmlFor={claudeQuotaChipId} label={t("quota.chipLabel")}>
          <SelectField
            id={claudeQuotaChipId}
            value={settings.claude_quota_chip}
            disabled={!settings.show_claude_quota}
            onChange={(claude_quota_chip) => onChange({ claude_quota_chip })}
            options={[
              { value: "off", label: t("quota.off") },
              { value: "auto", label: t("quota.auto") },
              { value: "session", label: t("quota.session") },
              { value: "weeklyAll", label: t("quota.weekly") },
              { value: "weeklyScoped", label: t("quota.weeklyScoped") },
            ]}
          />
        </SettingRow>
      </SettingsSection>
    </div>
  );
}
