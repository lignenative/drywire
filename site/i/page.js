// The invitation page: says which instance the link is for, and how to
// accept it. See invite.js for what is read, and README.md for why the
// page makes no request.
import { readInvitation } from "./invite.js";

const french = (navigator.language || "").toLowerCase().startsWith("fr");
const text = french
  ? {
      title: "Invitation Drywire",
      lead: "Vous êtes invité dans un espace Drywire, sur le serveur :",
      open: "Ouvrir dans Drywire",
      cli: "Ou, avec le client en ligne de commande :",
      copy: "Copier la commande",
      copied: "Copié",
      private:
        "Ce lien contient la clé de l'espace : ne le partagez qu'avec les personnes invitées. Cette page ne l'envoie nulle part.",
      get: "Pas encore installé ? Drywire est en préparation ; le client en ligne de commande se compile depuis github.com/lignenative/drywire.",
      damaged: "Ce lien d'invitation est abîmé : copiez-le à nouveau, en entier.",
      newer: "Ce lien vient d'une version plus récente de Drywire : mettez Drywire à jour.",
      none: "Cette page sert aux liens d'invitation Drywire. Ce lien ne contient pas d'invitation.",
    }
  : {
      title: "Drywire invitation",
      lead: "You are invited to a Drywire space, on the server:",
      open: "Open in Drywire",
      cli: "Or, with the command-line client:",
      copy: "Copy the command",
      copied: "Copied",
      private:
        "This link holds the space's key: share it only with the people you invite. This page sends it nowhere.",
      get: "Not installed yet? Drywire is in the works; the command-line client builds from github.com/lignenative/drywire.",
      damaged: "This invitation link is damaged: copy it again, whole.",
      newer: "This link comes from a newer Drywire: update Drywire.",
      none: "This page is for Drywire invitation links. This one holds no invitation.",
    };

const $ = (id) => document.getElementById(id);
document.documentElement.lang = french ? "fr" : "en";
document.title = text.title;
$("title").textContent = text.title;
$("private").textContent = text.private;
$("get").textContent = text.get;

// A new link opened in this tab changes only the fragment, which does not
// reload the page: start over for it. (The reload sends no fragment.)
addEventListener("hashchange", () => location.reload());

const fragment = location.hash.slice(1);
// Keep the code and the key out of the page's history entry, as far as the
// browser lets a page: it may have recorded the full link already.
if (fragment) history.replaceState(null, "", location.pathname);

const result = fragment ? readInvitation(fragment) : { error: "none" };
if (result.error) {
  $("problem").textContent = text[result.error];
  $("problem").hidden = false;
} else {
  const link = "https://drywire.gg/i#" + fragment;
  $("lead").textContent = text.lead;
  $("instance").textContent = result.instance;
  $("open").textContent = text.open;
  $("open").href = "drywire://i#" + fragment;
  $("cli").textContent = text.cli;
  $("command").textContent = "drywire-cli join --link '" + link + "'";
  $("copy").textContent = text.copy;
  $("copy").addEventListener("click", async () => {
    await navigator.clipboard.writeText($("command").textContent);
    $("copy").textContent = text.copied;
  });
  $("invitation").hidden = false;
}
