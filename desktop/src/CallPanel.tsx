// In a call: where, who, and how the link and the sound card are doing.
import { api, type CallView } from "./api";
import { formatMs, hostOf } from "./format";
import { errorText, useLanguage } from "./lang";
import { useState } from "react";

export function CallPanel({ call, onLeave }: { call: CallView; onLeave: () => void }) {
  const { t } = useLanguage();
  const [error, setError] = useState<string | null>(null);

  const rejoin = async () => {
    setError(null);
    try {
      await api.joinRoom(call.instance, call.spaceId, call.roomId);
    } catch (e) {
      setError(errorText(t, e));
    }
  };

  return (
    <section className="card call">
      <div className="row spread">
        <div>
          <h2># {call.roomName}</h2>
          <p className="muted small">
            {call.spaceName} · {hostOf(call.instance)}
          </p>
        </div>
        <div className="ping" title={t("call.ping")}>
          <span className={call.bound ? "dot live" : "dot"} />
          {call.rttMs !== null ? formatMs(call.rttMs) : "–"}
        </div>
      </div>

      {!call.bound && !call.ended && <p className="muted">{t("call.connecting")}</p>}
      {call.ended && (
        <div className="banner">
          <p>{t("call.ended", { reason: call.ended })}</p>
          <div className="row">
            <button onClick={rejoin}>{t("call.rejoin")}</button>
            <button className="ghost" onClick={onLeave}>
              {t("call.close")}
            </button>
          </div>
        </div>
      )}
      {error && <p className="error">{error}</p>}

      <ul className="people">
        {call.participants.map((p) => (
          <li key={p.session} className={p.speaking ? "person speaking" : "person"}>
            <span className="talk" aria-label={p.speaking ? t("call.speaking") : undefined} />
            <span className="name">{p.name}</span>
            {p.me && <span className="muted small">({t("call.you")})</span>}
          </li>
        ))}
      </ul>

      <details className="devices">
        <summary>{t("call.devices")}</summary>
        {call.devices.map((d) => (
          <div key={d.direction} className="small">
            <strong>{d.direction === "input" ? t("call.input") : t("call.output")}</strong>{" "}
            {d.name} · {d.mode}
            <div className="muted">
              {t("call.buffer", {
                buffer: formatMs(d.bufferMs),
                latency: formatMs(d.latencyMs),
              })}
            </div>
            {d.bluetooth && <div className="warn">{t("call.bluetooth")}</div>}
          </div>
        ))}
      </details>

      {!call.ended && (
        <button className="danger wide" onClick={onLeave}>
          {t("call.leave")}
        </button>
      )}
    </section>
  );
}
