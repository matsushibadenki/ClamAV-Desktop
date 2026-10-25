import WorkerMode from "../components/WorkerMode";
import AppUpdater from "../components/AppUpdater";
import InfoHint from "../components/InfoHint";
import useRetryBlock from "../useRetryBlock";
import Diagnostics from "../components/Diagnostics";
import DownloadCache from "../components/DownloadCache";
import { useState } from "react";
import { command, desktop } from "../bridge";
import { date, isCopyKey } from "../i18n";
import type { ViewProps } from "../components/shared";
export default function Settings(props: ViewProps) {
  const { data, t, run, pending, setup } = props;
  const retryBlocked = useRetryBlock(data.diagnostics);
  const [draft, setDraft] = useState(data.settings);
  const scheduled = data.scheduledRun;
  const scheduleStatus = `scheduled_${scheduled?.status ?? "pending"}`;
  const scheduleReason = `scheduleReason_${scheduled?.reason ?? "unknown"}`;
  const busy = pending || !!data.operation || data.scan?.status === "running";
  return (
    <>
      <header className="page-heading">
        <h1>{t("settingsTitle")}</h1>
        <InfoHint text={t("settingsDesc")} label={t("moreInfo")} />
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
            disabled={
              busy ||
              !desktop ||
              !data.engine.version ||
              retryBlocked("definitions_update")
            }
            onClick={() => void run(() => command("update_definitions"))}
          >
            {t("update")}
          </button>
          <button className="secondary" disabled={busy} onClick={setup}>
            {t("reinstall")}
          </button>
        </div>
      </section>
      <WorkerMode {...props} />
      <AppUpdater {...props} />
      <Diagnostics {...props} />
      <DownloadCache {...props} />
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
              <InfoHint text={t("autoUpdateDesc")} label={t("moreInfo")} />
            </span>
            <input
              type="checkbox"
              role="switch"
              aria-label={t("autoUpdate")}
              checked={draft.autoUpdate}
              onChange={(e) =>
                setDraft({ ...draft, autoUpdate: e.target.checked })
              }
            />
          </label>
          <label className="setting-toggle">
            <span>
              <strong>{t("schedule")}</strong>
              <InfoHint text={t("scheduleDesc")} label={t("moreInfo")} />
            </span>
            <input
              type="checkbox"
              role="switch"
              aria-label={t("schedule")}
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
        {scheduled && (
          <section className="settings-section" aria-live="polite">
            <h2>{t("savedSchedule")}</h2>
            <dl>
              <div><dt>{scheduled.localDate} · {String(scheduled.localHour).padStart(2, "0")}:00</dt>
                <dd>{isCopyKey(scheduleStatus) ? t(scheduleStatus) : t("scheduleReason_unknown")}</dd></div>
            </dl>
            {scheduled.reason && <p>{isCopyKey(scheduleReason) ? t(scheduleReason) : t("scheduleReason_unknown")}</p>}
            {scheduled.detail && <details><summary>{t("moreInfo")}</summary><pre>{scheduled.detail}</pre></details>}
          </section>
        )}
        <section className="settings-section">
          <h2>{t("downloadPolicy")}</h2>
          <label className="field">
            <span>{t("retentionDays")}</span>
            <input
              required
              type="number"
              min={1}
              max={365}
              value={draft.downloadRetentionDays}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  downloadRetentionDays: Number(e.target.value),
                })
              }
            />
          </label>
          <label className="field">
            <span>{t("cacheLimitMb")}</span>
            <input
              required
              type="number"
              min={257}
              max={16384}
              value={draft.downloadCacheMb}
              onChange={(e) =>
                setDraft({ ...draft, downloadCacheMb: Number(e.target.value) })
              }
            />
          </label>
          <InfoHint text={t("cacheLimitHelp")} label={t("moreInfo")} />
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
          <InfoHint text={t("limitsDesc")} label={t("moreInfo")} />
          <button type="submit" className="primary" disabled={pending}>
            {t("save")}
          </button>
        </section>
      </form>
      <section className="settings-section">
        <h2>{t("privacy")}</h2>
        <InfoHint text={t("privacyDesc")} label={t("moreInfo")} />
        <button
          className="secondary"
          disabled={!desktop || pending}
          onClick={() => void run(() => command("open_privacy_settings"))}
        >
          {t("fullDisk")}
        </button>
        <h3>{t("dataDir")}</h3>
        <p className="path">{data.dataDir}</p>
        <InfoHint text={t("onDemand")} label={t("moreInfo")} />
      </section>
    </>
  );
}
