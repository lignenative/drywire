// The invitation form's two fields, checked before they cross into the
// Rust side (ADR 0037). The link check mirrors the start of
// Invitation::parse in drywire-proto: the Rust side still decodes it and
// has the last word.
import { z } from "zod";

/** The two prefixes an invitation link starts with, then base64url. */
const LINK = /^(https:\/\/drywire\.gg\/i#|drywire:\/\/i#)[A-Za-z0-9_-]+=*$/;

export const invitationForm = z.object({
  link: z.string().trim().regex(LINK, { message: "invalid_link" }),
  name: z.string().trim().min(1, { message: "name_missing" }).max(64),
});

export type InvitationForm = z.infer<typeof invitationForm>;

/** Which field is wrong, if any: the link first, as the form lists it. */
export function invitationProblem(link: string, name: string): "invalid_link" | "name_missing" | null {
  const result = invitationForm.safeParse({ link, name });
  if (result.success) return null;
  return result.error.issues.some((i) => i.path[0] === "link") ? "invalid_link" : "name_missing";
}

/** An invitation link the window was opened with, offered to the
    invitation form: the web client's address carries one in its fragment
    (apps/web). The form opens with it filled in, until it closes. */
let offered: string | null = null;

export function offerInvitation(link: string | null): void {
  offered = link;
}

export function offeredInvitation(): string | null {
  return offered;
}

/** A creation code the window was opened with (ADR 0054): the web client's
    address carries one as `#create=<code>`, which `make creator-code`
    prints. The form opens on "Create a space" with it filled in. */
let offeredCode: string | null = null;

export function offerCreation(code: string | null): void {
  offeredCode = code;
}

export function offeredCreation(): string | null {
  return offeredCode;
}
