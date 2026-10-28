// The interface's text, in English and French (spec section 10). Every
// key exists in both languages: the type of `fr` makes a missing one a
// compile error, and i18n.test.ts checks the placeholders match.
//
// Placeholders are written {name} and filled by `translate`.

const en = {
  "app.tagline": "Voice for teams who play together",
  "language.label": "Language",

  "identity.title": "This device",
  "identity.help":
    "Your device key. An operator lists it to let you create spaces on an instance.",
  "identity.copy": "Copy",
  "identity.copied": "Copied",

  "accept.title": "Join with an invitation",
  "accept.link": "Invitation link",
  "accept.linkPlaceholder": "https://…/i/#…",
  "accept.name": "Your name",
  "accept.namePlaceholder": "alex",
  "accept.submit": "Join the space",
  "accept.busy": "Joining…",
  "accept.done": "You are now a member of {space}.",

  "spaces.title": "Spaces",
  "spaces.refresh": "Refresh",
  "spaces.loading": "Loading…",
  "spaces.empty": "No space yet. Paste an invitation link below.",
  "spaces.unreachable": "Cannot list the rooms: {reason}",
  "spaces.join": "Join",
  "spaces.joining": "Joining…",

  "call.leave": "Leave",
  "call.connecting": "Connecting to the relay…",
  "call.ended": "The call ended: {reason}",
  "call.rejoin": "Join again",
  "call.close": "Close",
  "call.you": "you",
  "call.speaking": "speaking",
  "call.ping": "Ping",
  "call.devices": "Audio devices",
  "call.input": "Microphone",
  "call.output": "Output",
  "call.buffer": "buffer {buffer}, reported latency {latency}",
  "call.bluetooth": "Bluetooth: expect far more latency than shown.",
  "call.mute": "Mute",
  "call.unmute": "Unmute",
  "call.muted": "Muted",
  "call.mic": "Microphone level",
  "call.gate": "Voice gate",
  "call.gateHelp":
    "Your voice is sent only above this level. Set it just above your background noise.",
  "call.volume": "Volume of {name}",
  "call.loss": "loss {loss}",
  "call.jitter": "buffer {buffer}",
  "call.uplink": "Lost on the way to the relay: {loss}",

  "error.storage": "This device's files could not be read or written.",
  "error.invalid_link": "This is not a Drywire invitation link.",
  "error.bad_instance": "The instance's address cannot be used.",
  "error.refused": "The server refused.",
  "error.network": "The server cannot be reached.",
  "error.protocol": "The server's answer was not understood: an update may be needed.",
  "error.relay": "The relay did not answer.",
  "error.audio": "The sound card could not start.",
  "error.not_found": "Not found.",
  "error.unknown": "Something went wrong.",
} as const;

export type Key = keyof typeof en;

const fr: Record<Key, string> = {
  "app.tagline": "La voix des équipes qui jouent ensemble",
  "language.label": "Langue",

  "identity.title": "Cet appareil",
  "identity.help":
    "La clé de votre appareil. Un opérateur l'ajoute pour vous permettre de créer des espaces sur une instance.",
  "identity.copy": "Copier",
  "identity.copied": "Copiée",

  "accept.title": "Rejoindre avec une invitation",
  "accept.link": "Lien d'invitation",
  "accept.linkPlaceholder": "https://…/i/#…",
  "accept.name": "Votre nom",
  "accept.namePlaceholder": "alex",
  "accept.submit": "Rejoindre l'espace",
  "accept.busy": "Connexion…",
  "accept.done": "Vous êtes maintenant membre de {space}.",

  "spaces.title": "Espaces",
  "spaces.refresh": "Actualiser",
  "spaces.loading": "Chargement…",
  "spaces.empty": "Aucun espace pour l'instant. Collez un lien d'invitation ci-dessous.",
  "spaces.unreachable": "Impossible de lister les salons : {reason}",
  "spaces.join": "Rejoindre",
  "spaces.joining": "Connexion…",

  "call.leave": "Quitter",
  "call.connecting": "Connexion au relais…",
  "call.ended": "L'appel est terminé : {reason}",
  "call.rejoin": "Rejoindre à nouveau",
  "call.close": "Fermer",
  "call.you": "vous",
  "call.speaking": "parle",
  "call.ping": "Ping",
  "call.devices": "Périphériques audio",
  "call.input": "Micro",
  "call.output": "Sortie",
  "call.buffer": "tampon {buffer}, latence annoncée {latency}",
  "call.bluetooth": "Bluetooth : la latence réelle est bien plus élevée qu'affiché.",
  "call.mute": "Couper le micro",
  "call.unmute": "Réactiver le micro",
  "call.muted": "Micro coupé",
  "call.mic": "Niveau du micro",
  "call.gate": "Seuil de voix",
  "call.gateHelp":
    "Votre voix n'est envoyée qu'au-dessus de ce niveau. Placez-le juste au-dessus de votre bruit de fond.",
  "call.volume": "Volume de {name}",
  "call.loss": "pertes {loss}",
  "call.jitter": "tampon {buffer}",
  "call.uplink": "Perdu en route vers le relais : {loss}",

  "error.storage": "Les fichiers de cet appareil n'ont pas pu être lus ou écrits.",
  "error.invalid_link": "Ce n'est pas un lien d'invitation Drywire.",
  "error.bad_instance": "L'adresse de l'instance est inutilisable.",
  "error.refused": "Le serveur a refusé.",
  "error.network": "Le serveur est injoignable.",
  "error.protocol": "La réponse du serveur est incomprise : une mise à jour est peut-être nécessaire.",
  "error.relay": "Le relais n'a pas répondu.",
  "error.audio": "La carte son n'a pas pu démarrer.",
  "error.not_found": "Introuvable.",
  "error.unknown": "Une erreur est survenue.",
};

export type Lang = "en" | "fr";

export const LANGUAGES: readonly Lang[] = ["en", "fr"];

export const dictionaries: Record<Lang, Record<Key, string>> = { en, fr };

/** The text of `key` in `lang`, with each {placeholder} filled from `vars`. */
export function translate(
  lang: Lang,
  key: Key,
  vars: Record<string, string | number> = {},
): string {
  return dictionaries[lang][key].replace(/\{(\w+)\}/g, (whole, name: string) =>
    name in vars ? String(vars[name]) : whole,
  );
}

/** The first of the system's preferred languages that the app speaks. */
export function detectLanguage(preferred: readonly string[]): Lang {
  for (const tag of preferred) {
    const base = tag.toLowerCase().split("-")[0];
    if (base === "fr" || base === "en") return base;
  }
  return "en";
}

/** The placeholders a text uses, sorted: {space} → ["space"]. */
export function placeholders(text: string): string[] {
  return [...text.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
}
