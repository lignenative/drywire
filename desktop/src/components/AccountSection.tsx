// The settings' account page (ADR 0055, the canvas's V4 page): the user
// name and the address this device signed in with, the account's
// devices (any other one can be signed out here), and signing this one
// out. No code to keep: the address is the way back.
import { useEffect, useState } from "react";
import { cn } from "cn";
import { api, type AccountInfoView, type DeviceInfoView } from "@/api";
import { loadAccount, saveAccount } from "@/account";
import { Button } from "@/components/ui/button";
import { errorText, useLanguage } from "@/lang";

/** Tells the window the account changed, so it shows the sign-in. */
export const ACCOUNT_CHANGED = "drywire-account";

export function AccountSection({ narrow }: { narrow: boolean }) {
  const { t, lang } = useLanguage();
  const account = loadAccount();
  const [info, setInfo] = useState<AccountInfoView | null>(null);
  const [devices, setDevices] = useState<DeviceInfoView[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = () => {
    if (!account) return;
    api.accountInfo(account.instance).then(setInfo, (e) => setError(errorText(t, e)));
    api.accountDevices(account.instance).then(setDevices, (e) => setError(errorText(t, e)));
  };
  useEffect(load, []);

  const removeDevice = async (key: string) => {
    if (!account) return;
    setError(null);
    try {
      await api.removeDevice(account.instance, key);
      load();
    } catch (e) {
      setError(errorText(t, e));
    }
  };

  const signOut = async () => {
    if (!account) return;
    setBusy(true);
    setError(null);
    try {
      await api.signOut(account.instance);
      saveAccount(null);
      window.dispatchEvent(new Event(ACCOUNT_CHANGED));
    } catch (e) {
      setError(errorText(t, e));
      setBusy(false);
    }
  };

  const row = cn("flex border-b border-line py-4.5", narrow ? "flex-col gap-1" : "items-baseline gap-4");
  const label = "text-[0.9375rem] font-medium";
  const help = "text-[0.8125rem] text-muted-foreground";
  const button = narrow ? "h-11 rounded-xl px-3.5 text-sm" : "";
  const seen = (iso: string) =>
    iso ? new Date(iso).toLocaleDateString(lang === "fr" ? "fr-FR" : "en-GB", { day: "numeric", month: "long" }) : "?";

  if (!account) {
    return <p className="border-b border-line py-4.5 text-[0.8125rem] text-muted-foreground">{t("account.notSignedIn")}</p>;
  }
  return (
    <>
      <div className={row}>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <div className={label}>{t("account.username")}</div>
          {!narrow && <div className={help}>{t("account.usernameHelp")}</div>}
        </div>
        <span className="font-mono text-sm font-medium text-soft">{info?.username ?? account.username}</span>
        {narrow && <div className={help}>{t("account.usernameHelp")}</div>}
      </div>
      <div className={row}>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <div className={label}>{t("account.email")}</div>
          {!narrow && <div className={help}>{t("account.emailHelp")}</div>}
        </div>
        <span className="text-sm text-soft">{info?.email ?? ""}</span>
        {narrow && <div className={help}>{t("account.emailHelp")}</div>}
      </div>
      <div className="flex flex-col gap-2.5 border-b border-line py-4.5">
        <div className={label}>{t("account.devices")}</div>
        {devices.map((d) => (
          <div key={d.publicKey} className="flex min-h-10 items-center gap-3">
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="text-sm">{d.thisDevice ? t("account.thisDevice") : t("account.otherDevice", { date: seen(d.lastSeenAt) })}</span>
              <span className="font-mono text-xs text-muted-foreground">
                {d.publicKey.slice(0, 8)}…{d.publicKey.slice(-8)}
              </span>
            </span>
            {!d.thisDevice && (
              <Button variant="secondary" size="md" onClick={() => removeDevice(d.publicKey)} className={button}>
                {t("account.signOutDevice")}
              </Button>
            )}
          </div>
        ))}
      </div>
      <div className={cn("flex py-4.5", narrow ? "flex-col gap-2.5" : "items-center gap-4")}>
        <div className="flex min-w-0 flex-1 flex-col gap-0.5">
          <div className={label}>{t("account.signOut")}</div>
          <div className={help}>{t("account.signOutHelp")}</div>
        </div>
        <Button
          variant="secondary"
          size="md"
          disabled={busy}
          onClick={signOut}
          className={cn(button, "self-start bg-[rgba(242,85,90,0.14)] text-[#ff8a8e] hover:bg-[rgba(242,85,90,0.22)]")}
        >
          {t("account.signOut")}
        </Button>
      </div>
      {error && (
        <p role="alert" className="pt-3 text-[0.8125rem] text-danger">
          {error}
        </p>
      )}
    </>
  );
}
