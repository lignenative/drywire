// The few glyphs of the canvas that Lucide does not draw: the logo, the
// connection's signal bars, the voice gate's step, the hang-up handset,
// and the bars that move while someone speaks. Everything else comes from
// lucide-react.
import { cn } from "cn";
import type { SVGProps } from "react";

export function Logo({ size = 26, label }: { size?: number; label?: string }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 44 44"
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      className="shrink-0"
    >
      <rect width="44" height="44" rx="13" className="fill-primary" />
      <path
        d="M9 27h8V16h10v11h8"
        fill="none"
        className="stroke-primary-foreground"
        strokeWidth="3.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}

/** Three bars, lit from the left: 3 for a good link, 1 for a bad one, 0
    while there is none. */
export function SignalBars({ bars, poor, size = 14 }: { bars: number; poor?: boolean; size?: number }) {
  const lit = poor ? "fill-alarm" : "fill-primary";
  return (
    <svg width={size} height={size} viewBox="0 0 14 14" aria-hidden="true" className="shrink-0">
      <rect x="1" y="9" width="3" height="4" rx="1" className={bars >= 1 ? lit : "fill-idle"} />
      <rect x="5.5" y="5.5" width="3" height="7.5" rx="1" className={bars >= 2 ? lit : "fill-idle"} />
      <rect x="10" y="2" width="3" height="11" rx="1" className={bars >= 3 ? lit : "fill-idle"} />
    </svg>
  );
}

function Stroke({ children, className, ...props }: SVGProps<SVGSVGElement>) {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={cn("size-[18px] shrink-0", className)}
      {...props}
    >
      {children}
    </svg>
  );
}

/** The voice gate: a step that only lets the loud part through. */
export function GateIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Stroke strokeWidth="2.2" {...props}>
      <path d="M3 15h4V8h10v7h4" />
    </Stroke>
  );
}

/** Leave the room: a handset put down. */
export function LeaveIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Stroke {...props}>
      <path d="M4.5 15.5c4.2-4 10.8-4 15 0l-1.8 2.3-3.2-1.1v-2.4a9 9 0 0 0-5 0v2.4l-3.2 1.1z" />
    </Stroke>
  );
}

/** The microphone, struck through when muted. */
export function MicIcon({ muted, ...props }: SVGProps<SVGSVGElement> & { muted?: boolean }) {
  return (
    <Stroke {...props}>
      <rect x="9" y="3" width="6" height="11" rx="3" />
      <path d="M5 11a7 7 0 0 0 14 0" />
      <path d="M12 18v3" />
      {muted && <path d="M4 4l16 16" />}
    </Stroke>
  );
}

/** The encryption lock: closed when everyone heard is verified, its
    shackle lifted otherwise. */
export function LockIcon({ open, ...props }: SVGProps<SVGSVGElement> & { open?: boolean }) {
  return (
    <Stroke {...props}>
      <rect x="5" y="11" width="14" height="10" rx="2.5" />
      <path d={open ? "M8 11V8a4 4 0 0 1 7.5-1.9" : "M8 11V8a4 4 0 0 1 8 0v3"} />
    </Stroke>
  );
}

/** The space's key, which never leaves the members' devices. */
export function KeyIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Stroke strokeWidth="2.2" {...props}>
      <circle cx="8" cy="15" r="4" />
      <path d="M11 12l9-9M17 6l3 3M15 8l2 2" />
    </Stroke>
  );
}

/** A dashed circle turning: someone's voice is being checked. */
export function CheckingIcon({ className, ...props }: SVGProps<SVGSVGElement>) {
  return (
    <Stroke strokeWidth="2.4" className={cn("size-3.5 animate-spin [animation-duration:2.4s]", className)} {...props}>
      <circle cx="12" cy="12" r="8" strokeDasharray="4 4" />
    </Stroke>
  );
}

/** The settings: three sliders. */
export function SettingsIcon(props: SVGProps<SVGSVGElement>) {
  return (
    <Stroke {...props}>
      <path d="M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12" />
      <circle cx="16" cy="6" r="2" />
      <circle cx="10" cy="12" r="2" />
      <circle cx="18" cy="18" r="2" />
    </Stroke>
  );
}

/** Waiting for the relay: an arrow going round. */
export function SpinIcon({ className, ...props }: SVGProps<SVGSVGElement>) {
  return (
    <Stroke strokeWidth="2.2" className={cn("size-4 animate-spin [animation-duration:2.4s]", className)} {...props}>
      <path d="M20 11a8 8 0 1 0-2.3 6.3M20 4v7h-7" />
    </Stroke>
  );
}

/** Three bars that move while someone speaks. Each bar has its own rhythm
    (see `speaking-bar` in index.css), so a name next to them reads as a
    voice, not a spinner. */
export function SpeakingBars({ size = "md", label }: { size?: "sm" | "md" | "lg"; label: string }) {
  const bar = {
    sm: "w-0.5 h-3",
    md: "w-[3px] h-3.5",
    lg: "w-1 h-[26px]",
  }[size];
  return (
    <span role="img" aria-label={label} className={cn("flex items-center", size === "lg" ? "gap-[3px]" : "gap-0.5")}>
      {[0, 1, 2].map((i) => (
        <span
          key={i}
          className={cn("speaking-bar rounded-[2px] bg-primary", bar)}
          style={{ animationDelay: `${i * -0.23}s`, animationDuration: `${0.7 + i * 0.17}s` }}
        />
      ))}
    </span>
  );
}
