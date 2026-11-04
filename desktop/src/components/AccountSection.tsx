// The settings' account page (ADR 0051): the recovery code, a code to
// add a device, and the device key an operator asks for. A code is shown
// once, in a dialog; nothing keeps it on screen afterwards.
import { Clock } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { cn } from "cn";
import { api, type AccountView, type DeviceCodeView } from "@/api";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { errorText, useLanguage } from "@/lang";
import { DeviceKey } from "./SettingsSections";

/** What the dialog shows: a recovery code, or a device code with its
    instance and deadline (a time in milliseconds). */
type Shown =
  | { kind: "recovery"; code: string }
  | { kind: "device"; code: string; instance: string; until: number };

export function AccountSection({ narrow }: { narrow: boolean }) {
  const { t } = useLanguage();
  const [account, setAccount] = useState<AccountView | null>(null);
  const [shown, setShown] = useState<Shown | null>(null);
  const [busy, setBusy] = useState<"recovery" | "device" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = () => api.account().then(setAccount, (e) => setError(errorText(t, e)));
  useEffect(() => {
    load();
  }, []);

  const recovery = async () => {
    setBusy("recovery");
    setError(null);
    try {
      setShown({ kind: "recovery", code: await api.newRecoveryCode() });
      load();
    } catch (e) {
      setError(errorText(t, e));
    } finally {
      setBusy(null);
    }
  };

  const device = async () => {
    setBusy("device");
    setError(null);
    try {
      const d: DeviceCodeView = await api.newDeviceCode();
      setShown({ kind: "device", code: d.code, instance: d.instance, until: Date.now() + d.expiresInS * 1000 });
    } catch (e) {
      setError(errorText(t, e));
    } finally {
      setBusy(null);
    }
  };

  // Nothing to put in a vault without a space, and nothing at all from a
  // browser key that cannot be read.
  const blocked = !account || account.spaces === 0 || !account.portable;
  const row = cn("flex flex-wrap items-center gap-x-4 border-b border-line py-4.5", narrow && "gap-y-3");
  const text = cn("min-w-0 flex-1", narrow && "basis-full");
  // A disabled button keeps its pointer events, so the cursor says why
  // nothing happens (CLAUDE.md, "Clickable means the pointing hand").
  const action = "disabled:pointer-events-auto disabled:opacity-40";
  return (
    <>
      {account && !account.portable && (
        <p className="border-b border-line py-4.5 text-[0.8125rem] text-muted-foreground">{t("account.notPortable")}</p>
      )}
      {account && account.portable && account.spaces === 0 && (
        <p className="border-b border-line py-4.5 text-[0.8125rem] text-muted-foreground">{t("account.noSpace")}</p>
      )}
      <div className={row}>
        <div className={text}>
          <div className="text-[0.9375rem] font-medium">{t("account.recovery")}</div>
          <div className="text-[0.8125rem] text-muted-foreground">{t("account.recoveryHelp")}</div>
          {account?.recoveryCode && (
            <div className="mt-1 text-[0.8125rem] text-subtle">{t("account.recoveryKnown")}</div>
          )}
        </div>
        <Button variant="secondary" size="md" disabled={blocked || busy !== null} onClick={recovery} className={action}>
          {account?.recoveryCode ? t("account.recoveryRenew") : t("account.recoveryCreate")}
        </Button>
      </div>
      <div className={row}>
        <div className={text}>
          <div className="text-[0.9375rem] font-medium">{t("account.device")}</div>
          <div className="text-[0.8125rem] text-muted-foreground">{t("account.deviceHelp")}</div>
        </div>
        <Button variant="secondary" size="md" disabled={blocked || busy !== null} onClick={device} className={action}>
          {t("account.deviceShow")}
        </Button>
      </div>
      {error && (
        <p role="alert" className="pt-3 text-[0.8125rem] text-danger">
          {error}
        </p>
      )}
      <DeviceKey narrow={narrow} />
      <CodeDialog shown={shown} narrow={narrow} onClose={() => setShown(null)} onAgain={device} />
    </>
  );
}

/** Writes a code in groups of four, in mono, in a frame like a field: on
    one line in the wide dialog, wrapped after a hyphen in a narrow one,
    never inside a group. Greyed once it expired. */
function Code({ code, narrow, expired }: { code: string; narrow: boolean; expired: boolean }) {
  const groups = code.split("-");
  return (
    <code
      className={cn(
        "rounded-xl border border-input bg-background px-4 py-3.5 text-center font-mono font-medium tracking-[0.02em]",
        narrow ? "text-[1.125rem] leading-[1.5]" : "text-[1.25rem] leading-[1.4]",
        expired ? "text-[#6f7078]" : "text-foreground",
      )}
    >
      {groups.map((group, i) => (
        <span key={i}>
          <span className="whitespace-nowrap">
            {group}
            {i < groups.length - 1 && "-"}
          </span>
          <wbr />
        </span>
      ))}
    </code>
  );
}

function CodeDialog({
  shown,
  narrow,
  onClose,
  onAgain,
}: {
  shown: Shown | null;
  narrow: boolean;
  onClose: () => void;
  onAgain: () => void;
}) {
  const { t } = useLanguage();
  const [copied, setCopied] = useState(false);
  const [now, setNow] = useState(Date.now());
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => {
    setCopied(false);
    setNow(Date.now());
    if (shown?.kind !== "device") return;
    const tick = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(tick);
  }, [shown]);
  useEffect(() => () => clearTimeout(timer.current), []);

  const copy = async () => {
    if (!shown) return;
    await navigator.clipboard.writeText(shown.code);
    setCopied(true);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setCopied(false), 1500);
  };

  const expired = shown?.kind === "device" && now >= shown.until;
  const minutes = shown?.kind === "device" ? Math.max(1, Math.ceil((shown.until - now) / 60_000)) : 0;
  const host = shown?.kind === "device" ? shown.instance.replace(/^https?:\/\//, "") : "";
  const button = "h-10.5 rounded-xl px-4 text-sm font-semibold";
  return (
    <Dialog open={shown !== null} onOpenChange={(open) => !open && onClose()}>
      <DialogContent
        showCloseButton={false}
        className={cn(
          "gap-0 rounded-[1.125rem] border border-border bg-popover p-0 shadow-[0_20px_48px_rgba(0,0,0,0.5)] ring-0 sm:max-w-120",
          narrow && "top-auto bottom-3 max-w-[calc(100%-1.5rem)] -translate-y-0 sm:max-w-100",
        )}
      >
        {shown && (
          <div className="flex flex-col gap-3.5 p-5">
            <DialogTitle className="text-lg font-semibold">
              {shown.kind === "recovery" ? t("code.recoveryTitle") : t("code.deviceTitle")}
            </DialogTitle>
            <Code code={shown.code} narrow={narrow} expired={expired} />
            {expired && <p className="text-[0.8125rem] text-warning">{t("code.expired")}</p>}
            {shown.kind === "device" && !expired && (
              <p className="flex items-center gap-1.5 text-[0.8125rem] text-subtle">
                <Clock aria-hidden="true" className="size-3.5" />
                {t("code.expires", { instance: host, minutes })}
              </p>
            )}
            {!expired && (
              <DialogDescription className="text-[0.8125rem] text-muted-foreground">
                {shown.kind === "recovery" ? t("code.recoveryWarn") : t("code.deviceWhere")}
              </DialogDescription>
            )}
            <div className="mt-1 flex justify-end gap-2">
              {expired ? (
                <Button variant="secondary" onClick={onAgain} className={button}>
                  {t("code.again")}
                </Button>
              ) : (
                <Button variant="secondary" onClick={copy} className={button}>
                  {copied ? t("code.copied") : t("settings.copy")}
                </Button>
              )}
              <Button onClick={onClose} className={button}>
                {t("code.done")}
              </Button>
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
