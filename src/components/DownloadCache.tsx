import { useEffect, useState } from "react";
import { command, desktop } from "../bridge";
import type { CacheReport } from "../types";
import type { ViewProps } from "./shared";
import Modal from "./Modal";
export const bytes = (value: number) =>
  value < 1024
    ? `${value} B`
    : value < 1024 * 1024
      ? `${(value / 1024).toFixed(2)} KiB`
      : `${(value / 1024 / 1024).toFixed(2)} MiB`;
export default function DownloadCache({ t, run, pending, data }: ViewProps) {
  const [report, setReport] = useState<CacheReport | null>(null);
  const [confirm, setConfirm] = useState(false);
  const [loading, setLoading] = useState(false);
  useEffect(() => {
    if (!desktop) return;
    let live = true;
    setLoading(true);
    void command<CacheReport>("get_download_cache")
      .then((r) => {
        if (live) setReport(r);
      })
      .catch(() => {
        if (live) setReport(null);
      })
      .finally(() => {
        if (live) setLoading(false);
      });
    return () => {
      live = false;
    };
  }, [
    data.installerReady,
    data.operation?.kind,
    data.settings.downloadRetentionDays,
    data.settings.downloadCacheMb,
  ]);
  const refresh = () =>
    run(async () =>
      setReport(await command<CacheReport>("get_download_cache")),
    );
  return (
    <section className="settings-section">
      <h2>{t("downloadCache")}</h2>
      <p>{t("cachePolicy")}</p>
      <p className="helper">
        {t("retentionDays")}: {data.settings.downloadRetentionDays} ·{" "}
        {t("cacheLimitMb")}: {data.settings.downloadCacheMb}
      </p>
      {report ? (
        <dl>
          <div>
            <dt>{t("cacheSize")}</dt>
            <dd>
              {bytes(report.bytes)} · {report.attempts}
            </dd>
          </div>
          <div>
            <dt>{t("reclaimable")}</dt>
            <dd>
              {bytes(report.reclaimableBytes)} · {report.removable}
            </dd>
          </div>
          <div>
            <dt>{t("cacheSkipped")}</dt>
            <dd>{report.skipped}</dd>
          </div>
        </dl>
      ) : (
        <p className="helper">
          {loading ? t("loadingDetails") : t("cacheUnavailable")}
        </p>
      )}
      <div className="button-row">
        <button
          className="secondary"
          disabled={!desktop || pending || loading}
          onClick={() => void refresh()}
        >
          {t("refreshDetails")}
        </button>
        <button
          className="secondary"
          disabled={
            !desktop || pending || !!data.operation || !report?.removable
          }
          onClick={() => setConfirm(true)}
        >
          {t("cleanDownloads")}
        </button>
      </div>
      {confirm && (
        <Modal
          title={t("cleanDownloads")}
          close={() => setConfirm(false)}
          closeLabel={t("cancel")}
        >
          <p>{t("cachePolicy")}</p>
          <p>
            {t("reclaimable")}: {bytes(report?.reclaimableBytes ?? 0)} ·{" "}
            {report?.removable ?? 0}
          </p>
          <div className="modal-actions">
            <button className="secondary" onClick={() => setConfirm(false)}>
              {t("cancel")}
            </button>
            <button
              className="primary"
              disabled={pending || !!data.operation}
              onClick={() =>
                void run(async () => {
                  await command("cleanup_download_cache");
                  setReport(await command<CacheReport>("get_download_cache"));
                }, "cacheCleaned").then((ok) => {
                  if (ok) setConfirm(false);
                })
              }
            >
              {t("cleanDownloads")}
            </button>
          </div>
        </Modal>
      )}
    </section>
  );
}
