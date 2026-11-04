// The settings page. In the wide window, its own menu on the left and the
// page on the right; in the narrow one, the menu first, then one page.
import { ChevronLeft, ChevronRight } from "lucide-react";
import { useState } from "react";
import { cn } from "cn";
import { Button } from "@/components/ui/button";
import { DEFAULT_GATE_DBFS, formatDb } from "@/format";
import { useLanguage } from "@/lang";
import { SETTINGS_TABS, type AppState, type SettingsTab } from "@/state";
import { AccountSection } from "./AccountSection";
import { AdvancedSection, DisplaySection, SoundsSection, VoiceSection } from "./SettingsSections";

function Section({ tab, app, narrow }: { tab: SettingsTab; app: AppState; narrow: boolean }) {
  switch (tab) {
    case "voice":
      return <VoiceSection app={app} narrow={narrow} />;
    case "sounds":
      return <SoundsSection app={app} narrow={narrow} />;
    case "display":
      return <DisplaySection app={app} narrow={narrow} />;
    case "account":
      return <AccountSection narrow={narrow} />;
    case "advanced":
      return <AdvancedSection narrow={narrow} />;
  }
}

export function SettingsPage({ app }: { app: AppState }) {
  const { t } = useLanguage();
  const tab = app.settingsTab;
  return (
    <div className="flex h-full overflow-hidden">
      <nav
        aria-label={t("settings.title")}
        className="flex w-62 shrink-0 flex-col gap-0.5 border-r border-line bg-sidebar px-3 pt-4.5 pb-5"
      >
        <div className="flex items-center gap-1.5 pb-4.5">
          <Button variant="ghost" size="icon-md" aria-label={t("settings.back")} onClick={app.closeSettings} className="text-subtle">
            <ChevronLeft />
          </Button>
          <span className="text-[0.9375rem] font-semibold">{t("settings.title")}</span>
        </div>
        {SETTINGS_TABS.map((id) => (
          <Button
            key={id}
            variant="ghost"
            aria-current={tab === id ? "page" : undefined}
            onClick={() => app.setSettingsTab(id)}
            className={cn(
              "h-10 justify-start rounded-[0.625rem] px-3 text-[0.90625rem] font-medium",
              tab === id ? "bg-raised text-white hover:bg-raised" : "text-subtle",
            )}
          >
            {t(`settings.tab.${id}`)}
          </Button>
        ))}
        <span className="flex-1" />
        <p className="px-3 text-[0.78125rem] text-dim">{t("settings.instant")}</p>
      </nav>
      <div className="min-w-0 flex-1 overflow-y-auto px-14 pt-11 pb-10">
        <div className="flex max-w-150 flex-col">
          <h2 className="mb-1.5 text-[1.375rem] font-semibold tracking-[-0.02em]">{t(`settings.tab.${tab}`)}</h2>
          <Section tab={tab} app={app} narrow={false} />
        </div>
      </div>
    </div>
  );
}

/** The narrow window's settings: a menu of the pages, then one page. */
export function NarrowSettings({ app }: { app: AppState }) {
  const { lang, t } = useLanguage();
  const [page, setPage] = useState<SettingsTab | null>(null);
  const s = app.settings;
  const values: Record<SettingsTab, { text: string; mono: boolean }> = {
    voice: { text: s ? formatDb(s.gateOpenDbfs ?? DEFAULT_GATE_DBFS) : "", mono: true },
    sounds: {
      text: !s ? "" : s.soundVolume === 0 ? t("settings.off") : `${Math.round(s.soundVolume * 100)} %`,
      mono: true,
    },
    display: { text: t(`language.${lang}`), mono: false },
    account: { text: "", mono: false },
    advanced: { text: "", mono: false },
  };

  if (page === null) {
    return (
      <div className="flex h-full flex-col overflow-hidden pt-1 pb-3">
        <header className="flex items-center gap-1 py-2 pr-5 pl-1.5">
          <Button variant="ghost" aria-label={t("settings.back")} onClick={app.closeSettings} className="size-11 rounded-xl text-subtle">
            <ChevronLeft className="size-5.5" />
          </Button>
        </header>
        <div className="flex min-h-0 flex-1 flex-col gap-4.5 overflow-y-auto px-4 pt-1 pb-4">
          <h1 className="px-1 text-[1.875rem] font-semibold tracking-[-0.03em]">{t("settings.title")}</h1>
          <nav aria-label={t("settings.title")} className="flex flex-col overflow-hidden rounded-[1.125rem] bg-muted">
            {SETTINGS_TABS.map((id, i) => (
              <Button
                key={id}
                variant="ghost"
                onClick={() => setPage(id)}
                className={cn(
                  "h-15 justify-start gap-2.5 rounded-none pr-3.5 pl-4.5 text-[1.0625rem] font-medium",
                  i < SETTINGS_TABS.length - 1 && "border-b border-accent",
                )}
              >
                <span className="flex-1 text-left">{t(`settings.tab.${id}`)}</span>
                <span className={cn("text-[0.9375rem] font-normal text-muted-foreground", values[id].mono && "font-mono")}>
                  {values[id].text}
                </span>
                <ChevronRight className="size-4.5 text-dim" strokeWidth={2.2} />
              </Button>
            ))}
          </nav>
          <p className="px-1 text-[0.8125rem] text-dim">{t("settings.instant")}</p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col overflow-hidden pt-1 pb-3">
      <header className="flex items-center gap-1 py-2 pr-5 pl-1.5">
        <Button
          variant="ghost"
          aria-label={t("settings.backTo")}
          onClick={() => setPage(null)}
          className="h-11 gap-0.5 rounded-xl pr-2.5 pl-1 text-base font-medium text-subtle"
        >
          <ChevronLeft className="size-5.5" />
          {t("settings.title")}
        </Button>
      </header>
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto px-5 pt-1 pb-4">
        <h1 className="mb-1 text-[1.875rem] font-semibold tracking-[-0.03em]">{t(`settings.tab.${page}`)}</h1>
        <Section tab={page} app={app} narrow />
      </div>
    </div>
  );
}
