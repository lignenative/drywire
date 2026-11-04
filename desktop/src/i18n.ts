// The interface's text, in English and French (spec section 10). Every
// key exists in both languages: the type of `fr` makes a missing one a
// compile error, and i18n.test.ts checks the placeholders match. The
// French follows the design canvas word for word.
//
// Placeholders are written {name} and filled by `translate`.

const en = {
  "app.name": "Drywire",
  "language.label": "Language",
  "language.en": "English",
  "language.fr": "Français",

  "spaces.label": "Spaces",
  "spaces.join": "Join a space",
  "spaces.unreachable": "Server unreachable",
  "spaces.retry": "Retry",
  "spaces.here": "You are here",
  "spaces.hereShort": "you're in",
  "spaces.back": "Back to the spaces",

  "voice.inCall": "In voice",
  "voice.connecting": "Connecting…",
  "voice.reconnecting": "Reconnecting…",
  "voice.leave": "Leave the room",
  "voice.show": "Show the call",

  "me.muted": "Muted",
  "me.idle": "Not in a call",
  "me.you": "{name} (you)",
  "mute.on": "Mute",
  "mute.off": "Unmute",

  "person.speaking": "speaking",
  "person.silent": "silent",
  "person.muted": "muted",
  "person.mouthToEar": "From their mouth to your ear",
  "person.estimate": "estimate",
  "person.estimateHelp":
    "Your devices, one frame, the round trip and your buffer, assuming their side matches yours.",
  "person.volume": "Volume",
  "person.volumeOf": "Volume of {name}",
  "person.stats":
    "received {received} · played {played} · concealed {concealed} · late {late} · dropped {dropped} · buffered {depth}",

  "call.you": "you",
  "call.link": "Connection details",
  "call.connecting": "Connecting to the relay…",
  "call.reconnecting": "Connection lost: joining the room again…",
  "call.retryIn": "Next try in {seconds} s.",
  "call.bluetooth": "Bluetooth: expect far more latency than shown.",
  "call.alone": "Nobody else yet.",
  "call.gateDefault": "Default threshold: calibrate it out of a call, in the settings.",
  "call.gateButton": "Voice gate, {gate}",

  "gate.title": "Voice gate",
  "gate.sending": "Sending",
  "gate.below": "Below the gate",
  "gate.help": "Your voice is sent only above the white mark. Set it just above your background noise.",
  "gate.default": "Default threshold: calibrate it out of a call.",
  "gate.calibrate": "Calibrate in 10 s, out of a call",

  "link.title": "Connection",
  "link.rtt": "Round trip to the relay",
  "link.rttValue": "{rtt} (min. {min})",
  "link.device": "This device, input + output",
  "link.loss": "Lost on the way to the relay",
  "link.input": "Input",
  "link.output": "Output",
  "link.detail": "{mode} · buffer {buffer} · reported {latency}",
  "badge.measured": "measured",
  "badge.reported": "reported",

  "ended.title": "Call ended",
  "ended.help": "This device joined the room elsewhere.",
  "ended.rejoin": "Join here",
  "ended.rejoinShort": "Join",
  "ended.close": "Close",

  "peek.inRoom": "In the room. You will hear them when you join.",
  "peek.empty": "Nobody yet",
  "peek.join": "Join",
  "peek.joining": "Joining…",
  "peek.switching": "You will leave {where}.",

  "calibrate.title": "Calibrate your voice gate first",
  "calibrate.help": "Ten seconds of silence, controller in hand. Then your voice goes out only when you speak.",
  "calibrate.go": "Calibrate now",

  "empty.title": "No space yet",
  "empty.help": "A friend sends you an invitation link: paste it here to join their space.",
  "empty.paste": "Paste an invitation link",

  "invite.title": "Join a space",
  "invite.link": "Invitation link",
  "invite.linkPlaceholder": "https://…/i/#…",
  "invite.name": "Your name",
  "invite.nameHelp": "The name others see in this space.",
  "invite.nameMissing": "Enter the name others will see.",
  "invite.cancel": "Cancel",
  "invite.submit": "Join the space",
  "invite.busy": "Joining…",

  "settings.title": "Settings",
  "settings.back": "Back",
  "settings.backTo": "Back to the settings",
  "settings.tab.voice": "Voice",
  "settings.tab.sounds": "Sounds",
  "settings.tab.display": "Display",
  "settings.tab.advanced": "Advanced",
  "settings.instant": "Every setting applies at once.",

  "settings.gate": "Voice gate",
  "settings.gateHelp": "Your voice is sent only above it.",
  "settings.calibrate": "Calibrate",
  "settings.calibrating": "Listening… {seconds} s",
  "settings.calibrateIntro": "Ten seconds of silence, controller in hand. Result:",
  "settings.noise": "Background noise",
  "settings.peaks": "Peaks",
  "settings.proposed": "Proposed threshold",
  "settings.tooLoud":
    "It is loud: quiet speech may not open the gate. Lower the game in your headset, or move the microphone.",
  "settings.apply": "Apply",
  "settings.discard": "Cancel",
  "settings.shortcut": "Mute shortcut",
  "settings.shortcutHelp": "Works even while the game has the focus.",
  "settings.shortcutEdit": "Change the shortcut, now {keys}",
  "settings.shortcutRecording": "Press the keys…",
  "settings.shortcutHint": "Backspace removes it, Escape keeps it.",
  "settings.shortcutNone": "None",

  "settings.sounds": "Joins and leaves",
  "settings.soundsHelp": "Played apart: they never delay a voice.",
  "settings.soundVolume": "Sound volume",
  "settings.off": "off",
  "settings.preview.join": "Joins",
  "settings.preview.leave": "Leaves",
  "settings.preview.disconnect": "Connection lost",

  "settings.overlay": "Show who is speaking over the game",
  "settings.overlayHelp": "In borderless windowed mode. Never takes a click.",
  "settings.corner": "Screen corner",
  "settings.corner.top-left": "Top left",
  "settings.corner.top-right": "Top right",
  "settings.corner.bottom-left": "Bottom left",
  "settings.corner.bottom-right": "Bottom right",
  "settings.atLogin": "Start when you log in",
  "settings.atLoginHelp": "Hidden by the clock. Closing the window puts it there too: the call goes on.",

  "settings.deviceTest": "Device test",
  "settings.outOfCall": "· out of a call",
  "settings.deviceTestHelp":
    "Earphone against the microphone, volume up: Drywire plays clicks and times how long the microphone takes to hear them.",
  "settings.run": "Run",
  "settings.rerun": "Run again",
  "settings.listening": "Listening…",
  "settings.deviceResult": "input + output, median of {count} clicks",
  "settings.measured": "Measured",
  "settings.systemSays": "The system says",
  "settings.wireless": "Wireless headsets often add far more than the system reports.",
  "settings.noClick": "No click was heard. Hold the earphone closer to the microphone, and turn the volume up.",
  "settings.key": "Device key",
  "settings.keyHelp": "Only a server operator needs it, to let you create spaces.",
  "settings.copy": "Copy",
  "settings.copied": "Copied",

  "overlay.join": "{name} joined",
  "overlay.leave": "{name} left",
  "overlay.disconnect": "{name} lost their connection",
  "overlay.reconnecting": "Connection lost: joining again…",

  "error.shortcut": "This shortcut cannot be used: another app may hold it.",
  "error.busy": "Leave the room first: the call is using the sound card.",
  "error.storage": "This device's files could not be read or written.",
  "error.invalid_link": "This is not a Drywire invitation link.",
  "error.bad_instance": "The instance's address cannot be used.",
  "error.refused": "The server refused.",
  "error.network": "The server cannot be reached.",
  "error.unavailable": "The server is unavailable for a moment: try again shortly.",
  "error.protocol": "The server's answer was not understood: an update may be needed.",
  "error.relay": "The relay did not answer.",
  "error.audio": "The sound card could not start.",
  "error.not_found": "Not found.",
  "error.autostart": "Starting with the session could not be changed.",
  "error.unknown": "Something went wrong.",
} as const;

export type Key = keyof typeof en;

const fr: Record<Key, string> = {
  "app.name": "Drywire",
  "language.label": "Langue",
  "language.en": "English",
  "language.fr": "Français",

  "spaces.label": "Espaces",
  "spaces.join": "Rejoindre un espace",
  "spaces.unreachable": "Serveur injoignable",
  "spaces.retry": "Réessayer",
  "spaces.here": "Vous êtes ici",
  "spaces.hereShort": "vous y êtes",
  "spaces.back": "Retour aux espaces",

  "voice.inCall": "En vocal",
  "voice.connecting": "Connexion…",
  "voice.reconnecting": "Reconnexion…",
  "voice.leave": "Quitter le salon",
  "voice.show": "Afficher l’appel",

  "me.muted": "Micro coupé",
  "me.idle": "Hors appel",
  "me.you": "{name} (vous)",
  "mute.on": "Couper le micro",
  "mute.off": "Réactiver le micro",

  "person.speaking": "parle",
  "person.silent": "silencieux",
  "person.muted": "micro coupé",
  "person.mouthToEar": "De sa bouche à votre oreille",
  "person.estimate": "estimation",
  "person.estimateHelp":
    "Vos périphériques, une trame, l’aller-retour et votre tampon, en supposant que son côté ressemble au vôtre.",
  "person.volume": "Volume",
  "person.volumeOf": "Volume de {name}",
  "person.stats":
    "reçus {received} · joués {played} · masqués {concealed} · en retard {late} · abandonnés {dropped} · en tampon {depth}",

  "call.you": "vous",
  "call.link": "Détails de la connexion",
  "call.connecting": "Connexion au relais…",
  "call.reconnecting": "Connexion perdue : on rejoint le salon…",
  "call.retryIn": "Nouvel essai dans {seconds} s.",
  "call.bluetooth": "Bluetooth : la latence réelle est bien plus élevée qu’affiché.",
  "call.alone": "Personne d’autre pour l’instant.",
  "call.gateDefault": "Seuil par défaut : calibrez-le hors appel, dans les réglages.",
  "call.gateButton": "Seuil de voix, {gate}",

  "gate.title": "Seuil de voix",
  "gate.sending": "Envoi",
  "gate.below": "Sous le seuil",
  "gate.help":
    "Votre voix n’est envoyée qu’au-dessus du trait blanc. Placez-le juste au-dessus de votre bruit de fond.",
  "gate.default": "Seuil par défaut : calibrez-le hors appel.",
  "gate.calibrate": "Calibrer en 10 s, hors appel",

  "link.title": "Connexion",
  "link.rtt": "Aller-retour vers le relais",
  "link.rttValue": "{rtt} (min. {min})",
  "link.device": "Cet appareil, entrée + sortie",
  "link.loss": "Perdu en route vers le relais",
  "link.input": "Entrée",
  "link.output": "Sortie",
  "link.detail": "{mode} · tampon {buffer} · annoncé {latency}",
  "badge.measured": "mesuré",
  "badge.reported": "annoncé",

  "ended.title": "Appel terminé",
  "ended.help": "Cet appareil a rejoint le salon ailleurs.",
  "ended.rejoin": "Rejoindre ici",
  "ended.rejoinShort": "Rejoindre",
  "ended.close": "Fermer",

  "peek.inRoom": "Dans le salon. Vous les entendrez en rejoignant.",
  "peek.empty": "Personne pour l’instant",
  "peek.join": "Rejoindre",
  "peek.joining": "Connexion…",
  "peek.switching": "Vous quitterez {where}.",

  "calibrate.title": "Calibrez d’abord votre seuil de voix",
  "calibrate.help": "Dix secondes en silence, manette en main. Votre voix ne part ensuite que quand vous parlez.",
  "calibrate.go": "Calibrer maintenant",

  "empty.title": "Aucun espace pour l’instant",
  "empty.help": "Un ami vous envoie un lien d’invitation : collez-le ici pour rejoindre son espace.",
  "empty.paste": "Coller un lien d’invitation",

  "invite.title": "Rejoindre un espace",
  "invite.link": "Lien d’invitation",
  "invite.linkPlaceholder": "https://…/i/#…",
  "invite.name": "Votre nom",
  "invite.nameHelp": "Le nom que les autres voient dans cet espace.",
  "invite.nameMissing": "Entrez le nom que les autres verront.",
  "invite.cancel": "Annuler",
  "invite.submit": "Rejoindre l’espace",
  "invite.busy": "Connexion…",

  "settings.title": "Réglages",
  "settings.back": "Retour",
  "settings.backTo": "Retour aux réglages",
  "settings.tab.voice": "Voix",
  "settings.tab.sounds": "Sons",
  "settings.tab.display": "Affichage",
  "settings.tab.advanced": "Avancé",
  "settings.instant": "Chaque réglage s’applique tout de suite.",

  "settings.gate": "Seuil de voix",
  "settings.gateHelp": "Votre voix n’est envoyée qu’au-dessus.",
  "settings.calibrate": "Calibrer",
  "settings.calibrating": "Écoute… {seconds} s",
  "settings.calibrateIntro": "Dix secondes en silence, manette en main. Résultat :",
  "settings.noise": "Bruit de fond",
  "settings.peaks": "Pointes",
  "settings.proposed": "Seuil proposé",
  "settings.tooLoud":
    "C’est bruyant : une voix basse risque de ne pas ouvrir le seuil. Baissez le jeu dans le casque, ou éloignez le micro.",
  "settings.apply": "Appliquer",
  "settings.discard": "Annuler",
  "settings.shortcut": "Raccourci pour couper le micro",
  "settings.shortcutHelp": "Marche même quand le jeu a le focus.",
  "settings.shortcutEdit": "Modifier le raccourci, actuellement {keys}",
  "settings.shortcutRecording": "Appuyez sur les touches…",
  "settings.shortcutHint": "Retour arrière l’efface, Échap le garde.",
  "settings.shortcutNone": "Aucun",

  "settings.sounds": "Arrivées et départs",
  "settings.soundsHelp": "Joués à part : ils ne retardent jamais une voix.",
  "settings.soundVolume": "Volume des sons",
  "settings.off": "coupés",
  "settings.preview.join": "Arrivée",
  "settings.preview.leave": "Départ",
  "settings.preview.disconnect": "Connexion perdue",

  "settings.overlay": "Afficher qui parle par-dessus le jeu",
  "settings.overlayHelp": "En fenêtré sans bordure. Ne prend jamais un clic.",
  "settings.corner": "Coin de l’écran",
  "settings.corner.top-left": "En haut à gauche",
  "settings.corner.top-right": "En haut à droite",
  "settings.corner.bottom-left": "En bas à gauche",
  "settings.corner.bottom-right": "En bas à droite",
  "settings.atLogin": "Lancer à l’ouverture de session",
  "settings.atLoginHelp": "Caché près de l’horloge. Fermer la fenêtre l’y range aussi : l’appel continue.",

  "settings.deviceTest": "Test des périphériques",
  "settings.outOfCall": "· hors appel",
  "settings.deviceTestHelp":
    "Écouteur contre le micro, volume haut : Drywire joue des clics et chronomètre le temps que le micro met à les entendre.",
  "settings.run": "Lancer",
  "settings.rerun": "Relancer",
  "settings.listening": "Écoute…",
  "settings.deviceResult": "entrée + sortie, médiane de {count} clics",
  "settings.measured": "Mesuré",
  "settings.systemSays": "Le système dit",
  "settings.wireless": "Les casques sans fil ajoutent souvent bien plus que ce qu’annonce le système.",
  "settings.noClick": "Aucun clic entendu. Rapprochez l’écouteur du micro et montez le volume.",
  "settings.key": "Clé de l’appareil",
  "settings.keyHelp": "Seul un opérateur de serveur en a besoin, pour vous laisser créer des espaces.",
  "settings.copy": "Copier",
  "settings.copied": "Copiée",

  "overlay.join": "{name} est arrivé·e",
  "overlay.leave": "{name} est parti·e",
  "overlay.disconnect": "{name} a perdu la connexion",
  "overlay.reconnecting": "Connexion perdue : on rejoint…",

  "error.shortcut": "Ce raccourci est inutilisable : une autre application le détient peut-être.",
  "error.busy": "Quittez d’abord le salon : l’appel utilise la carte son.",
  "error.storage": "Les fichiers de cet appareil n’ont pas pu être lus ou écrits.",
  "error.invalid_link": "Ce n’est pas un lien d’invitation Drywire.",
  "error.bad_instance": "L’adresse de l’instance est inutilisable.",
  "error.refused": "Le serveur a refusé.",
  "error.network": "Le serveur est injoignable.",
  "error.unavailable": "Le serveur est momentanément indisponible : réessayez dans un instant.",
  "error.protocol": "La réponse du serveur est incomprise : une mise à jour est peut-être nécessaire.",
  "error.relay": "Le relais n’a pas répondu.",
  "error.audio": "La carte son n’a pas pu démarrer.",
  "error.not_found": "Introuvable.",
  "error.autostart": "Le lancement à l’ouverture de session n’a pas pu être modifié.",
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
