import type { ButtonHTMLAttributes, ReactNode } from "react";

export type PixelButtonVariant =
  | "default"
  | "primary"
  | "danger"
  | "success"
  | "ghost"
  | "link";

export function PixelButton({
  variant = "default",
  className = "",
  type = "button",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: PixelButtonVariant }) {
  return (
    <button
      type={type}
      data-variant={variant}
      className={`pixel-button ${className}`}
      {...props}
    />
  );
}

export function IconButton({
  label,
  children,
  className = "",
  type = "button",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  label: string;
  children: ReactNode;
}) {
  return (
    <button
      type={type}
      aria-label={label}
      title={label}
      className={`island-icon ${className}`}
      {...props}
    >
      <span className="island-icon-face" aria-hidden="true">{children}</span>
    </button>
  );
}
