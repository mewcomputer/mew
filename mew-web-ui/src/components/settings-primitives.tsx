import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

export function SettingsGroup({
  title,
  description,
  action,
  children,
}: {
  title: string;
  description?: string;
  action?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="space-y-2">
      <div className="flex min-h-5 items-center justify-between gap-3 px-1">
        <h3 className="text-xs font-medium text-muted-foreground">
          {title}
        </h3>
        {action}
      </div>
      <div className="divide-y divide-border/60 overflow-hidden rounded-xl border border-border/80 bg-card/40">
        {children}
      </div>
      {description && (
        <p className="px-1 text-xs leading-relaxed text-muted-foreground">
          {description}
        </p>
      )}
    </section>
  );
}

export function SettingsRow({
  label,
  description,
  leading,
  trailing,
  onClick,
  disabled,
  className,
  labelClassName,
  ariaCurrent,
  role,
  ariaChecked,
}: {
  label: ReactNode;
  description?: ReactNode;
  leading?: ReactNode;
  trailing?: ReactNode;
  onClick?: () => void;
  disabled?: boolean;
  className?: string;
  labelClassName?: string;
  ariaCurrent?: boolean;
  role?: "radio" | "button";
  ariaChecked?: boolean;
}) {
  const body = (
    <>
      <div className="flex min-w-0 flex-1 items-start gap-3">
        {leading && <div className="mt-0.5 shrink-0">{leading}</div>}
        <div className="min-w-0 flex-1 space-y-0.5">
          <div
            className={cn(
              "text-[13px] font-medium leading-snug text-foreground",
              labelClassName,
            )}
          >
            {label}
          </div>
          {description && (
            <div className="text-xs leading-relaxed text-muted-foreground">
              {description}
            </div>
          )}
        </div>
      </div>
      {trailing && <div className="shrink-0">{trailing}</div>}
    </>
  );

  if (onClick) {
    return (
      <button
        type="button"
        role={role}
        aria-checked={ariaChecked}
        aria-current={ariaCurrent ? "true" : undefined}
        disabled={disabled}
        onClick={onClick}
        className={cn(
          "flex w-full items-center justify-between gap-4 px-4 py-3 text-left transition-colors duration-150 ease-out hover:bg-accent/40 active:bg-accent/60 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset disabled:cursor-not-allowed disabled:opacity-50 motion-reduce:transition-none",
          className,
        )}
      >
        {body}
      </button>
    );
  }

  return (
    <div
      className={cn(
        "flex items-center justify-between gap-4 px-4 py-3",
        className,
      )}
    >
      {body}
    </div>
  );
}

export function SettingsSwitch({
  checked,
  disabled,
  onChange,
  ariaLabel,
}: {
  checked: boolean;
  disabled?: boolean;
  onChange: () => void;
  ariaLabel: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-label={ariaLabel}
      aria-checked={checked}
      disabled={disabled}
      onClick={onChange}
      className={cn(
        "relative inline-flex h-[22px] w-[38px] shrink-0 items-center rounded-full p-0.5 transition-colors duration-150 ease-out active:scale-[0.96] disabled:cursor-not-allowed disabled:opacity-45 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background motion-reduce:transition-none motion-reduce:active:scale-100",
        checked ? "bg-primary" : "bg-muted-foreground/25",
      )}
    >
      <span
        className={cn(
          "pointer-events-none block h-[18px] w-[18px] rounded-full bg-background shadow-sm transition-transform duration-150 ease-out motion-reduce:transition-none",
          checked ? "translate-x-4" : "translate-x-0",
        )}
      />
    </button>
  );
}
