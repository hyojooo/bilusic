/**
 * A small styled Select that replaces the platform `<select>` element so the
 * Settings page matches the surrounding pill / panel design language.
 *
 * - Trigger styled like `.field` (surface-2 + border, rounded-xl) with a
 *   chevron that flips when open.
 * - Panel is a `<ul role="listbox">` with full keyboard support per the
 *   WAI-ARIA Listbox pattern:
 *      Arrow Up/Down — move active option (wraps)
 *      Home / End    — first / last option
 *      Enter / Space — select active option
 *      Escape        — close + restore focus to trigger
 * - Panel animation is a CSS fade + slide-up; respects
 *   `prefers-reduced-motion`.
 * - Click outside / blur on trigger closes the panel.
 */
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";

export interface SelectOption {
  value: string;
  label: ReactNode;
}

export interface SelectProps {
  value: string;
  onChange: (next: string) => void;
  options: SelectOption[];
  /** Visible caption rendered above the trigger (small, muted). */
  label?: ReactNode;
  /** Required for accessibility when no visible `label`. */
  ariaLabel?: string;
  /** Disable interaction. */
  disabled?: boolean;
  className?: string;
}

const ChevronIcon = ({ flipped }: { flipped: boolean }) => (
  <svg
    width="16"
    height="16"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true"
    style={{
      transition: "transform 200ms cubic-bezier(0.16, 1, 0.3, 1)",
      transform: flipped ? "rotate(180deg)" : "rotate(0deg)",
    }}
  >
    <polyline points="6 9 12 15 18 9" />
  </svg>
);

const CheckIcon = () => (
  <svg
    width="14"
    height="14"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="2.5"
    strokeLinecap="round"
    strokeLinejoin="round"
    aria-hidden="true"
  >
    <polyline points="20 6 9 17 4 12" />
  </svg>
);

export function Select({
  value,
  onChange,
  options,
  label,
  ariaLabel,
  disabled = false,
  className = "",
}: SelectProps) {
  const listboxId = useId();
  const triggerId = useId();
  const rootRef = useRef<HTMLDivElement | null>(null);
  const triggerRef = useRef<HTMLButtonElement | null>(null);
  const listRef = useRef<HTMLUListElement | null>(null);

  const [open, setOpen] = useState(false);
  const initialIdx = useMemo(
    () => Math.max(0, options.findIndex((o) => o.value === value)),
    [options, value],
  );
  const [activeIdx, setActiveIdx] = useState<number>(initialIdx);

  // Keep `activeIdx` in sync when the value changes externally (programmatically
  // — happens when the user's persisted selection is restored).
  useEffect(() => {
    if (!open) setActiveIdx(initialIdx);
  }, [initialIdx, open]);

  // Close panel when clicking outside the root.
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (rootRef.current && !rootRef.current.contains(e.target as Node)) {
        setOpen(false);
        triggerRef.current?.focus();
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  // Focus the active option when the panel opens.
  useEffect(() => {
    if (!open) return;
    const id = listboxId;
    requestAnimationFrame(() => {
      const el = document.getElementById(`${id}-opt-${activeIdx}`);
      el?.scrollIntoView({ block: "nearest" });
      // Do not move focus here — only on keyboard nav; this prevents the
      // listbox from stealing focus on mouse click.
    });
  }, [open, listboxId, activeIdx]);

  const focusOption = useCallback(
    (idx: number) => {
      const el = document.getElementById(`${listboxId}-opt-${idx}`);
      el?.focus();
      el?.scrollIntoView({ block: "nearest" });
    },
    [listboxId],
  );

  const handleTriggerKey = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (disabled) return;
    if (e.key === "ArrowDown" || e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      setOpen(true);
      const start = Math.max(0, options.findIndex((o) => o.value === value));
      setActiveIdx(start);
      // Focus the start option after the panel mounts.
      requestAnimationFrame(() => focusOption(start));
    }
  };

  const handleOptionKey = (
    e: KeyboardEvent<HTMLLIElement>,
    idx: number,
  ) => {
    const last = options.length - 1;
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        setActiveIdx((idx + 1) % options.length);
        focusOption((idx + 1) % options.length);
        break;
      case "ArrowUp":
        e.preventDefault();
        setActiveIdx((idx - 1 + options.length) % options.length);
        focusOption((idx - 1 + options.length) % options.length);
        break;
      case "Home":
        e.preventDefault();
        setActiveIdx(0);
        focusOption(0);
        break;
      case "End":
        e.preventDefault();
        setActiveIdx(last);
        focusOption(last);
        break;
      case "Enter":
      case " ": {
        e.preventDefault();
        const opt = options[idx];
        onChange(opt.value);
        setOpen(false);
        triggerRef.current?.focus();
        break;
      }
      case "Escape":
      case "Tab":
        // Escape closes + restores focus; Tab lets the browser handle
        // focus order naturally.
        if (e.key === "Escape") {
          e.preventDefault();
          setOpen(false);
          triggerRef.current?.focus();
        }
        break;
    }
  };

  const selected = options.find((o) => o.value === value);

  return (
    <div ref={rootRef} className={`relative ${className}`}>
      {label !== undefined && (
        <span className="mb-1.5 block text-xs font-medium text-muted">
          {label}
        </span>
      )}
      <button
        ref={triggerRef}
        id={triggerId}
        type="button"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={open ? listboxId : undefined}
        aria-label={ariaLabel}
        onClick={() => !disabled && setOpen((o) => !o)}
        onKeyDown={handleTriggerKey}
        className={`group flex w-full items-center justify-between gap-2 rounded-xl border bg-surface-2 px-3 py-2 text-left text-sm text-fg outline-none transition-colors ${
          open
            ? "border-accent-500"
            : "border-line hover:border-accent-500/40"
        } ${disabled ? "cursor-not-allowed opacity-50" : ""}`}
      >
        <span className="truncate">{selected?.label}</span>
        <ChevronIcon flipped={open} />
      </button>

      {open && (
        <ul
          ref={listRef}
          id={listboxId}
          role="listbox"
          aria-labelledby={triggerId}
          tabIndex={-1}
          // Position the panel flush with the trigger; CSS-only fade+slide.
          className="select-panel absolute left-0 right-0 top-full z-30 mt-1 max-h-60 overflow-auto rounded-xl border border-line bg-surface p-1 shadow-float"
        >
          {options.map((opt, idx) => {
            const isSelected = opt.value === value;
            const isActive = idx === activeIdx;
            return (
              <li
                key={opt.value}
                id={`${listboxId}-opt-${idx}`}
                role="option"
                aria-selected={isSelected}
                tabIndex={0}
                onClick={() => {
                  onChange(opt.value);
                  setOpen(false);
                  triggerRef.current?.focus();
                }}
                onMouseEnter={() => setActiveIdx(idx)}
                onKeyDown={(e) => handleOptionKey(e, idx)}
                className={`flex cursor-pointer items-center justify-between gap-2 rounded-lg px-3 py-2 text-sm outline-none transition-colors ${
                  isSelected
                    ? "bg-accent-500/10 text-accent-500"
                    : isActive
                      ? "bg-surface-2 text-fg"
                      : "text-fg"
                }`}
              >
                <span className="truncate">{opt.label}</span>
                {isSelected && <CheckIcon />}
              </li>
            );
          })}
        </ul>
      )}

      {/* Animations + a11y motion-preference: bare CSS so we keep deps
          minimal (no GSAP / Framer Motion needed for this small panel). */}
      <style>{`
        @keyframes select-panel-in {
          from { opacity: 0; transform: translateY(-4px) scaleY(0.96); }
          to   { opacity: 1; transform: translateY(0)     scaleY(1); }
        }
        .select-panel {
          transform-origin: top center;
          animation: select-panel-in 160ms cubic-bezier(0.16, 1, 0.3, 1);
        }
        @media (prefers-reduced-motion: reduce) {
          .select-panel { animation: none; }
        }
      `}</style>
    </div>
  );
}
