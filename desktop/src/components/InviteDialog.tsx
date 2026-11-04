// Joining a space with the link a friend sent, and the name to show there;
// or, from the same dialog, getting one's account back with a code
// (ADR 0051).
import { useEffect, useState, type FormEvent } from "react";
import { cn } from "cn";
import { api, isAppError, platform } from "@/api";
import { Button } from "@/components/ui/button";
import { Dialog, DialogClose, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { invitationProblem, offerInvitation, offeredInvitation } from "@/invite";
import { errorText, useLanguage } from "@/lang";
import { NAME_KEY, type AppState } from "@/state";

export function InviteDialog({ app, narrow }: { app: AppState; narrow: boolean }) {
  const [restoring, setRestoring] = useState(false);
  useEffect(() => {
    if (app.inviteOpen) setRestoring(false);
  }, [app.inviteOpen]);
  return (
    <Dialog open={app.inviteOpen} onOpenChange={app.setInviteOpen}>
      <DialogContent
        showCloseButton={false}
        className={cn(
          "gap-0 rounded-[1.125rem] border border-border bg-popover p-0 shadow-[0_20px_48px_rgba(0,0,0,0.5)] ring-0 sm:max-w-100",
          narrow && "top-auto bottom-3 max-w-[calc(100%-1.5rem)] -translate-y-0",
        )}
      >
        {restoring ? (
          <RestoreForm app={app} onBack={() => setRestoring(false)} />
        ) : (
          <InviteForm app={app} onRestore={() => setRestoring(true)} />
        )}
      </DialogContent>
    </Dialog>
  );
}

const field = "h-11 rounded-xl border-input bg-background px-3 dark:bg-background";
const label = "flex-col items-stretch gap-1.5 text-[0.78125rem] leading-normal font-medium text-subtle";
const quiet = "h-10.5 rounded-xl px-3.5 text-sm font-medium text-soft";

function InviteForm({ app, onRestore }: { app: AppState; onRestore: () => void }) {
  const { t } = useLanguage();
  const [link, setLink] = useState("");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Each opening starts afresh: the link the window was opened with, if
  // any (forgotten once the form closes), and the last name typed.
  useEffect(() => {
    if (!app.inviteOpen) return offerInvitation(null);
    setLink(offeredInvitation() ?? "");
    setName(localStorage.getItem(NAME_KEY) ?? app.displayName);
    setError(null);
  }, [app.inviteOpen]);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const problem = invitationProblem(link, name);
    if (problem) {
      setError(problem === "invalid_link" ? t("error.invalid_link") : t("invite.nameMissing"));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const space = await api.acceptInvitation(link.trim(), name.trim());
      app.accepted(space, name.trim());
      app.setInviteOpen(false);
    } catch (e) {
      setError(errorText(t, e));
    } finally {
      setBusy(false);
    }
  };

  return (
        <form onSubmit={submit} className="flex flex-col gap-3.5 p-5">
          <DialogTitle className="text-lg font-semibold">{t("invite.title")}</DialogTitle>
          <Label className="flex-col items-stretch gap-1.5 text-[0.78125rem] leading-normal font-medium text-subtle">
            {t("invite.link")}
            <Input
              type="url"
              value={link}
              onChange={(e) => setLink(e.target.value)}
              placeholder={t("invite.linkPlaceholder")}
              autoComplete="off"
              spellCheck={false}
              autoFocus
              className={cn(field, "font-mono text-[0.8125rem] md:text-[0.8125rem]")}
            />
          </Label>
          <Label className="flex-col items-stretch gap-1.5 text-[0.78125rem] leading-normal font-medium text-subtle">
            {t("invite.name")}
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              maxLength={64}
              className={cn(field, "text-sm md:text-sm")}
            />
            <DialogDescription className="text-[0.78125rem] font-normal text-muted-foreground">
              {t("invite.nameHelp")}
            </DialogDescription>
          </Label>
          {error && (
            <p role="alert" className="text-[0.8125rem] text-danger">
              {error}
            </p>
          )}
          <div className="mt-1 flex items-center justify-end gap-2">
            <DialogClose render={<Button variant="ghost" className={quiet} />}>{t("invite.cancel")}</DialogClose>
            <Button type="submit" disabled={busy} className="h-10.5 rounded-xl px-4 text-sm font-semibold">
              {busy ? t("invite.busy") : t("invite.submit")}
            </Button>
          </div>
          <Button type="button" variant="ghost" onClick={onRestore} className={cn(quiet, "self-center")}>
            {t("restore.open")}
          </Button>
        </form>
  );
}

/** "Get my account back": a recovery code, or a device code, on the
    instance it was made on. The web client is that instance: it asks for
    the code alone. */
function RestoreForm({ app, onBack }: { app: AppState; onBack: () => void }) {
  const { t } = useLanguage();
  const [instance, setInstance] = useState(platform === "web" ? location.origin : "");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // How many spaces the account this device has would lose: set once the
  // app refused to replace it, so that the next click does.
  const [replacing, setReplacing] = useState<number | null>(null);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.restoreAccount(instance.trim(), code.trim(), replacing !== null);
      app.reloadSpaces();
      app.setInviteOpen(false);
    } catch (e) {
      if (isAppError(e) && e.code === "account_exists") {
        const account = await api.account().catch(() => null);
        setReplacing(account?.spaces ?? 1);
      } else if (isAppError(e) && (e.code === "bad_code" || e.code === "code_unknown")) {
        // The sentence says it all; the server's English adds nothing.
        setError(t(`error.${e.code}`));
      } else {
        setError(errorText(t, e));
      }
    } finally {
      setBusy(false);
    }
  };

  return (
    <form onSubmit={submit} className="flex flex-col gap-3.5 p-5">
      <DialogTitle className="text-lg font-semibold">{t("restore.title")}</DialogTitle>
      <DialogDescription className="text-[0.8125rem] text-muted-foreground">{t("restore.help")}</DialogDescription>
      {platform !== "web" && (
        <Label className={label}>
          {t("restore.instance")}
          <Input
            type="url"
            value={instance}
            onChange={(e) => setInstance(e.target.value)}
            placeholder={t("restore.instancePlaceholder")}
            autoComplete="off"
            spellCheck={false}
            className={cn(field, "font-mono text-[0.8125rem] md:text-[0.8125rem]")}
          />
        </Label>
      )}
      <Label className={label}>
        {t("restore.code")}
        <Input
          value={code}
          onChange={(e) => {
            setCode(e.target.value);
            setReplacing(null);
          }}
          placeholder={t("restore.codePlaceholder")}
          autoComplete="off"
          autoCapitalize="characters"
          spellCheck={false}
          autoFocus
          aria-invalid={error !== null}
          className={cn(
            field,
            "font-mono text-[0.9375rem] tracking-[0.04em] md:text-[0.9375rem]",
            error !== null && "border-destructive",
          )}
        />
      </Label>
      {replacing !== null && (
        <p role="alert" className="rounded-xl border border-warning/40 bg-warning/10 px-3.5 py-3 text-[0.8125rem] text-warning">
          {t("restore.replace", { count: replacing })}
        </p>
      )}
      {error && (
        <p role="alert" className="text-[0.8125rem] text-danger">
          {error}
        </p>
      )}
      <div className="mt-1 flex items-center justify-end gap-2">
        <DialogClose render={<Button variant="ghost" className={quiet} />}>{t("invite.cancel")}</DialogClose>
        <Button
          type="submit"
          // Replacing this device's account is said in red, as leaving is.
          variant={replacing !== null ? "leave" : "default"}
          disabled={busy || code.trim() === "" || instance.trim() === ""}
          className="h-10.5 rounded-xl px-4 text-sm font-semibold"
        >
          {busy ? t("restore.busy") : replacing !== null ? t("restore.replaceSubmit") : t("restore.submit")}
        </Button>
      </div>
      <Button type="button" variant="ghost" onClick={onBack} className={cn(quiet, "self-center")}>
        {t("restore.back")}
      </Button>
    </form>
  );
}
