export function AppLogo({ showBackground = false }: { showBackground?: boolean }) {
  return (
    <span
      className={showBackground ? "app-logo" : "app-logo island-logo"}
      data-background={showBackground || undefined}
      aria-hidden="true"
    >
      <span />
      <span />
    </span>
  );
}
