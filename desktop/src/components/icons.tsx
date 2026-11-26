import type { SVGProps } from "react";

type IconProps = SVGProps<SVGSVGElement>;

function Base({ children, ...props }: IconProps) {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.6"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...props}
    >
      {children}
    </svg>
  );
}

export function SpeakerIcon({ muted = false, ...props }: IconProps & { muted?: boolean }) {
  return (
    <Base {...props}>
      <path d="M2 6h3l3.5-3v10L5 10H2z" />
      {muted ? (
        <path d="M10.5 6l3 4m0-4-3 4" />
      ) : (
        <path d="M10.7 6a3 3 0 0 1 0 4m1.9-5.7a5.5 5.5 0 0 1 0 7.4" />
      )}
    </Base>
  );
}

export function GearIcon(props: IconProps) {
  return (
    <Base {...props}>
      <circle cx="8" cy="8" r="2.1" />
      <path d="M6.5 1.4h3l.3 1.7 1.4.8 1.6-.6 1.5 2.6-1.3 1.1v1.8l1.3 1.1-1.5 2.6-1.6-.6-1.4.8-.3 1.7h-3l-.3-1.7-1.4-.8-1.6.6-1.5-2.6L3 8.8V7L1.7 5.9l1.5-2.6 1.6.6 1.4-.8z" />
    </Base>
  );
}

export function PowerIcon(props: IconProps) {
  return (
    <Base {...props}>
      <path d="M8 1.8v5.6" />
      <path d="M4.6 4.2a5 5 0 1 0 6.8 0" />
    </Base>
  );
}

export function WaveformIcon({ custom = false, ...props }: IconProps & { custom?: boolean }) {
  return (
    <Base {...props}>
      {custom && <circle cx="8" cy="8" r="7" fill="currentColor" stroke="none" />}
      <path d="M4.5 7v2m2-4v6m2-8v10m2-8v6m2-4v2" stroke={custom ? "#1c1c1e" : "currentColor"} />
    </Base>
  );
}

export function PlayCircleIcon(props: IconProps) {
  return (
    <Base {...props}>
      <circle cx="8" cy="8" r="7.2" fill="currentColor" stroke="none" />
      <path d="m6.4 4.7 4.7 3.3-4.7 3.3z" fill="#1c1c1e" stroke="none" />
    </Base>
  );
}

export function FocusWindowIcon(props: IconProps) {
  return (
    <Base {...props}>
      <path d="M11.4 8.4v4H3.6V4.6h4m.6-1h4.2v4.2m0-4.2-5.1 5.1" />
    </Base>
  );
}

export function TrashIcon(props: IconProps) {
  return (
    <Base {...props}>
      <path d="M2.7 4.3h10.6M6 4.3V2.4h4v1.9m2 0-.7 9.1H4.7L4 4.3m2.6 2.3v4.5m2.8-4.5v4.5" />
    </Base>
  );
}

export function BellIcon(props: IconProps) {
  return (
    <Base {...props} fill="currentColor" stroke="none">
      <path d="M7 2.4a1 1 0 0 1 2 0v.2A4.1 4.1 0 0 1 12.1 6.6v2c0 1 .4 1.7 1.1 2.5a.8.8 0 0 1-.6 1.4H3.4a.8.8 0 0 1-.6-1.4c.7-.8 1.1-1.5 1.1-2.5v-2A4.1 4.1 0 0 1 7 2.6Zm-.8 11.1h3.6a1.8 1.8 0 0 1-3.6 0Z" />
    </Base>
  );
}

export function MoonIcon(props: IconProps) {
  return (
    <Base {...props}>
      <path d="M12.9 9.6a5.6 5.6 0 0 1-6.5-6.5 5.6 5.6 0 1 0 6.5 6.5Z" fill="currentColor" />
    </Base>
  );
}

export function QuestionBadgeIcon(props: IconProps) {
  return (
    <Base {...props}>
      <path d="M3.1 2h9.8A2.1 2.1 0 0 1 15 4.1v5.4a2.1 2.1 0 0 1-2.1 2.1h-4l-3.6 2.7a.5.5 0 0 1-.8-.4v-2.3H3.1A2.1 2.1 0 0 1 1 9.5V4.1A2.1 2.1 0 0 1 3.1 2Z" fill="currentColor" stroke="none" />
      <path d="M6.45 5.2a1.65 1.65 0 1 1 2.2 1.55c-.45.19-.65.55-.65.95M8 9.5h.01" stroke="#000" />
    </Base>
  );
}
