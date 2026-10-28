// The latency lab, out of a call (spec section 11): the loopback device
// test, and the 10-second voice gate calibration (spec section 10). Both
// need the sound card the call would hold.
import { useEffect, useState } from "react";
import { api, type CalibrationView, type LoopbackView } from "./api";
import { formatMs } from "./format";
import { errorText, useLanguage } from "./lang";

const CALIBRATION_SECONDS = 10;

export function LabCard() {
  const { t } = useLanguage();
  return (
    <section className="card">
      <h2>{t("lab.title")}</h2>
      <DeviceTest />
      <Calibration />
      <p className="muted small">{t("lab.network")}</p>
    </section>
  );
}

function DeviceTest() {
  const { t } = useLanguage();
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<LoopbackView | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async () => {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      setResult(await api.loopbackTest());
    } catch (e) {
      setError(errorText(t, e));
    } finally {
      setBusy(false);
    }
  };

  const reported = result?.devices.reduce((sum, d) => sum + d.bufferMs + d.latencyMs, 0);
  return (
    <div className="lab-step">
      <div className="row spread">
        <strong>{t("lab.device")}</strong>
        <button onClick={run} disabled={busy}>
          {busy ? t("lab.listening") : t("lab.start")}
        </button>
      </div>
      <p className="muted small">{t("lab.deviceHelp")}</p>
      {result?.medianMs != null && (
        <p className="ok">
          {t("lab.deviceResult", { ms: formatMs(result.medianMs), count: result.delaysMs.length })}
        </p>
      )}
      {result && result.medianMs == null && <p className="warn">{t("lab.noClick")}</p>}
      {result && reported !== undefined && (
        <p className="muted small">{t("lab.reported", { ms: formatMs(reported) })}</p>
      )}
      {error && <p className="error">{error}</p>}
    </div>
  );
}

function Calibration() {
  const { t } = useLanguage();
  const [left, setLeft] = useState<number | null>(null);
  const [found, setFound] = useState<CalibrationView | null>(null);
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);

  // A countdown while the Rust side records.
  useEffect(() => {
    if (left === null || left <= 0) return;
    const timer = setTimeout(() => setLeft(left - 1), 1000);
    return () => clearTimeout(timer);
  }, [left]);

  const run = async () => {
    setFound(null);
    setMessage(null);
    setLeft(CALIBRATION_SECONDS);
    try {
      setFound(await api.calibrateGate());
    } catch (e) {
      setMessage({ text: errorText(t, e), error: true });
    } finally {
      setLeft(null);
    }
  };

  const apply = async () => {
    if (!found) return;
    try {
      await api.setGate(found.gateDbfs);
      setMessage({ text: t("lab.applied", { gate: `${Math.round(found.gateDbfs)} dB` }), error: false });
      setFound(null);
    } catch (e) {
      setMessage({ text: errorText(t, e), error: true });
    }
  };

  const dB = (value: number) => `${Math.round(value)} dB`;
  return (
    <div className="lab-step">
      <div className="row spread">
        <strong>{t("lab.calibrate")}</strong>
        <button onClick={run} disabled={left !== null}>
          {left !== null ? t("lab.recording", { seconds: Math.max(left, 0) }) : t("lab.start")}
        </button>
      </div>
      <p className="muted small">{t("lab.calibrateHelp")}</p>
      {left !== null && (
        <div className="progress">
          <div style={{ width: `${((CALIBRATION_SECONDS - left) / CALIBRATION_SECONDS) * 100}%` }} />
        </div>
      )}
      {found && (
        <div className="stack">
          <p className="small">
            {t("lab.noise", { noise: dB(found.noiseDbfs), peak: dB(found.peakDbfs) })}{" "}
            <strong>{t("lab.proposed", { gate: dB(found.gateDbfs) })}</strong>
          </p>
          {found.tooLoud && <p className="warn small">{t("lab.tooLoud")}</p>}
          <div className="row">
            <button onClick={apply}>{t("lab.apply")}</button>
            <button className="ghost" onClick={() => setFound(null)}>
              {t("lab.discard")}
            </button>
          </div>
        </div>
      )}
      {message && <p className={message.error ? "error" : "ok"}>{message.text}</p>}
    </div>
  );
}
