import InfoHint from "./InfoHint";
import useRetryBlock from "../useRetryBlock";
import { command, desktop } from "../bridge";
import { date, isCopyKey } from "../i18n";
import type { DiagnosticAttempt } from "../types";
import type { ViewProps } from "./shared";
export default function Diagnostics({
  data,
  t,
  run,
  pending,
  setup,
  kinds,
  compact = false,
}: ViewProps & { kinds?: string[]; compact?: boolean }) {
  const retryBlocked = useRetryBlock(data.diagnostics);
  const attempts = data.diagnostics.filter(
    (d) => (!kinds || kinds.includes(d.kind)) && d.status === "failed",
  );
  const busy = pending || !!data.operation || data.scan?.status === "running";
  const retry = (attempt: DiagnosticAttempt) => {
    if (attempt.kind === "engine_install")
      return run(() => command("prepare_installer"));
    if (attempt.kind === "engine_launch")
      return run(() => command("open_installer"));
    if (attempt.kind === "definitions_update")
      return run(() => command("update_definitions"));
    return run(() => command("refresh_engine"));
  };
  const label = (prefix: string, code: string) => {
    const key = `${prefix}_${code}`;
    return isCopyKey(key)
      ? t(key)
      : t(
          prefix === "diagnosticKind"
            ? "diagnosticKind_unknown"
            : "diagnostic_unknown",
        );
  };
  return (
    <section className={compact ? "diagnostic-section" : "settings-section"}>
      <h2>{t("diagnosticsTitle")}</h2>
      {attempts.length ? (
        attempts.map((attempt) => (
          <article className="recovery-reason" key={attempt.kind}>
            <strong>{label("diagnosticKind", attempt.kind)}</strong>
            <p>{label("diagnostic", attempt.code)}</p>
            <small>{date(attempt.at, data.settings.language)}</small>
            {attempt.retryNotBefore && (
              <p className="helper">
                {t("retryNotBefore")}:{" "}
                {date(attempt.retryNotBefore, data.settings.language)}
              </p>
            )}
            <details>
              <summary>{t("diagnosticDetails")}</summary>
              <p className="path">{attempt.detail || "—"}</p>
            </details>
            <div className="button-row">
              {retryBlocked(attempt.kind) ? (
                <>
                  <span className="helper">{t("waitingToRetry")}</span>
                  <InfoHint text={t("rateLimitWait")} label={t("moreInfo")} />
                </>
              ) : ["engine_missing", "engine_update_required"].includes(
                  attempt.code,
                ) ||
                (attempt.kind === "engine_launch" && !data.installerReady) ? (
                <button className="secondary" disabled={busy} onClick={setup}>
                  {t("reinstall")}
                </button>
              ) : (
                <button
                  className="secondary"
                  disabled={
                    busy ||
                    !desktop ||
                    (attempt.kind === "engine_launch" && !data.installerReady)
                  }
                  onClick={() => void retry(attempt)}
                >
                  {t("retryOperation")}
                </button>
              )}
            </div>
          </article>
        ))
      ) : (
        <p className="helper">{t("noDiagnosticFailures")}</p>
      )}
      {!compact && (
        <>
          <div className="button-row">
            <button
              className="secondary"
              disabled={busy || !desktop}
              onClick={() => void run(() => command("export_diagnostics"))}
            >
              {t("exportDiagnostics")}
            </button>
            <InfoHint text={t("diagnosticsExportHelp")} label={t("moreInfo")} />
          </div>
        </>
      )}
    </section>
  );
}
