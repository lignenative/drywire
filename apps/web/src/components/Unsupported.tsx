// The web client in a browser that lacks what it needs: what is missing,
// then where Drywire runs (the canvas's "Web · navigateur non pris en
// charge", wide, narrow and phone).
import { X } from "lucide-react";
import { cn } from "cn";
import { Logo } from "@/components/icons";
import { useNarrow } from "@/hooks";
import { useLanguage } from "@/lang";
import { webText } from "../strings";

export function Unsupported({ missing }: { missing: string[] }) {
  const { lang, t } = useLanguage();
  const narrow = useNarrow();
  const w = (key: Parameters<typeof webText>[1]) => webText(lang, key);
  return (
    <div className="flex h-screen flex-col overflow-hidden bg-background text-sm leading-[1.4] text-foreground">
      <header className={cn("flex items-center gap-2.5", narrow ? "py-3 pr-3 pl-5" : "px-4 pt-4.5 pb-2.5")}>
        <Logo size={narrow ? 28 : 26} label={t("app.name")} />
        <span className={cn("font-bold tracking-[-0.02em]", narrow ? "text-lg" : "text-[0.9375rem]")}>
          {t("app.name")}
        </span>
      </header>
      <main role="alert" className={cn("flex min-h-0 flex-1 flex-col justify-center", narrow ? "px-6 pb-10" : "px-14 pb-20")}>
        <div className="mx-auto flex w-full max-w-140 flex-col gap-3">
          <h1
            className={cn(
              "leading-[1.15] font-semibold tracking-[-0.03em]",
              narrow ? "text-[1.75rem]" : "text-[2.125rem]",
            )}
          >
            {w("unsupported.title")}
          </h1>
          <p className={cn("max-w-115 text-subtle", narrow ? "text-[0.9375rem]" : "text-sm")}>
            {w("unsupported.missing")}
          </p>
          <ul className="flex flex-wrap gap-2">
            {missing.map((m) => (
              <li
                key={m}
                className="flex h-7.5 items-center gap-1.5 rounded-full border border-border pr-3 pl-2.5 font-mono text-[0.8125rem] font-medium text-soft"
              >
                <X className="size-3 text-danger" strokeWidth={2.6} aria-hidden />
                {m}
              </li>
            ))}
          </ul>
          <p className={cn("mt-2 max-w-115 text-subtle", narrow ? "text-[0.9375rem]" : "text-sm")}>
            {w("unsupported.where")}
          </p>
          <p className="max-w-115 text-[0.8125rem] text-muted-foreground">{w("unsupported.app")}</p>
        </div>
      </main>
    </div>
  );
}
