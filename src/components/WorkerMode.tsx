import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ArrowRightLeft, Monitor, Server, Power, RefreshCw, Wrench, ExternalLink, RotateCw } from "lucide-react";
import { command, desktop } from "../bridge";
import type { ViewProps } from "./shared";
import InfoHint from "./InfoHint";

interface Status {
  mode: "local" | "worker" | "switching";
  transitioning: boolean;
}

interface LoginStatus {
  available: boolean;
  registration: "absent" | "matches" | "conflict" | "repairable" | null;
  reason: string | null;
  os?: { permission: "enabled" | "requires_approval" | "not_registered" | "not_found" | "unknown"; loaded: boolean | null; permissionSupported: boolean; queryFailed: boolean } | null;
}
const permissionLabels = {
  enabled: "loginPermissionEnabled", requires_approval: "loginPermissionApproval",
  not_registered: "loginPermissionNotRegistered", not_found: "loginPermissionNotFound", unknown: "loginPermissionUnknown",
} as const;

export default function WorkerMode({ data, t, run, pending }: ViewProps) {
  const [status, setStatus] = useState<Status | null>(null);
  const [login, setLogin] = useState<LoginStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const mounted = useRef(true);
  const request = useRef<Promise<void> | null>(null);
  const refreshLogin = useCallback((): Promise<void> => {
    if (request.current) return request.current;
    setChecking(true);
    const work = command<LoginStatus>("get_login_service").then((next) => {
      if (mounted.current) setLogin(next);
    }).catch((error: unknown) => {
      if (mounted.current) setLogin({ available: false, registration: null, reason: String(error) });
      throw error;
    }).finally(() => {
      request.current = null;
      if (mounted.current) setChecking(false);
    });
    request.current = work;
    return work;
  }, []);
  async function refreshAfterChange() {
    await request.current?.catch(() => {});
    await Promise.allSettled([refreshLogin(), command<Status>("get_backend_status").then((next) => { if (mounted.current) setStatus(next); })]);
  }
  useEffect(() => {
    if (!desktop) return;
    mounted.current = true;
    const focus = () => { void refreshLogin().catch(() => {}); };
    const visibility = () => { if (document.visibilityState === "visible") focus(); };
    window.addEventListener("focus", focus);
    document.addEventListener("visibilitychange", visibility);
    let disposed = false;
    let revision = 0;
    let cleanup: (() => void) | undefined;
    void (async () => {
      cleanup = await listen<Status>("core:backend", ({ payload }) => {
        revision++;
        if (!disposed) setStatus(payload);
      });
      if (disposed) { cleanup(); return; }
      const before = revision;
      const [backend, service] = await Promise.allSettled([
        command<Status>("get_backend_status"), refreshLogin(),
      ]);
      if (disposed) return;
      if (service.status === "rejected") setLogin({ available: false, registration: null, reason: String(service.reason) });
      if (backend.status === "fulfilled" && revision === before) setStatus(backend.value);
    })().catch(() => { cleanup?.(); });
    return () => { disposed = true; mounted.current = false; cleanup?.(); window.removeEventListener("focus", focus); document.removeEventListener("visibilitychange", visibility); };
  }, [refreshLogin]);
  const worker = status?.mode === "worker";
  const switching = status?.transitioning || status?.mode === "switching";
  // Returning to the app remains available after a disconnected worker leaves
  // stale progress. Rust atomically rejects live jobs before stopping a worker.
  const busy = pending || switching || (!worker && (!!data.operation || data.scan?.status === "running"));
  const Icon = worker ? Server : Monitor;
  const registered = login?.registration === "matches";
  const repairable = login?.registration === "repairable";
  const manualBlocked = !login || login.registration !== "absent" && login.available;
  return (
    <section className="settings-section" aria-live="polite">
      <div className="button-row">
        <h2>{t("workerTitle")}</h2>
        <InfoHint text={t("workerHelp")} label={t("moreInfo")} />
      </div>
      <div className="button-row">
        <Icon size={18} aria-hidden="true" />
        <span>{switching ? t("workerSwitching") : !status ? "—" : worker ? t("workerMode") : t("localMode")}</span>
        <button
          type="button"
          className="secondary"
          disabled={!desktop || !status || busy || manualBlocked}
          onClick={() => void run(async () => {
            setStatus(await command<Status>("set_worker_mode", { worker: !worker }));
          })}
        >
          <ArrowRightLeft size={16} aria-hidden="true" />
          {worker ? t("useLocal") : t("useWorker")}
        </button>
      </div>
      <div className="button-row login-service-row">
        <Power size={18} aria-hidden="true" />
        <span>{t("loginStartup")}</span>
        <InfoHint text={t("loginHelp")} label={t("moreInfo")} />
        <span className="muted">{!login ? "—" : !login.available ? t("loginUnavailable") : registered ? t("loginRegistered") : repairable ? t("loginMoved") : login.registration === "conflict" ? t("loginConflict") : t("loginOff")}</span>
        <button type="button" className="secondary"
          disabled={!desktop || !login?.available || !status || busy || checking || login.registration === "conflict"}
          onClick={() => void run(async () => {
            try { setStatus(await command<Status>(repairable ? "repair_login_service" : "set_login_service", repairable ? undefined : { enable: !registered })); }
            finally {
              await refreshAfterChange();
            }
          })}>
          {repairable ? <Wrench size={16} aria-hidden="true" /> : <Power size={16} aria-hidden="true" />}{repairable ? t("loginRepair") : registered ? t("loginDisable") : t("loginEnable")}
        </button>
        {registered && <button type="button" className="secondary" disabled={!desktop || !status || busy || checking || login?.os?.permission === "requires_approval"}
          onClick={() => void run(async () => {
            try { setStatus(await command<Status>("recover_login_service")); }
            finally { await refreshAfterChange(); }
          })}>
          <RotateCw size={16} aria-hidden="true" />{t("loginRecover")}
        </button>}
        <button type="button" className="secondary icon-button" aria-label={t("loginRefresh")} title={t("loginRefresh")}
          disabled={!desktop || busy || checking} onClick={() => void run(refreshLogin)}>
          <RefreshCw size={16} aria-hidden="true" />
        </button>
      </div>
      {login && !login.available && <p className="muted">{t("loginInstallRequired")}</p>}
      <div className="button-row login-os-state">
        <span className="muted">{t("loginPermission")}: {t(permissionLabels[login?.os?.permission ?? "unknown"])}</span>
        <span className="muted">{t("loginLoading")}: {login?.os?.loaded === true ? t("loginLoaded") : login?.os?.loaded === false ? t("loginUnloaded") : t("loginLoadUnknown")}</span>
        <InfoHint text={t("loginOsHelp")} label={t("moreInfo")} />
        {login?.os?.permission === "requires_approval" && <button type="button" className="secondary" disabled={busy}
          onClick={() => void run(() => command("open_login_settings"))}>
          <ExternalLink size={16} aria-hidden="true" />{t("loginSettings")}
        </button>}
      </div>
      {registered && <InfoHint text={t("loginRecoveryHelp")} label={t("moreInfo")} />}
      {login?.os?.permission === "requires_approval" && <p className="muted">{t("loginApprovalHelp")}</p>}
      {login?.os?.queryFailed && <p role="status" className="muted">{t("loginQueryFailed")}</p>}
      {repairable && <p className="muted">{t("loginRepairHelp")}</p>}
    </section>
  );
}
