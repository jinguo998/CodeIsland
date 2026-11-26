import { useId, type ReactNode } from "react";
import { clamp } from "../lib/settings";
import type { AppSettings } from "../lib/types";

export interface SettingsTabProps {
  settings: AppSettings;
  onChange: (patch: Partial<AppSettings>) => void;
}

/** A grouped form section: small gray header over a rounded card. */
export function SettingsSection({
  title,
  children,
}: {
  title?: string;
  children: ReactNode;
}) {
  return (
    <section className="mb-5">
      {title && (
        <h2 className="mb-1.5 px-1 text-[11px] font-medium text-white/45">
          {title}
        </h2>
      )}
      <div className="overflow-hidden rounded-[10px] bg-white/[0.055]">
        {children}
      </div>
    </section>
  );
}

export function SettingRow({
  label,
  description,
  htmlFor,
  children,
}: {
  label: string;
  description?: ReactNode;
  htmlFor: string;
  children: ReactNode;
}) {
  return (
    <div className="flex min-h-[42px] items-center justify-between gap-4 border-b border-white/[0.06] px-3.5 py-2 last:border-b-0">
      <div className="min-w-0">
        <label htmlFor={htmlFor} className="block text-[13px] text-white/90">
          {label}
        </label>
        {description && (
          <p className="mt-0.5 text-[11px] leading-snug text-white/45">
            {description}
          </p>
        )}
      </div>
      <div className="flex flex-none items-center gap-2">{children}</div>
    </div>
  );
}

export function Toggle({
  id,
  checked,
  label,
  onChange,
}: {
  id: string;
  checked: boolean;
  label: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label htmlFor={id} className="relative inline-flex cursor-pointer items-center">
      <input
        id={id}
        type="checkbox"
        checked={checked}
        aria-label={label}
        onChange={(event) => onChange(event.currentTarget.checked)}
        className="peer sr-only"
      />
      <span
        aria-hidden="true"
        className={`relative block h-[22px] w-[38px] rounded-full transition-colors duration-150 peer-focus-visible:outline peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-[#4dd966] ${
          checked ? "bg-[#4dd966]" : "bg-white/20"
        }`}
      >
        <span
          className={`absolute top-[2px] h-[18px] w-[18px] rounded-full bg-white shadow transition-[left] duration-150 ${
            checked ? "left-[18px]" : "left-[2px]"
          }`}
        />
      </span>
    </label>
  );
}

export interface SelectOption {
  value: string;
  label: string;
}

export function SelectField({
  id,
  value,
  options,
  onChange,
  disabled,
}: {
  id: string;
  value: string;
  options: SelectOption[];
  onChange: (value: string) => void;
  disabled?: boolean;
}) {
  return (
    <select
      id={id}
      value={value}
      disabled={disabled}
      onChange={(event) => onChange(event.currentTarget.value)}
      className="min-w-[120px] rounded-md border border-white/10 bg-white/[0.08] px-2 py-1 text-[12px] text-white/90 outline-none focus:border-[#4dd966]/60"
    >
      {options.map((option) => (
        <option key={option.value} value={option.value} className="bg-[#161618]">
          {option.label}
        </option>
      ))}
    </select>
  );
}

export function NumberField({
  id,
  value,
  min,
  max,
  onChange,
}: {
  id: string;
  value: number;
  min: number;
  max: number;
  onChange: (value: number) => void;
}) {
  return (
    <input
      id={id}
      type="number"
      min={min}
      max={max}
      value={value}
      onChange={(event) => {
        const raw = event.currentTarget.value;
        if (raw === "") return;
        const parsed = Number(raw);
        if (!Number.isFinite(parsed)) return;
        onChange(clamp(parsed, min, max));
      }}
      className="w-16 rounded-md border border-white/10 bg-white/[0.08] px-2 py-1 text-right text-[12px] tabular-nums text-white/90 outline-none focus:border-[#4dd966]/60"
    />
  );
}

export function TextField({
  id,
  value,
  placeholder,
  invalid = false,
  onChange,
  onBlur,
  type = "text",
  className = "",
}: {
  id: string;
  value: string;
  placeholder?: string;
  invalid?: boolean;
  onChange: (value: string) => void;
  onBlur?: () => void;
  type?: "text" | "password";
  className?: string;
}) {
  return (
    <input
      id={id}
      type={type}
      value={value}
      placeholder={placeholder}
      aria-invalid={invalid || undefined}
      onChange={(event) => onChange(event.currentTarget.value)}
      onBlur={onBlur}
      className={`rounded-md border bg-white/[0.08] px-2 py-1 text-[12px] text-white/90 placeholder:text-white/30 outline-none focus:border-[#4dd966]/60 ${
        invalid ? "border-[#ff616b]" : "border-white/10"
      } ${className}`}
    />
  );
}

export function TimeField({
  id,
  value,
  onChange,
}: {
  id: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <input
      id={id}
      type="time"
      value={value}
      onChange={(event) => onChange(event.currentTarget.value)}
      className="rounded-md border border-white/10 bg-white/[0.08] px-2 py-1 text-[12px] tabular-nums text-white/90 outline-none focus:border-[#4dd966]/60"
    />
  );
}

export function SliderField({
  id,
  value,
  min,
  max,
  step,
  valueText,
  disabled,
  onChange,
}: {
  id: string;
  value: number;
  min: number;
  max: number;
  step: number;
  valueText?: string;
  disabled?: boolean;
  onChange: (value: number) => void;
}) {
  return (
    <input
      id={id}
      type="range"
      value={value}
      min={min}
      max={max}
      step={step}
      aria-valuetext={valueText}
      disabled={disabled}
      onChange={(event) => onChange(Number(event.currentTarget.value))}
      className="settings-slider w-40 disabled:opacity-45"
    />
  );
}

export function SegmentedControl<T extends string>({
  value,
  options,
  onChange,
}: {
  value: T;
  options: Array<{ value: T; label: string }>;
  onChange: (value: T) => void;
}) {
  return (
    <div className="inline-flex rounded-md bg-white/[0.08] p-0.5">
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          onClick={() => onChange(option.value)}
          className={`rounded-[5px] px-3 py-1 text-[11px] font-medium transition-colors ${
            value === option.value
              ? "bg-white/15 text-white"
              : "text-white/55 hover:text-white/80"
          }`}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

/** Convenience hook for controls that need a stable label/input id. */
export function useControlId(prefix: string): string {
  const id = useId();
  return `${prefix}-${id}`;
}
