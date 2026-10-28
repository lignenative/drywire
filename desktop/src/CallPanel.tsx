// In a call: the compact widget (spec section 10). Where and how fast,
// the microphone (meter, gate, mute), then everyone in the room with a
// speaking light and their own volume.
import { useEffect, useRef, useState } from "react";
import { api, type CallView, type Participant } from "./api";
import {
  formatMs,
  hostOf,
  linkQuality,
  meterPercent,
  volumePercent,
} from "./format";
import { errorText, useLanguage } from "./lang";

/** Calls `f` once the value stopped changing for `ms`: a slider being
    dragged writes the settings once, not at every step. */
function useDebounced<T>(f: (value: T) => void, ms = 150): (value: T) => void {
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const latest = useRef(f);
  latest.current = f;
  useEffect(() => () => clearTimeout(timer.current), []);
  return (value: T) => {
    clearTimeout(timer.current);
    timer.current = setTimeout(() => latest.current(value), ms);
  };
}

export function CallPanel({ call, onLeave }: { call: CallView; onLeave: () => void }) {
  const { t } = useLanguage();
  const [error, setError] = useState<string | null>(null);
  const quality = linkQuality(call.rttMs, call.uplinkLossPct);

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
        <div className="where">
          <h2># {call.roomName}</h2>
          <p className="muted small">
            {call.spaceName} · {hostOf(call.instance)}
          </p>
        </div>
        <div
          className={`ping ${call.bound ? quality : "poor"}`}
          title={
            call.uplinkLossPct > 0
              ? t("call.uplink", { loss: `${call.uplinkLossPct.toFixed(1)} %` })
              : t("call.ping")
          }
        >
          <span className="dot" />
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

      {!call.ended && <Microphone call={call} />}

      <ul className="people">
        {call.participants.map((p) => (
          <Person key={p.session} person={p} />
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

/** The level meter with the gate's threshold on it, the mute button, and
    the threshold's slider. */
function Microphone({ call }: { call: CallView }) {
  const { t } = useLanguage();
  // The slider moves at once; the call follows when it rests.
  const [gate, setGate] = useState(call.gateDbfs);
  const dragging = useRef(false);
  useEffect(() => {
    if (!dragging.current) setGate(call.gateDbfs);
  }, [call.gateDbfs]);
  const saveGate = useDebounced((value: number) => {
    dragging.current = false;
    api.setGate(value).catch(() => {});
  });
  const open = call.inputDbfs >= gate;

  return (
    <div className={call.muted ? "mic muted" : "mic"}>
      <div className="row">
        <div
          className="meter"
          role="meter"
          aria-label={t("call.mic")}
          aria-valuemin={-90}
          aria-valuemax={0}
          aria-valuenow={Math.round(call.inputDbfs)}
        >
          <div
            className={open ? "level open" : "level"}
            style={{ width: `${meterPercent(call.inputDbfs)}%` }}
          />
          <div className="threshold" style={{ left: `${meterPercent(gate)}%` }} />
        </div>
        <button
          className={call.muted ? "mute on" : "mute"}
          onClick={() => api.setMuted(!call.muted)}
          aria-pressed={call.muted}
        >
          {call.muted ? t("call.unmute") : t("call.mute")}
        </button>
      </div>
      {call.muted && <p className="muted-label small">{t("call.muted")}</p>}
      <label className="gate" title={t("call.gateHelp")}>
        <span className="row spread">
          <span>{t("call.gate")}</span>
          <span className="value">{Math.round(gate)} dB</span>
        </span>
        <input
          type="range"
          min={-80}
          max={-10}
          step={1}
          value={gate}
          onChange={(e) => {
            dragging.current = true;
            const value = Number(e.target.value);
            setGate(value);
            saveGate(value);
          }}
        />
      </label>
    </div>
  );
}

/** One person: speaking light, name, link figures, and, for the others,
    their volume from 0 to 200 %. */
function Person({ person }: { person: Participant }) {
  const { t } = useLanguage();
  const [volume, setVolume] = useState(volumePercent(person.volume));
  const dragging = useRef(false);
  useEffect(() => {
    if (!dragging.current) setVolume(volumePercent(person.volume));
  }, [person.volume]);
  const saveVolume = useDebounced((value: number) => {
    dragging.current = false;
    api.setVolume(person.publicKey, value / 100).catch(() => {});
  });

  const figures = [
    person.bufferMs !== null ? t("call.jitter", { buffer: formatMs(person.bufferMs) }) : null,
    person.lossPct !== null ? t("call.loss", { loss: `${person.lossPct.toFixed(0)} %` }) : null,
  ].filter(Boolean);

  return (
    <li className={person.speaking ? "person speaking" : "person"}>
      <div className="row">
        <span className="talk" aria-label={person.speaking ? t("call.speaking") : undefined} />
        <span className="name">{person.name}</span>
        {person.me && <span className="muted small">({t("call.you")})</span>}
        {figures.length > 0 && <span className="figures muted small">{figures.join(" · ")}</span>}
      </div>
      {!person.me && (
        <label className="volume">
          <input
            type="range"
            min={0}
            max={200}
            step={5}
            value={volume}
            aria-label={t("call.volume", { name: person.name })}
            onChange={(e) => {
              dragging.current = true;
              const value = Number(e.target.value);
              setVolume(value);
              saveVolume(value);
            }}
          />
          <span className="value small">{volume} %</span>
        </label>
      )}
    </li>
  );
}
