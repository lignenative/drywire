// Joining a space with the link a friend sent, and the name to show there.
import { useEffect, useState, type FormEvent } from "react";
import { cn } from "cn";
import { api } from "@/api";
import { Button } from "@/components/ui/button";
import { Dialog, DialogClose, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { invitationProblem } from "@/invite";
import { errorText, useLanguage } from "@/lang";
import { NAME_KEY, type AppState } from "@/state";

export function InviteDialog({ app, narrow }: { app: AppState; narrow: boolean }) {
  const { t } = useLanguage();
  const [link, setLink] = useState("");
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Each opening starts afresh, with the last name typed.
  useEffect(() => {
    if (!app.inviteOpen) return;
    setLink("");
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

  const field = "h-11 rounded-xl border-input bg-background px-3 dark:bg-background";
  return (
    <Dialog open={app.inviteOpen} onOpenChange={app.setInviteOpen}>
      <DialogContent
        showCloseButton={false}
        className={cn(
          "gap-0 rounded-[18px] border border-border bg-popover p-0 shadow-[0_20px_48px_rgba(0,0,0,0.5)] ring-0 sm:max-w-[400px]",
          narrow && "top-auto bottom-3 max-w-[calc(100%-24px)] -translate-y-0",
        )}
      >
        <form onSubmit={submit} className="flex flex-col gap-3.5 p-5">
          <DialogTitle className="text-lg font-semibold">{t("invite.title")}</DialogTitle>
          <Label className="flex-col items-stretch gap-1.5 text-[12.5px] leading-normal font-medium text-subtle">
            {t("invite.link")}
            <Input
              type="url"
              value={link}
              onChange={(e) => setLink(e.target.value)}
              placeholder={t("invite.linkPlaceholder")}
              autoComplete="off"
              spellCheck={false}
              autoFocus
              className={cn(field, "font-mono text-[13px] md:text-[13px]")}
            />
          </Label>
          <Label className="flex-col items-stretch gap-1.5 text-[12.5px] leading-normal font-medium text-subtle">
            {t("invite.name")}
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              maxLength={64}
              className={cn(field, "text-sm md:text-sm")}
            />
            <DialogDescription className="text-[12.5px] font-normal text-muted-foreground">
              {t("invite.nameHelp")}
            </DialogDescription>
          </Label>
          {error && (
            <p role="alert" className="text-[13px] text-danger">
              {error}
            </p>
          )}
          <div className="mt-1 flex justify-end gap-2">
            <DialogClose
              render={
                <Button variant="ghost" className="h-[42px] rounded-xl px-3.5 text-sm font-medium text-soft" />
              }
            >
              {t("invite.cancel")}
            </DialogClose>
            <Button type="submit" disabled={busy} className="h-[42px] rounded-xl px-4 text-sm font-semibold">
              {busy ? t("invite.busy") : t("invite.submit")}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
