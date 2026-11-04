// The microphone, asked for when the person joins a room: a dialog while
// the browser's own prompt is open, and the way out when the browser
// refuses (the canvas's "Web · le micro demandé" and "Web · micro
// bloqué"). In a narrow window it sits at the bottom, as the invitation
// does. The join itself fails with "error.microphone", which the room's
// screen shows under its button.
import { useEffect, useState } from "react";
import { cn } from "cn";
import { Button } from "@/components/ui/button";
import { Dialog, DialogClose, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { CheckingIcon, MicIcon } from "@/components/icons";
import { useNarrow } from "@/hooks";
import { useLanguage } from "@/lang";
import { type MicState, onMicState, setMicState } from "../audio/engine";
import { webText } from "../strings";

export function MicDialog() {
  const { lang } = useLanguage();
  const narrow = useNarrow();
  const [state, setState] = useState<MicState>("idle");
  useEffect(() => onMicState(setState), []);
  const w = (key: Parameters<typeof webText>[1]) => webText(lang, key);
  const denied = state === "denied";

  return (
    <Dialog
      open={state !== "idle"}
      // The browser's prompt decides while asking: only a refusal closes.
      onOpenChange={(open) => !open && denied && setMicState("idle")}
    >
      <DialogContent
        showCloseButton={false}
        className={cn(
          "flex flex-col gap-3 rounded-[1.125rem] border border-border bg-popover p-5 shadow-[0_20px_48px_rgba(0,0,0,0.5)] ring-0 sm:max-w-100",
          narrow && "top-auto bottom-3 max-w-[calc(100%-1.5rem)] -translate-y-0",
        )}
      >
        <div className="flex items-center gap-2.5">
          <span
            className={cn(
              "flex size-9 shrink-0 items-center justify-center rounded-xl",
              denied ? "bg-alarm/16 text-danger" : "bg-primary/12 text-primary",
            )}
          >
            <MicIcon muted={denied} className="size-4.5" />
          </span>
          <DialogTitle className="text-lg font-semibold">{denied ? w("mic.denied") : w("mic.asking")}</DialogTitle>
        </div>
        {denied ? (
          <>
            <DialogDescription className="text-sm text-soft">{w("mic.deniedText")}</DialogDescription>
            <ol className="flex list-decimal flex-col gap-1 pl-5 text-[0.8125rem] text-subtle">
              <li>{w("mic.step1")}</li>
              <li>{w("mic.step2")}</li>
              <li>{w("mic.step3")}</li>
            </ol>
            <div className="mt-1 flex justify-end">
              <DialogClose
                render={
                  <Button
                    variant="secondary"
                    className={cn("rounded-xl px-4 text-sm font-semibold", narrow ? "h-12 w-full" : "h-10.5")}
                  />
                }
              >
                {w("mic.ok")}
              </DialogClose>
            </div>
          </>
        ) : (
          <>
            <DialogDescription className="text-sm text-soft">{w("mic.askingText")}</DialogDescription>
            <p className="text-[0.8125rem] text-muted-foreground">{w("mic.untouched")}</p>
            <span role="status" className="flex items-center gap-2 text-[0.8125rem] text-subtle">
              <CheckingIcon className="size-3.5 text-dim" />
              {w("mic.waiting")}
            </span>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
