import {
  FileText,
  Shield,
  Folder,
  HardDrive,
  ChevronRight,
  Check,
  Square,
  LoaderCircle,
} from "lucide-react";
import { useEffect, useState } from "react";
import { command, desktop } from "../bridge";
import { date, translate, isCopyKey, type CopyKey } from "../i18n";
import type { Snapshot, Page, Scan } from "../types";
export type T = (key: CopyKey) => string;
export type Run = (
  work: () => Promise<unknown>,
  success?: CopyKey,
) => Promise<boolean>;
export interface ViewProps {
  data: Snapshot;
  t: T;
  run: Run;
  pending: boolean;
  navigate: (page: Page) => void;
  setup: () => void;
}
export function Empty({
  title,
  description,
  icon = "file",
  children,
}: {
  title: string;
  description?: string;
  icon?: "file" | "shield";
  children?: React.ReactNode;
}) {
  const Icon = icon === "shield" ? Shield : FileText;
  return (
    <div className="empty">
      <Icon size={42} strokeWidth={1.5} />
      <strong>{title}</strong>
      {description && <p>{description}</p>}
      {children}
    </div>
  );
}
export function ScanChoices({ data, t, run, pending, navigate }: ViewProps) {
  const ready = !!data.engine.version && data.engine.definitions;
  const busy = pending || !!data.operation || data.scan?.status === "running";
  async function start(kind: string) {
    let target: string | null = null;
    if (kind === "custom") {
      target = await command<string | null>("choose_folder");
      if (!target) return;
    }
    await command("start_scan", { kind, target });
    navigate("scan");
  }
  return (
    <div className="scan-choices">
      {(["quick", "custom", "full"] as const).map((kind) => {
        const Icon = kind === "full" ? HardDrive : Folder;
        return (
          <button
            className="scan-choice"
            key={kind}
            disabled={!desktop || !ready || busy}
            onClick={() => void run(() => start(kind))}
          >
            <Icon className="accent" size={28} strokeWidth={1.6} />
            <span>
              <strong>{t(kind)}</strong>
              <small>{t(`${kind}Desc`)}</small>
            </span>
            <ChevronRight size={18} />
          </button>
        );
      })}
      {!ready && <p className="helper">{t("needsSetup")}</p>}
    </div>
  );
}
export function ScanResult({
  scan,
  data,
  t,
  run,
  pending,
}: {
  scan: Scan;
  data: Snapshot;
  t: T;
  run: Run;
  pending: boolean;
}) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => {
    if (scan.status !== "running") return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [scan.status]);
  const seconds = Math.max(
    0,
    Math.floor(
      ((scan.finishedAt ? Date.parse(scan.finishedAt) : now) -
        Date.parse(scan.startedAt)) /
        1000,
    ),
  );
  return (
    <section className="scan-result">
      <div className="section-heading">
        <h2>{isCopyKey(scan.status) ? t(scan.status) : scan.status}</h2>
        {scan.status === "running" && (
          <button
            className="secondary"
            disabled={pending}
            onClick={() => void run(() => command("stop_scan"))}
          >
            <Square size={14} />
            {pending ? t("stopping") : t("stop")}
          </button>
        )}
      </div>
      <p className="path">{scan.target}</p>
      <div className="scan-stats">
        {[
          [t("scanned"), scan.scanned],
          [t("detections"), scan.findings.length],
          [t("errors"), scan.errors],
          [
            t("elapsed"),
            `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`,
          ],
        ].map(([label, value]) => (
          <div key={label}>
            <small>{label}</small>
            <strong>{value}</strong>
          </div>
        ))}
      </div>
      {scan.status === "running" && (
        <div className="current-file">
          <LoaderCircle className="spin" size={18} />
          <span>{scan.currentFile || t("waiting")}</span>
        </div>
      )}
      {scan.message && (
        <details className="technical">
          <summary>{t("operationError")}</summary>
          <pre>{scan.message}</pre>
        </details>
      )}
      {scan.findings.length > 0 ? (
        <div className="findings">
          <h3>{t("review")}</h3>
          {scan.findings.map((f) => (
            <div className="finding" key={f.id}>
              <div>
                <strong>{f.signature}</strong>
                <p className="path">{f.path}</p>
              </div>
              {f.quarantined ? (
                <span className="positive">
                  <Check size={15} />
                  {t("quarantined")}
                </span>
              ) : (
                <button
                  className="secondary"
                  disabled={pending || !f.digest || scan.status === "running"}
                  onClick={() =>
                    void run(() => command("quarantine_file", { id: f.id }))
                  }
                >
                  {t("quarantineAction")}
                </button>
              )}
            </div>
          ))}
        </div>
      ) : (
        scan.status !== "running" && (
          <div className="scan-empty">
            <strong>{t("noDetections")}</strong>
            <p>{t("noDetectionsDesc")}</p>
          </div>
        )
      )}
      <p className="helper">
        {date(scan.startedAt, data.settings.language)} · {t("onDemand")}
      </p>
    </section>
  );
}
export function getText(data: Snapshot): T {
  return (key) => translate(data.settings.language, key);
}
