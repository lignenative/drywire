// The web client's own words, for the screens the desktop app does not
// have (the design canvas's "Navigateur · client web" row). Everything
// else comes from the shared interface's text (desktop/src/i18n.ts).
import type { Lang } from "@/i18n";

const en = {
  "unsupported.title": "This browser cannot run Drywire",
  "unsupported.missing": "It lacks what carries your voice live:",
  "unsupported.where": "Open the link in Chrome or Edge, Firefox on a computer, or Safari 26.4 or later.",
  "unsupported.app": "The Drywire app needs none of this, and adds less delay than a browser.",
  "mic.asking": "Allow the microphone",
  "mic.askingText": "Your browser is asking, at the top of the window. Drywire only opens it during a call.",
  "mic.untouched": "Your voice goes out as it is: no noise reduction, no echo cancellation, no automatic gain.",
  "mic.waiting": "Waiting for your answer…",
  "mic.denied": "Microphone blocked",
  "mic.deniedText": "The browser refuses this page the microphone. Without it, you cannot join a room.",
  "mic.step1": "Click the icon left of the address.",
  "mic.step2": "Allow the microphone for this site.",
  "mic.step3": "Join the room again.",
  "mic.ok": "Got it",
};

const fr: Record<keyof typeof en, string> = {
  "unsupported.title": "Ce navigateur ne peut pas faire tourner Drywire",
  "unsupported.missing": "Il lui manque de quoi faire passer votre voix en direct :",
  "unsupported.where": "Ouvrez le lien dans Chrome ou Edge, Firefox sur ordinateur, ou Safari 26.4 et plus récent.",
  "unsupported.app": "L’application Drywire, elle, n’a besoin de rien de tout ça, et ajoute moins de délai qu’un navigateur.",
  "mic.asking": "Autorisez le micro",
  "mic.askingText": "Votre navigateur vous le demande, en haut de la fenêtre. Drywire ne l’ouvre que pendant un appel.",
  "mic.untouched": "Votre voix part telle quelle : ni réduction de bruit, ni annulation d’écho, ni gain automatique.",
  "mic.waiting": "En attente de votre réponse…",
  "mic.denied": "Micro bloqué",
  "mic.deniedText": "Le navigateur refuse le micro à cette page. Sans lui, vous ne pouvez pas rejoindre un salon.",
  "mic.step1": "Cliquez sur l’icône à gauche de l’adresse.",
  "mic.step2": "Autorisez le micro pour ce site.",
  "mic.step3": "Rejoignez le salon de nouveau.",
  "mic.ok": "Compris",
};

export type WebKey = keyof typeof en;

export function webText(lang: Lang, key: WebKey): string {
  return (lang === "fr" ? fr : en)[key];
}
