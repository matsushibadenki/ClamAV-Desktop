import {
  Shield,
  ShieldCheck,
  Settings2,
  Download,
  Clock,
  Info,
} from "lucide-react";
import { command, desktop } from "../bridge";
import { date } from "../i18n";
import { ScanChoices, type ViewProps } from "../components/shared";
import ActivityPanel from "../components/ActivityPanel";
export default function Home(props: ViewProps) {
  const { data, t, run, pending, setup, navigate } = props;
  const installed = !!data.engine.version;
  const ready = installed && data.engine.definitions;
  const Icon = ready ? ShieldCheck : Shield;
  return (
    <>
      <header className="page-heading">
        <h1>{t("homeTitle")}</h1>
        <p>{t("homeDesc")}</p>
      </header>
      <section className="panel readiness">
        <div className="readiness-content">
          <Icon size={72} className="accent" strokeWidth={1.6} />
          <h2>
            {t(
              ready
                ? "readyTitle"
                : installed
                  ? "definitionsTitle"
                  : "setupTitle",
            )}
          </h2>
          <p>
            {t(
              ready ? "readyDesc" : installed ? "definitionsDesc" : "setupDesc",
            )}
          </p>
          <button
            className="primary"
            disabled={
              pending ||
              !!data.operation ||
              !data.engine.checked ||
              data.scan?.status === "running"
            }
            onClick={() =>
              ready
                ? navigate("scan")
                : installed
                  ? void run(() => command("update_definitions"))
                  : setup()
            }
          >
            {t(ready ? "startScan" : installed ? "update" : "setup")}
          </button>
        </div>
        <div className="engine-stats">
          {[
            {
              icon: Settings2,
              title: "engine" as const,
              value: data.engine.checked
                ? data.engine.version?.split("/")[0] || t("notInstalled")
                : t("checking"),
            },
            {
              icon: Download,
              title: "definitions" as const,
              value: data.engine.definitions
                ? t("available")
                : t("notDownloaded"),
            },
            {
              icon: Clock,
              title: "lastScan" as const,
              value: data.history[0]
                ? date(data.history[0].startedAt, data.settings.language)
                : t("noScans"),
            },
          ].map(({ icon: StatIcon, title, value }) => (
            <div key={title}>
              <StatIcon size={24} strokeWidth={1.7} />
              <span>
                <strong>{t(title)}</strong>
                <small>{value}</small>
              </span>
            </div>
          ))}
        </div>
      </section>
      <div
        className={`home-lower dock-${data.activityDock}`}
        onDragOver={(e) => {
          if (desktop) e.preventDefault();
        }}
        onDrop={(e) => {
          if (e.dataTransfer.getData("text/plain") === "clamav-activity") {
            e.preventDefault();
            const box = e.currentTarget.getBoundingClientRect();
            void run(() =>
              command("set_activity_panel", {
                floating: false,
                side: e.clientX - box.left < box.width / 2 ? "left" : "right",
              }),
            );
          }
        }}
      >
        <section className="panel scan-panel">
          <h2>{t("startScan")}</h2>
          <ScanChoices {...props} />
        </section>
        {!data.activityFloating ? (
          <ActivityPanel {...props} />
        ) : (
          <div className="panel floating-placeholder">
            <p>{t("panelFloating")}</p>
            <button
              className="secondary"
              onClick={() =>
                void run(() =>
                  command("set_activity_panel", {
                    floating: false,
                    side: null,
                  }),
                )
              }
            >
              {t("dock")}
            </button>
          </div>
        )}
      </div>
      <footer className="page-footer">
        <Info size={17} />
        <span>{t("footer")}</span>
      </footer>
    </>
  );
}
