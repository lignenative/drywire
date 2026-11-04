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
