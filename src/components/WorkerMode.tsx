import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ArrowRightLeft, Monitor, Server } from "lucide-react";
import { command, desktop } from "../bridge";
import type { ViewProps } from "./shared";
import InfoHint from "./InfoHint";

interface Status {
  mode: "local" | "worker" | "switching";
  transitioning: boolean;
}

export default function WorkerMode({ data, t, run, pending }: ViewProps) {
  const [status, setStatus] = useState<Status | null>(null);
  useEffect(() => {
    if (!desktop) return;
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
      const next = await command<Status>("get_backend_status");
      if (!disposed && revision === before) setStatus(next);
    })().catch(() => { cleanup?.(); });
    return () => { disposed = true; cleanup?.(); };
  }, []);
  const worker = status?.mode === "worker";
  const switching = status?.transitioning || status?.mode === "switching";
  // Returning to the app remains available after a disconnected worker leaves
  // stale progress. Rust atomically rejects live jobs before stopping a worker.
  const busy = pending || switching || (!worker && (!!data.operation || data.scan?.status === "running"));
  const Icon = worker ? Server : Monitor;
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
          disabled={!desktop || !status || busy}
          onClick={() => void run(async () => {
            setStatus(await command<Status>("set_worker_mode", { worker: !worker }));
          })}
        >
          <ArrowRightLeft size={16} aria-hidden="true" />
          {worker ? t("useLocal") : t("useWorker")}
        </button>
      </div>
    </section>
  );
}
