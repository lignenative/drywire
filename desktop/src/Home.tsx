// Out of a call: the spaces this device knows and their rooms, the form
// that accepts an invitation link, and the device key.
import { useCallback, useEffect, useState, type FormEvent } from "react";
import { api, type CallView, type SpaceView } from "./api";
import { hostOf } from "./format";
import { errorText, useLanguage } from "./lang";

const NAME_KEY = "drywire.displayName";

export function Home({ onJoined }: { onJoined: (call: CallView) => void }) {
  const { t } = useLanguage();
  const [spaces, setSpaces] = useState<SpaceView[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [joining, setJoining] = useState<string | null>(null);

  const load = useCallback(() => {
    setSpaces(null);
    api.listSpaces().then(setSpaces, (e) => {
      setSpaces([]);
      setError(errorText(t, e));
    });
  }, [t]);

  useEffect(load, []);

  const join = async (space: SpaceView, roomId: string) => {
    setJoining(roomId);
    setError(null);
    try {
      onJoined(await api.joinRoom(space.instance, space.id, roomId));
    } catch (e) {
      setError(errorText(t, e));
    } finally {
      setJoining(null);
    }
  };

  return (
    <>
      <section className="card">
        <div className="row spread">
          <h2>{t("spaces.title")}</h2>
          <button className="ghost" onClick={load} disabled={spaces === null}>
            {t("spaces.refresh")}
          </button>
        </div>
        {error && <p className="error">{error}</p>}
        {spaces === null && <p className="muted">{t("spaces.loading")}</p>}
        {spaces?.length === 0 && <p className="muted">{t("spaces.empty")}</p>}
        {spaces?.map((space) => (
          <div className="space" key={`${space.instance} ${space.id}`}>
            <div className="row spread">
              <strong>{space.name}</strong>
              <span className="muted small">{hostOf(space.instance)}</span>
            </div>
            {space.error && (
              <p className="error small">
                {t("spaces.unreachable", { reason: errorText(t, space.error) })}
              </p>
            )}
            <ul className="rooms">
              {space.rooms.map((room) => (
                <li key={room.id} className="row spread">
                  <span># {room.name}</span>
                  <button
                    onClick={() => join(space, room.id)}
                    disabled={joining !== null}
                  >
                    {joining === room.id ? t("spaces.joining") : t("spaces.join")}
                  </button>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </section>
      <AcceptForm onAccepted={load} />
      <Identity />
    </>
  );
}

function AcceptForm({ onAccepted }: { onAccepted: () => void }) {
  const { t } = useLanguage();
  const [link, setLink] = useState("");
  const [name, setName] = useState(() => localStorage.getItem(NAME_KEY) ?? "");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setMessage(null);
    try {
      const space = await api.acceptInvitation(link.trim(), name.trim());
      localStorage.setItem(NAME_KEY, name.trim());
      setLink("");
      setMessage({ text: t("accept.done", { space: space.name }), error: false });
      onAccepted();
    } catch (e) {
      setMessage({ text: errorText(t, e), error: true });
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="card">
      <h2>{t("accept.title")}</h2>
      <form onSubmit={submit} className="stack">
        <label>
          <span>{t("accept.link")}</span>
          <input
            value={link}
            onChange={(e) => setLink(e.target.value)}
            placeholder={t("accept.linkPlaceholder")}
            autoComplete="off"
            spellCheck={false}
            required
          />
        </label>
        <label>
          <span>{t("accept.name")}</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={t("accept.namePlaceholder")}
            maxLength={64}
            required
          />
        </label>
        <button type="submit" disabled={busy || !link.trim() || !name.trim()}>
          {busy ? t("accept.busy") : t("accept.submit")}
        </button>
        {message && <p className={message.error ? "error" : "ok"}>{message.text}</p>}
      </form>
    </section>
  );
}

function Identity() {
  const { t } = useLanguage();
  const [key, setKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    api.publicKey().then(setKey, (e) => setError(errorText(t, e)));
  }, []);

  const copy = async () => {
    if (!key) return;
    await navigator.clipboard.writeText(key);
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  return (
    <section className="card">
      <div className="row spread">
        <h2>{t("identity.title")}</h2>
        <button className="ghost" onClick={copy} disabled={!key}>
          {copied ? t("identity.copied") : t("identity.copy")}
        </button>
      </div>
      <p className="muted small">{t("identity.help")}</p>
      {key && <code className="key">{key}</code>}
      {error && <p className="error">{error}</p>}
    </section>
  );
}
