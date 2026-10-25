import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { command, desktop } from "../bridge";
import { isCopyKey } from "../i18n";
import type { ViewProps } from "./shared";
import InfoHint from "./InfoHint";
import Modal from "./Modal";
interface Status {
  currentVersion: string; phase: string; version: string | null; notes: string | null;
  checkedAt: string | null; downloaded: number; total: number | null; errorCode: string | null; detail: string | null;
}
export default function AppUpdater({ data, t, run, pending }: ViewProps) {
  const [state, setState] = useState<Status | null>(null);
  const [confirmVersion, setConfirmVersion] = useState<string | null>(null);
  useEffect(() => {
    if (!desktop) return;
    let disposed = false;
    let revision = 0;
    let cleanup: (() => void) | undefined;
    void (async () => {
      cleanup = await listen<Status>("app:update", (e) => { revision++; if (!disposed) setState(e.payload); });
      if (disposed) { cleanup(); return; }
      const before = revision;
      const next = await command<Status>("get_app_update");
      if (!disposed && revision === before) setState(next);
    })().catch(() => { cleanup?.(); });
    return () => { disposed = true; cleanup?.(); };
  }, []);
  const phase = state?.phase ?? "idle";
  const label = `appUpdate_${phase}`;
  const errorLabel = `appUpdateError_${state?.errorCode ?? "network_or_install"}`;
  const updating = ["checking", "downloading", "installing"].includes(phase);
  const busy = pending || updating || data.scan?.status === "running" || (!!data.operation && data.operation.kind !== "app_update");
  const perform = (name: "check_app_update" | "install_app_update") => void run(async () => { setState(await command<Status>(name)); });
  return (
    <section className="settings-section app-update-section" aria-live="polite">
      <div className="button-row"><h2>{t("appUpdateTitle")}</h2><InfoHint label={t("moreInfo")} text={t("appUpdateHelp")} /></div>
      <dl><div><dt>{t("appUpdateCurrent")}</dt><dd>{state?.currentVersion ?? "—"}</dd></div></dl>
      <p>{isCopyKey(label) ? t(label) : t("appUpdate_error")}{state?.version ? ` · ${state.version}` : ""}</p>
      {phase === "error" && <p role="alert">{isCopyKey(errorLabel) ? t(errorLabel) : t("appUpdate_error")}</p>}
      {state?.detail && <details><summary>{t("operationError")}</summary><pre>{state.detail}</pre></details>}
      {state?.notes && <details><summary>{t("appUpdateNotes")}</summary><pre>{state.notes}</pre></details>}
      {phase === "downloading" && <><progress aria-label={t("appUpdate_downloading")} value={state?.total ? state.downloaded : undefined} max={state?.total ?? undefined} /><p>{((state?.downloaded ?? 0) / 1048576).toFixed(2)} MiB{state?.total ? ` / ${(state.total / 1048576).toFixed(2)} MiB` : ""}</p></>}
      <div className="button-row">
        <button type="button" className="secondary" disabled={!desktop || busy || phase === "installed"} onClick={() => perform("check_app_update")}>{t("appUpdateCheck")}</button>
        {phase === "available" && <button type="button" className="primary" disabled={busy} onClick={() => setConfirmVersion(state?.version ?? null)}>{t("appUpdateInstall")}</button>}
        {phase === "installed" && <button type="button" className="primary" disabled={pending} onClick={() => void run(() => command("restart_app_update"))}>{t("appUpdateRestart")}</button>}
      </div>
      {confirmVersion && phase === "available" && state?.version === confirmVersion && (
        <Modal title={t("appUpdateInstall")} close={() => setConfirmVersion(null)} closeLabel={t("cancel")}>
          <p>{state.currentVersion} → {confirmVersion}</p>
          <p>{t("appUpdateConfirm")}</p>
          <div className="modal-actions">
            <button type="button" className="secondary" onClick={() => setConfirmVersion(null)}>{t("cancel")}</button>
            <button type="button" className="primary" disabled={busy} onClick={() => { setConfirmVersion(null); perform("install_app_update"); }}>{t("appUpdateInstall")}</button>
          </div>
        </Modal>
      )}
    </section>
  );
}
