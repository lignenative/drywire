// The first screen of a device that has no space yet (the canvas's V3
// page, "Bienvenue"): what an account is here, and the three ways in. No
// side menu, since there is nothing to list. Each way opens the invitation
// dialog in its mode, which asks for the person's name where one is
// needed; settings stay one click away, for the microphone.
import { cn } from "cn";
import { platform } from "@/api";
import { Button } from "@/components/ui/button";
import { useLanguage } from "@/lang";
import type { AppState } from "@/state";
import { Logo, SettingsIcon } from "./icons";

export function Welcome({ app, narrow }: { app: AppState; narrow: boolean }) {
  const { t } = useLanguage();
  const action = "h-12 w-full rounded-[0.875rem] text-[0.9375rem] pointer-coarse:h-13";
  return (
    <div className={cn("flex h-full flex-col", narrow && "pt-1 pb-3")}>
      <header className={cn("flex justify-end", narrow ? "px-2 pt-1" : "px-4 pt-3.5")}>
        <Button
          variant="ghost"
          aria-label={t("settings.title")}
          title={t("settings.title")}
          onClick={() => app.openSettings()}
          className={cn("rounded-xl text-subtle", narrow ? "size-11" : "size-10")}
        >
          <SettingsIcon className="size-5" />
        </Button>
      </header>
      <main
        className={cn(
          "flex min-h-0 flex-1 flex-col items-center justify-center",
          // On a phone the choices sit low, near the thumb.
          "pointer-coarse:justify-end",
          narrow ? "px-5 pb-6 pointer-coarse:pb-4" : "px-14 pb-18",
        )}
      >
        <div className={cn("flex w-full flex-col gap-3.5", narrow ? "max-w-85" : "max-w-95")}>
          <Logo size={narrow ? 40 : 44} />
          <h1
            className={cn(
              "mt-1.5 leading-[1.1] font-semibold tracking-[-0.03em]",
              narrow ? "text-[1.625rem] pointer-coarse:text-[1.875rem]" : "text-[2.125rem]",
            )}
          >
            {t("welcome.title")}
          </h1>
          <p className={cn("text-soft", narrow ? "text-base pointer-coarse:text-[1.0625rem]" : "text-[1.0625rem]")}>
            {t("welcome.lead")}
          </p>
          <p className="text-sm text-subtle">{t(platform === "web" ? "welcome.accountWeb" : "welcome.accountApp")}</p>
          <div className="mt-4.5 flex flex-col gap-2">
            <Button onClick={() => app.openInvite("invite")} className={cn(action, "font-semibold")}>
              {t("welcome.join")}
            </Button>
            <Button variant="secondary" onClick={() => app.openInvite("create")} className={cn(action, "font-semibold")}>
              {t("welcome.create")}
            </Button>
            <Button
              variant="ghost"
              onClick={() => app.openInvite("restore")}
              className={cn(action, "font-medium text-soft")}
            >
              {t("welcome.restore")}
            </Button>
          </div>
        </div>
      </main>
    </div>
  );
}
