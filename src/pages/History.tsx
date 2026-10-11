import { Search, Download, ChevronRight } from "lucide-react";
import { useState } from "react";
import { command, desktop } from "../bridge";
import { date, isCopyKey } from "../i18n";
import { Empty, ScanResult, type ViewProps } from "../components/shared";
export default function History(props: ViewProps) {
  const { data, t, run, pending, navigate } = props;
  const [query, setQuery] = useState("");
  const [id, setId] = useState<string | null>(null);
  const rows = data.history.filter((s) =>
    `${s.target} ${s.findings.map((f) => f.signature).join(" ")}`
      .toLowerCase()
      .includes(query.toLowerCase()),
  );
  const selected = data.history.find((s) => s.id === id);
  return (
    <>
      <header className="page-heading">
        <h1>{t("historyTitle")}</h1>
        <p>{t("historyDesc")}</p>
      </header>
      <div className="history-toolbar">
        <label className="search-field">
          <Search size={18} />
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            aria-label={t("search")}
            placeholder={t("search")}
          />
        </label>
        <button
          className="secondary"
          disabled={pending || !desktop || data.history.length === 0}
          onClick={() => void run(() => command("export_history"))}
        >
          <Download size={17} />
          {t("export")}
        </button>
      </div>
      <section className="panel">
        {data.history.length === 0 ? (
          <Empty title={t("emptyHistory")} description={t("emptyHistoryDesc")}>
            <button className="secondary" onClick={() => navigate("scan")}>
              {t("startScan")}
            </button>
          </Empty>
        ) : rows.length === 0 ? (
          <Empty title={t("noMatches")} />
        ) : (
          <div className="history-list">
            {rows.map((scan) => (
              <button
                className={`history-row ${id === scan.id ? "selected" : ""}`}
                key={scan.id}
                onClick={() => setId(id === scan.id ? null : scan.id)}
                aria-label={`${t("selectHistory")}: ${scan.target}`}
                aria-expanded={id === scan.id}
              >
                <div>
                  <strong>
                    {isCopyKey(scan.kind) ? t(scan.kind) : scan.kind}
                  </strong>
                  <span className="path">{scan.target}</span>
                  <small>{date(scan.startedAt, data.settings.language)}</small>
                </div>
                <div className="history-summary">
                  <span
                    className={
                      scan.status === "completed" && scan.findings.length === 0
                        ? "positive"
                        : ""
                    }
                  >
                    {isCopyKey(scan.status) ? t(scan.status) : scan.status}
                  </span>
                  <small>
                    {t("detections")}: {scan.findings.length} · {t("errors")}:{" "}
                    {scan.errors}
                  </small>
                </div>
                <ChevronRight size={18} />
              </button>
            ))}
          </div>
        )}
      </section>
      {selected && (
        <div className="panel spaced">
          <ScanResult {...props} scan={selected} />
        </div>
      )}
    </>
  );
}
