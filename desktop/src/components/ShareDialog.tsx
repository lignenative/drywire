// Inviting people into a space (ADR 0054): the person-plus button on a
// space's header, for its owner and admins, and the dialog that makes a
// link for 10 people and 7 days. The link holds the space's key: it is
// shown and copied, never stored or logged.
import { useEffect, useRef, useState } from "react";
import { cn } from "cn";
import { api, isAppError, type InvitationView, type SpaceView } from "@/api";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { errorText, useLanguage } from "@/lang";
import type { AppState } from "@/state";
import { PersonPlusIcon, SpinIcon } from "./icons";

/** Whether this device may invite into a space. */
export function canInvite(space: SpaceView): boolean {
  return !space.error && (space.role === "owner" || space.role === "admin");
}

/** The quiet person-plus button: always shown, since touch has no hover. */
export function InviteButton({ app, space, narrow }: { app: AppState; space: SpaceView; narrow: boolean }) {
  const { t } = useLanguage();
  return (
    <Button
      variant="ghost"
      aria-label={t("share.button", { space: space.name })}
      title={t("share.button", { space: space.name })}
      onClick={() => app.setSharing(space)}
      className={cn(
        "shrink-0 p-0 text-muted-foreground hover:bg-raised hover:text-foreground focus-visible:bg-raised focus-visible:text-foreground",
        narrow ? "size-11 rounded-xl [&_svg]:size-5" : "size-7 rounded-lg [&_svg]:size-4",
      )}
    >
      <PersonPlusIcon />
    </Button>
  );
}

type State = { kind: "loading" } | { kind: "ready"; invitation: InvitationView } | { kind: "error"; text: string };

export function ShareDialog({ app, narrow }: { app: AppState; narrow: boolean }) {
  const { t } = useLanguage();
  const space = app.sharing;
  const [state, setState] = useState<State>({ kind: "loading" });
  const [copied, setCopied] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  // A new link each time the dialog opens: a link shown once and closed is
  // not kept anywhere.
  useEffect(() => {
    if (!space || space.group) return;
    let current = true;
    setState({ kind: "loading" });
    setCopied(false);
    api.createInvitation(space.instance, space.id).then(
      (invitation) => current && setState({ kind: "ready", invitation }),
      (e) =>
        current &&
        setState({
          kind: "error",
          text: isAppError(e) && e.code === "refused" ? t("share.denied") : errorText(t, e),
        }),
    );
    return () => {
      current = false;
    };
  }, [space]);
  useEffect(() => () => clearTimeout(timer.current), []);

  const copy = async () => {
    if (state.kind !== "ready") return;
    await navigator.clipboard.writeText(state.invitation.link);
    setCopied(true);
    clearTimeout(timer.current);
    timer.current = setTimeout(() => setCopied(false), 1500);
  };

  const days =
    state.kind === "ready"
      ? Math.max(1, Math.round((Date.parse(state.invitation.expiresAt) - Date.now()) / 86_400_000))
      : 7;
  const uses = state.kind === "ready" ? state.invitation.uses : 10;
  const box = "rounded-xl border border-[#2f3036] bg-[#0f1012] px-3.5 py-3";
  const button = "h-10.5 rounded-xl px-4 text-sm font-semibold";
  return (
    <Dialog open={space !== null} onOpenChange={(open) => !open && app.setSharing(null)}>
      <DialogContent
        showCloseButton={false}
        className={cn(
          "gap-0 rounded-[1.125rem] border border-border bg-popover p-0 shadow-[0_20px_48px_rgba(0,0,0,0.5)] ring-0 sm:max-w-120",
          narrow && "top-auto bottom-3 max-w-[calc(100%-1.5rem)] -translate-y-0 sm:max-w-100",
        )}
      >
        {space?.group && <InviteByName space={space} onDone={() => app.setSharing(null)} />}
        {space && !space.group && (
          <div className="flex flex-col gap-3.5 p-5">
            <DialogTitle className="text-lg font-semibold">{t("share.title", { space: space.name })}</DialogTitle>
            {state.kind === "error" ? (
              <p
                role="alert"
                className="rounded-xl border border-[rgba(242,85,90,0.35)] bg-[rgba(242,85,90,0.10)] px-3.5 py-3 text-[0.8125rem] text-[#ff8a8e]"
              >
                {state.text}
              </p>
            ) : state.kind === "loading" ? (
              <p role="status" className={cn(box, "flex min-h-16.5 items-center gap-2.5 text-[0.84375rem] text-subtle")}>
                <SpinIcon className="size-4" />
                {t("share.loading")}
              </p>
            ) : (
              <code
                className={cn(
                  box,
                  "font-mono text-[0.8125rem] leading-[1.55] break-all text-foreground select-all",
                )}
              >
                {state.invitation.link}
              </code>
            )}
            {state.kind !== "error" && (
              <DialogDescription className="text-[0.8125rem] text-muted-foreground">
                {t("share.help", { uses, days })}
              </DialogDescription>
            )}
            <div className="mt-1 flex justify-end gap-2">
              {state.kind !== "error" && (
                <Button
                  variant="secondary"
                  disabled={state.kind !== "ready"}
                  onClick={copy}
                  className={cn(button, "disabled:pointer-events-auto disabled:opacity-40")}
                >
                  {copied ? t("share.copied") : t("share.copy")}
                </Button>
              )}
              <Button onClick={() => app.setSharing(null)} className={button}>
                {t("code.done")}
              </Button>
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

/** A space with an MLS group invites by user name (ADR 0055): the
    invitation waits in that person's app, and their device is let into
    the group by any member's app online. */
function InviteByName({ space, onDone }: { space: SpaceView; onDone: () => void }) {
  const { t } = useLanguage();
  const [username, setUsername] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const button = "h-10.5 rounded-xl px-4 text-sm font-semibold";

  const invite = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setResult(null);
    try {
      const name = await api.inviteUser(space.instance, space.id, username.trim());
      setResult({ ok: true, text: t("invite.sent", { name }) });
      setUsername("");
    } catch (e) {
      const code = isAppError(e) ? e.code : "";
      const text =
        code === "not_found"
          ? t("invite.unknown")
          : isAppError(e) && /409/.test(e.message)
            ? t("invite.member")
            : code === "refused"
              ? t("share.denied")
              : errorText(t, e);
      setResult({ ok: false, text });
    } finally {
      setBusy(false);
    }
  };

  return (
    <form onSubmit={invite} className="flex flex-col gap-3.5 p-5">
      <DialogTitle className="text-lg font-semibold">{t("share.title", { space: space.name })}</DialogTitle>
      <label className="flex flex-col gap-1.5 text-[0.78125rem] font-medium text-subtle">
        {t("invite.byName")}
        <span className="flex gap-2">
          <input
            type="search"
            value={username}
            onChange={(e) => setUsername(e.target.value.toLowerCase())}
            autoCapitalize="none"
            spellCheck={false}
            maxLength={32}
            autoFocus
            className="h-11 min-w-0 flex-1 rounded-xl border border-input bg-background px-3 text-base text-foreground outline-none focus-visible:border-ring md:text-sm"
          />
          <Button type="submit" disabled={busy || username.trim().length < 2} className={cn(button, "disabled:pointer-events-auto disabled:opacity-40")}>
            {t("invite.send")}
          </Button>
        </span>
      </label>
      {result && (
        <p role={result.ok ? "status" : "alert"} className={cn("text-[0.8125rem]", result.ok ? "text-primary" : "text-danger")}>
          {result.text}
        </p>
      )}
      <DialogDescription className="text-[0.8125rem] text-muted-foreground">{t("onboard.nameLead")}</DialogDescription>
      <div className="mt-1 flex justify-end">
        <Button type="button" variant="secondary" onClick={onDone} className={button}>
          {t("code.done")}
        </Button>
      </div>
    </form>
  );
}
