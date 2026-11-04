// Signing up and signing in by e-mail (ADR 0055, the canvas's V4 page):
// an address, a 6-digit code, then a user name for a new account. Then,
// with no space yet, the home: invitations waiting, create a space, or a
// link. Nobody sees a key.
import { type FormEvent, useEffect, useRef, useState } from "react";
import { cn } from "cn";
import { api, isAppError, type PendingInvitationView } from "@/api";
import { codeDigits, type SignedIn } from "@/account";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { errorText, useLanguage } from "@/lang";
import type { AppState } from "@/state";
import { Logo, SettingsIcon } from "./icons";

const field = "h-12 rounded-xl border-input bg-background px-3.5 text-base md:text-base dark:bg-background pointer-coarse:h-13";
const label = "flex-col items-stretch gap-1.5 text-[0.78125rem] leading-normal font-medium text-subtle";
const primary = "h-12 w-full rounded-[0.875rem] text-[0.9375rem] font-semibold pointer-coarse:h-13";

/** The frame every step shares: settings at the top, one column. */
function Frame({
  app,
  narrow,
  back,
  top,
  children,
}: {
  app: AppState;
  narrow: boolean;
  back?: { label: string; onClick: () => void };
  /** Keep the column high, above a phone's keyboard. */
  top?: boolean;
  children: React.ReactNode;
}) {
  const { t } = useLanguage();
  return (
    <div className={cn("flex h-full flex-col", narrow && "pt-1 pb-3")}>
      <header className={cn("flex items-center gap-2", narrow ? "px-2 pt-1" : "px-4 pt-3.5")}>
        {back && (
          <Button variant="ghost" onClick={back.onClick} className="h-10 gap-1.5 rounded-xl px-2.5 text-sm text-subtle">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" className="size-4.5" aria-hidden>
              <path d="M15 18l-6-6 6-6" />
            </svg>
            {back.label}
          </Button>
        )}
        <span className="flex-1" />
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
          "flex min-h-0 flex-1 flex-col items-center overflow-y-auto",
          top ? "justify-center pointer-coarse:justify-start pointer-coarse:pt-6" : "justify-center pointer-coarse:justify-end",
          narrow ? "px-5 pb-6 pointer-coarse:pb-4" : "px-14 pb-18",
        )}
      >
        <div className={cn("flex w-full flex-col gap-3.5", narrow ? "max-w-85" : "max-w-95")}>{children}</div>
      </main>
    </div>
  );
}

function Title({ narrow, children }: { narrow: boolean; children: React.ReactNode }) {
  return (
    <h1
      className={cn(
        "leading-[1.1] font-semibold tracking-[-0.03em]",
        narrow ? "text-[1.625rem] pointer-coarse:text-[1.875rem]" : "text-[2.125rem]",
      )}
    >
      {children}
    </h1>
  );
}

function Lead({ narrow, children }: { narrow: boolean; children: React.ReactNode }) {
  return (
    <p className={cn("text-soft", narrow ? "text-base pointer-coarse:text-[1.0625rem]" : "text-[1.0625rem]")}>{children}</p>
  );
}

function Alert({ children }: { children: React.ReactNode }) {
  return (
    <p
      role="alert"
      className="rounded-xl border border-[rgba(242,85,90,0.35)] bg-[rgba(242,85,90,0.10)] px-3.5 py-2.5 text-[0.8125rem] text-[#ff8a8e]"
    >
      {children}
    </p>
  );
}

type Step = { kind: "email" } | { kind: "code"; email: string } | { kind: "username" };

/** The sign-up and sign-in steps, until the account has a user name. */
export function Onboarding({
  app,
  narrow,
  pending,
  onSignedIn,
}: {
  app: AppState;
  narrow: boolean;
  /** An account signed in without a user name yet. */
  pending: SignedIn | null;
  onSignedIn: (account: SignedIn) => void;
}) {
  const { t, lang } = useLanguage();
  const [instance, setInstance] = useState(pending?.instance ?? "");
  const [askInstance, setAskInstance] = useState(false);
  const [step, setStep] = useState<Step>(pending ? { kind: "username" } : { kind: "email" });
  const [email, setEmail] = useState("");
  const [code, setCode] = useState("");
  const [username, setUsername] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [suggestions, setSuggestions] = useState<string[]>([]);
  const [resendAt, setResendAt] = useState(0);
  const [now, setNow] = useState(Date.now());

  useEffect(() => {
    if (instance) return;
    api.defaultInstance().then(
      (found) => (found ? setInstance(found) : setAskInstance(true)),
      () => setAskInstance(true),
    );
  }, []);
  useEffect(() => {
    if (step.kind !== "code") return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [step.kind]);

  const sendCode = async (to: string) => {
    await api.emailCode(instance.trim(), to, lang);
    setResendAt(Date.now() + 30_000);
  };

  const submitEmail = async (event: FormEvent) => {
    event.preventDefault();
    if (!email.includes("@") || !instance.trim()) return;
    setBusy(true);
    setError(null);
    try {
      await sendCode(email.trim());
      setCode("");
      setStep({ kind: "code", email: email.trim() });
    } catch (e) {
      setError(errorText(t, e));
    } finally {
      setBusy(false);
    }
  };

  const submitCode = async (typed: string) => {
    if (step.kind !== "code" || typed.length !== 6 || busy) return;
    setBusy(true);
    setError(null);
    try {
      const signed = await api.emailSignIn(instance.trim(), step.email, typed);
      if (signed.username) {
        onSignedIn({ instance: instance.trim(), username: signed.username });
      } else {
        onSignedIn({ instance: instance.trim(), username: "" });
        setStep({ kind: "username" });
      }
    } catch (e) {
      if (isAppError(e) && e.code === "wrong_code") {
        const left = Number(/\d+/.exec(e.message)?.[0] ?? 0);
        setError(left > 0 ? t("onboard.wrongCode", { n: left }) : t("onboard.codeSpent"));
      } else {
        setError(errorText(t, e));
      }
      setCode("");
    } finally {
      setBusy(false);
    }
  };

  const submitUsername = async (event: FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    setSuggestions([]);
    try {
      const name = await api.setUsername(instance.trim(), username.trim(), displayName.trim());
      onSignedIn({ instance: instance.trim(), username: name });
      app.reloadSpaces();
    } catch (e) {
      if (isAppError(e) && e.code === "username_taken") {
        setSuggestions(e.message.split(",").filter(Boolean));
      } else {
        setError(errorText(t, e));
      }
    } finally {
      setBusy(false);
    }
  };

  if (step.kind === "code") {
    const wait = Math.max(0, Math.ceil((resendAt - now) / 1000));
    return (
      <Frame app={app} narrow={narrow} top back={{ label: t("onboard.changeEmail"), onClick: () => setStep({ kind: "email" }) }}>
        <Title narrow={narrow}>{t("onboard.checkTitle")}</Title>
        <Lead narrow={narrow}>{t("onboard.checkLead", { email: step.email })}</Lead>
        <CodeField
          value={code}
          invalid={error !== null}
          onChange={(typed) => {
            setCode(typed);
            if (typed.length === 6) void submitCode(typed);
          }}
        />
        {error && <Alert>{error}</Alert>}
        <Button
          onClick={() => void submitCode(code)}
          disabled={code.length !== 6 || busy}
          className={cn(primary, "mt-1 disabled:pointer-events-auto disabled:opacity-40")}
        >
          {t("onboard.continue")}
        </Button>
        <p className="text-[0.8125rem] text-muted-foreground">
          {t("onboard.noMail")}{" "}
          {wait > 0 ? (
            t("onboard.resendIn", { s: wait })
          ) : (
            <button
              type="button"
              className="text-soft underline"
              onClick={() => sendCode(step.email).catch((e) => setError(errorText(t, e)))}
            >
              {t("onboard.resend")}
            </button>
          )}
        </p>
      </Frame>
    );
  }

  if (step.kind === "username") {
    return (
      <Frame app={app} narrow={narrow} top>
        <Title narrow={narrow}>{t("onboard.nameTitle")}</Title>
        <Lead narrow={narrow}>{t("onboard.nameLead")}</Lead>
        <form onSubmit={submitUsername} className="mt-2.5 flex flex-col gap-3.5">
          <Label className={label}>
            {t("onboard.username")}
            <Input
              value={username}
              onChange={(e) => setUsername(e.target.value.toLowerCase())}
              autoComplete="username"
              autoCapitalize="none"
              spellCheck={false}
              maxLength={32}
              autoFocus
              aria-invalid={suggestions.length > 0 || undefined}
              className={field}
            />
            {suggestions.length > 0 && (
              <span role="alert" className="font-normal text-[#ff8a8e]">
                {t("onboard.taken")}{" "}
                {suggestions.map((s, i) => (
                  <span key={s}>
                    {i > 0 && ", "}
                    <button type="button" className="text-foreground underline" onClick={() => setUsername(s)}>
                      {s}
                    </button>
                  </span>
                ))}
              </span>
            )}
            <span className="font-normal text-muted-foreground">{t("onboard.usernameHelp")}</span>
          </Label>
          <Label className={label}>
            {t("onboard.displayName")}
            <Input value={displayName} onChange={(e) => setDisplayName(e.target.value)} autoComplete="nickname" maxLength={32} className={field} />
            <span className="font-normal text-muted-foreground">{t("onboard.displayHelp")}</span>
          </Label>
          {error && <Alert>{error}</Alert>}
          <Button type="submit" disabled={busy || username.trim().length < 2} className={cn(primary, "disabled:pointer-events-auto disabled:opacity-40")}>
            {t("onboard.createAccount")}
          </Button>
        </form>
      </Frame>
    );
  }

  return (
    <Frame app={app} narrow={narrow} top>
      <Logo size={narrow ? 40 : 44} />
      <Title narrow={narrow}>{t("welcome.title")}</Title>
      <Lead narrow={narrow}>{t("welcome.lead")}</Lead>
      <form onSubmit={submitEmail} className="mt-3 flex flex-col gap-2.5">
        {askInstance && (
          <Label className={label}>
            {t("onboard.instance")}
            <Input type="url" value={instance} onChange={(e) => setInstance(e.target.value)} placeholder="https://voice.example.org" className={field} />
          </Label>
        )}
        <Label className={label}>
          {t("onboard.email")}
          <Input
            type="email"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            placeholder={t("onboard.emailPlaceholder")}
            autoComplete="email"
            autoFocus
            className={field}
          />
        </Label>
        {error && <Alert>{error}</Alert>}
        <Button type="submit" disabled={busy || !email.includes("@") || !instance.trim()} className={cn(primary, "disabled:pointer-events-auto disabled:opacity-40")}>
          {t("onboard.continue")}
        </Button>
      </form>
      <p className="text-[0.8125rem] text-subtle">{t("onboard.emailHelp")}</p>
      <p className="mt-1.5 text-xs text-muted-foreground">{t("onboard.terms")}</p>
    </Frame>
  );
}

/** One input the system can fill from Mail or Messages
    (autocomplete="one-time-code"), drawn as six boxes: six separate
    fields would break that autofill. */
function CodeField({ value, invalid, onChange }: { value: string; invalid: boolean; onChange: (digits: string) => void }) {
  const { t } = useLanguage();
  const input = useRef<HTMLInputElement>(null);
  const [focused, setFocused] = useState(false);
  return (
    <label className={cn(label, "mt-2.5 flex")}>
      {t("onboard.code")}
      <span className="relative flex gap-2 pointer-coarse:gap-1.5" onClick={() => input.current?.focus()}>
        {[0, 1, 2, 3, 4, 5].map((i) => (
          <span
            key={i}
            aria-hidden
            className={cn(
              "flex h-14 flex-1 items-center justify-center rounded-xl border bg-background font-mono text-2xl font-medium text-foreground",
              invalid ? "border-danger" : focused && i === Math.min(value.length, 5) ? "border-primary" : "border-input",
            )}
          >
            {value[i] ?? ""}
          </span>
        ))}
        <input
          ref={input}
          value={value}
          onChange={(e) => onChange(codeDigits(e.target.value))}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          autoComplete="one-time-code"
          inputMode="numeric"
          pattern="[0-9]*"
          maxLength={9}
          autoFocus
          aria-label={t("onboard.code")}
          className="absolute inset-0 h-full w-full cursor-text opacity-0"
        />
      </span>
    </label>
  );
}

/** Signed in, with no space yet: invitations by user name wait here. */
export function SignedInHome({ app, narrow, account }: { app: AppState; narrow: boolean; account: SignedIn }) {
  const { t } = useLanguage();
  const [waiting, setWaiting] = useState<PendingInvitationView[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);

  useEffect(() => {
    let current = true;
    const load = () =>
      api.pendingInvitations(account.instance).then(
        (list) => current && setWaiting(list),
        () => {},
      );
    void load();
    // Someone may invite this account while the window shows this.
    const timer = setInterval(load, 15_000);
    return () => {
      current = false;
      clearInterval(timer);
    };
  }, [account.instance]);

  const answer = async (invitation: PendingInvitationView, accept: boolean) => {
    setError(null);
    try {
      await api.answerInvitation(invitation.instance, invitation.spaceId, accept);
      setWaiting((list) => list.filter((i) => i.spaceId !== invitation.spaceId));
      if (accept) app.reloadSpaces();
    } catch (e) {
      setError(errorText(t, e));
    }
  };

  return (
    <Frame app={app} narrow={narrow}>
      <Title narrow={narrow}>{t("home.hello", { name: account.username })}</Title>
      {waiting.map((invitation) => (
        <section key={invitation.spaceId} aria-label={t("home.invitation")} className="mt-1.5 flex flex-col gap-3 rounded-2xl bg-card p-4">
          <span className="text-[0.8125rem] font-medium text-subtle">{t("home.invitation")}</span>
          <span className="text-[1.0625rem] font-semibold tracking-[-0.02em]">
            {t("home.invitedBy", { who: invitation.invitedBy || "?", space: invitation.spaceName })}
          </span>
          <span className="flex gap-2">
            <Button onClick={() => answer(invitation, true)} className="h-11 flex-1 rounded-xl text-[0.9375rem] font-semibold">
              {t("home.join")}
            </Button>
            <Button variant="secondary" onClick={() => answer(invitation, false)} className="h-11 rounded-xl px-4 text-[0.9375rem]">
              {t("home.decline")}
            </Button>
          </span>
        </section>
      ))}
      {error && <Alert>{error}</Alert>}
      <div className="mt-2 flex flex-col gap-2">
        <Button variant="secondary" onClick={() => setCreating(true)} className={primary}>
          {t("home.create")}
        </Button>
        <Button variant="ghost" onClick={() => app.openInvite("invite")} className={cn(primary, "font-medium text-soft")}>
          {t("home.link")}
        </Button>
      </div>
      <p className="text-center text-[0.8125rem] text-muted-foreground">
        {t("home.yourName")} <span className="font-medium text-soft">{account.username}</span>
      </p>
      <CreateSpaceDialog app={app} narrow={narrow} account={account} open={creating} onClose={() => setCreating(false)} />
    </Frame>
  );
}

/** Creating a space takes a name, nothing else (ADR 0055). */
export function CreateSpaceDialog({
  app,
  narrow,
  account,
  open,
  onClose,
}: {
  app: AppState;
  narrow: boolean;
  account: SignedIn;
  open: boolean;
  onClose: () => void;
}) {
  const { t } = useLanguage();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (open) {
      setName("");
      setError(null);
    }
  }, [open]);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.createSpace(account.instance, null, name.trim(), account.username);
      app.reloadSpaces();
      onClose();
    } catch (e) {
      setError(errorText(t, e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent
        showCloseButton={false}
        className={cn(
          "gap-0 rounded-[1.125rem] border border-border bg-popover p-0 shadow-[0_20px_48px_rgba(0,0,0,0.5)] ring-0 sm:max-w-110",
          narrow && "top-auto bottom-3 max-w-[calc(100%-1.5rem)] -translate-y-0",
        )}
      >
        <form onSubmit={submit} className="flex flex-col gap-3.5 p-5">
          <DialogTitle className="text-lg font-semibold">{t("create.title")}</DialogTitle>
          <Label className={label}>
            {t("create.name")}
            <Input value={name} onChange={(e) => setName(e.target.value)} placeholder={t("create.namePlaceholder")} maxLength={32} autoFocus className="h-11 rounded-xl bg-background px-3 text-sm md:text-sm dark:bg-background" />
          </Label>
          <DialogDescription className="text-[0.8125rem] text-muted-foreground">{t("create.simpleHelp")}</DialogDescription>
          {error && <Alert>{error}</Alert>}
          <div className="mt-1 flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={onClose} className="h-10.5 rounded-xl px-3.5 text-sm font-medium text-soft">
              {t("invite.cancel")}
            </Button>
            <Button type="submit" disabled={busy || !name.trim()} className="h-10.5 rounded-xl px-4 text-sm font-semibold disabled:pointer-events-auto disabled:opacity-40">
              {busy ? t("create.busy") : t("create.submit")}
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
