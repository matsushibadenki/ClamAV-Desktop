import { useState } from "react";
import { command, desktop } from "../bridge";
import { date } from "../i18n";
import type { ViewProps } from "../components/shared";
export default function Settings({ data, t, run, pending, setup }: ViewProps) {
  const [draft, setDraft] = useState(data.settings);
  const busy = pending || !!data.operation || data.scan?.status === "running";
  return (
    <>
      <header className="page-heading">
        <h1>{t("settingsTitle")}</h1>
        <p>{t("settingsDesc")}</p>
      </header>
      <section className="settings-section">
        <h2>{t("engineSection")}</h2>
        <dl>
          <div>
            <dt>{t("engine")}</dt>
            <dd>{data.engine.version || t("notInstalled")}</dd>
          </div>
          <div>
            <dt>{t("enginePath")}</dt>
            <dd className="path">{data.engine.path || "—"}</dd>
          </div>
          <div>
            <dt>{t("definitions")}</dt>
            <dd>
              {data.engine.definitions
                ? date(data.engine.definitionsAt, data.settings.language)
                : t("notDownloaded")}
            </dd>
          </div>
        </dl>
        <div className="button-row">
          <button
            className="primary"
            disabled={busy || !desktop || !data.engine.version}
            onClick={() => void run(() => command("update_definitions"))}
          >
            {t("update")}
          </button>
          <button className="secondary" disabled={busy} onClick={setup}>
            {t("reinstall")}
          </button>
        </div>
      </section>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void run(
            () => command("save_settings", { settings: draft }),
            "saved",
          );
        }}
      >
        <section className="settings-section">
          <label className="setting-toggle">
            <span>
              <strong>{t("autoUpdate")}</strong>
              <small>{t("autoUpdateDesc")}</small>
            </span>
            <input
              type="checkbox"
              role="switch"
              checked={draft.autoUpdate}
              onChange={(e) =>
                setDraft({ ...draft, autoUpdate: e.target.checked })
              }
            />
          </label>
          <label className="setting-toggle">
            <span>
              <strong>{t("schedule")}</strong>
              <small>{t("scheduleDesc")}</small>
            </span>
            <input
              type="checkbox"
              role="switch"
              checked={draft.scheduleEnabled}
              onChange={(e) =>
                setDraft({ ...draft, scheduleEnabled: e.target.checked })
              }
            />
          </label>
          <label className="field">
            <span>{t("hour")}</span>
            <select
              disabled={!draft.scheduleEnabled}
              value={draft.scheduleHour}
              onChange={(e) =>
                setDraft({ ...draft, scheduleHour: Number(e.target.value) })
              }
            >
              {Array.from({ length: 24 }, (_, i) => (
                <option key={i} value={i}>
                  {String(i).padStart(2, "0")}:00
                </option>
              ))}
            </select>
          </label>
        </section>
        <section className="settings-section">
          <h2>{t("limits")}</h2>
          <label className="field">
            <span>{t("maxFile")}</span>
            <input
              required
              type="number"
              min={1}
              max={4096}
              value={draft.maxFileMb}
              onChange={(e) =>
                setDraft({ ...draft, maxFileMb: Number(e.target.value) })
              }
            />
          </label>
          <p>{t("limitsDesc")}</p>
          <button type="submit" className="primary" disabled={pending}>
            {t("save")}
          </button>
        </section>
      </form>
      <section className="settings-section">
        <h2>{t("privacy")}</h2>
        <p>{t("privacyDesc")}</p>
        <button
          className="secondary"
          disabled={!desktop || pending}
          onClick={() => void run(() => command("open_privacy_settings"))}
        >
          {t("fullDisk")}
        </button>
        <h3>{t("dataDir")}</h3>
        <p className="path">{data.dataDir}</p>
        <p className="helper">{t("onDemand")}</p>
      </section>
    </>
  );
}
