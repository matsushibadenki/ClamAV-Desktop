import {
  ExternalLink,
  PanelRightClose,
  ArrowLeftToLine,
  ArrowRightToLine,
} from "lucide-react";
import { command, desktop } from "../bridge";
import { date, isCopyKey } from "../i18n";
import { Empty, type ViewProps } from "./shared";
export default function ActivityPanel({
  data,
  t,
  run,
  pending,
  floating = false,
}: ViewProps & { floating?: boolean }) {
  return (
    <aside className={`panel activity-panel ${floating ? "floating" : ""}`}>
      <header
        className="section-heading"
        draggable={!floating && desktop}
        onDragStart={(e) =>
          e.dataTransfer.setData("text/plain", "clamav-activity")
        }
      >
        <h2>{t("activity")}</h2>
        <div className="panel-tools">
          {desktop && (
            <>
              <button
                className="icon-button"
                aria-label={floating ? t("dock") : t("float")}
                title={floating ? t("dock") : t("float")}
                disabled={pending}
                onClick={() =>
                  void run(() =>
                    command("set_activity_panel", {
                      floating: !floating,
                      side: null,
                    }),
                  )
                }
              >
                {floating ? (
                  <PanelRightClose size={17} />
                ) : (
                  <ExternalLink size={17} />
                )}
              </button>
              {!floating && (
                <button
                  className="icon-button"
                  aria-label={
                    data.activityDock === "right"
                      ? t("dockLeft")
                      : t("dockRight")
                  }
                  title={
                    data.activityDock === "right"
                      ? t("dockLeft")
                      : t("dockRight")
                  }
                  onClick={() =>
                    void run(() =>
                      command("set_activity_panel", {
                        floating: false,
                        side: data.activityDock === "right" ? "left" : "right",
                      }),
                    )
                  }
                >
                  {data.activityDock === "right" ? (
                    <ArrowLeftToLine size={17} />
                  ) : (
                    <ArrowRightToLine size={17} />
                  )}
                </button>
              )}
            </>
          )}
        </div>
      </header>
      {data.activity.length === 0 ? (
        <Empty title={t("noActivity")} description={t("noActivityDesc")} />
      ) : (
        <ol className="activity-list">
          {data.activity.map((item, index) => (
            <li key={`${item.at}-${index}`}>
              <span
                className={`activity-dot ${item.code === "error" ? "danger" : ""}`}
              />
              <div>
                <strong>
                  {isCopyKey(item.code) ? t(item.code) : item.code}
                </strong>
                <time>{date(item.at, data.settings.language)}</time>
                {item.detail && <p>{item.detail}</p>}
              </div>
            </li>
          ))}
        </ol>
      )}
    </aside>
  );
}
