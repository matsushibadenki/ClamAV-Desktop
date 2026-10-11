import { Archive, RotateCcw } from "lucide-react";
import { quarantineList, type QuarantineSort } from "../quarantine-list";
import { useState } from "react";
import { command } from "../bridge";
import { date } from "../i18n";
import { Empty, type ViewProps } from "../components/shared";
import Modal from "../components/Modal";
import QuarantineDetails, {
  statusLabel,
} from "../components/QuarantineDetails";
import type { QuarantineDetail, QuarantineItem } from "../types";
export default function Quarantine({
  data,
  t,
  run,
  pending,
  navigate,
}: ViewProps) {
  const [restore, setRestore] = useState<QuarantineItem | null>(null);
  const [detail, setDetail] = useState<QuarantineDetail | null>(null);
  const [query, setQuery] = useState("");
  const [sort, setSort] = useState<QuarantineSort>("newest");
  const [showCompleted, setShowCompleted] = useState(false);
  const showDetail = (id: string) =>
    run(async () =>
      setDetail(
        await command<QuarantineDetail>("get_quarantine_detail", { id }),
      ),
    );
  const items = quarantineList(
    data.quarantine,
    query,
    showCompleted,
    sort,
    (status) => statusLabel(status, t),
  );
  return (
    <>
      <header className="page-heading">
        <h1>{t("quarantineTitle")}</h1>
        <p>{t("quarantineDesc")}</p>
      </header>
      <div className="quarantine-filters">
        <label className="field">
          <span>{t("searchQuarantine")}</span>
          <input
            type="search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("searchQuarantineHint")}
          />
        </label>
        <label className="field">
          <span>{t("sortQuarantine")}</span>
          <select
            value={sort}
            onChange={(e) => setSort(e.target.value as QuarantineSort)}
          >
            <option value="newest">{t("sortNewest")}</option>
            <option value="oldest">{t("sortOldest")}</option>
            <option value="path">{t("sortPath")}</option>
            <option value="attention">{t("sortAttention")}</option>
          </select>
        </label>
      </div>
      <p className="helper" role="status">
        {t("matchingRecords")}: {items.length}
      </p>
      <label className="completed-toggle">
        <input
          type="checkbox"
          checked={showCompleted}
          onChange={(e) => setShowCompleted(e.target.checked)}
        />
        {t("showCompleted")}
      </label>
      <section className="panel">
        {items.length === 0 ? (
          <Empty
            icon="shield"
            title={t(query.trim() ? "noSearchResults" : "emptyQuarantine")}
            description={t(
              query.trim() ? "searchQuarantineHint" : "emptyQuarantineDesc",
            )}
          >
            {query.trim() ? (
              <button className="secondary" onClick={() => setQuery("")}>
                {t("clearSearch")}
              </button>
            ) : (
              <button className="secondary" onClick={() => navigate("scan")}>
                {t("startScan")}
              </button>
            )}
          </Empty>
        ) : (
          <div className="quarantine-list">
            {items.map((item) => (
              <article className="quarantine-row" key={item.id}>
                <Archive className="accent" size={24} />
                <div>
                  <strong>{item.signature}</strong>
                  <p className="path">{item.originalPath}</p>
                  <small>
                    {date(item.quarantinedAt, data.settings.language)}
                    {` · ${statusLabel(item.status, t)}`}
                  </small>
                </div>
                <div className="quarantine-actions">
                  <button
                    className="secondary"
                    disabled={pending}
                    onClick={() => void showDetail(item.id)}
                  >
                    {t("details")}
                  </button>
                  {item.status === "transfer_pending" ? (
                    <div>
                      <p className="helper">{t("transferPending")}</p>
                      <button
                        className="secondary"
                        disabled={pending}
                        onClick={() =>
                          void run(() => command("recover_file_transfers"))
                        }
                      >
                        {t("retryRecovery")}
                      </button>
                    </div>
                  ) : ["active", "review"].includes(item.status) ? (
                    <button
                      className="secondary"
                      disabled={
                        pending || !["active", "review"].includes(item.status)
                      }
                      onClick={() => setRestore(item)}
                    >
                      <RotateCcw size={15} />
                      {t("restore")}
                    </button>
                  ) : null}
                </div>
              </article>
            ))}
          </div>
        )}
      </section>
      {detail && (
        <QuarantineDetails
          detail={detail}
          t={t}
          language={data.settings.language}
          close={() => setDetail(null)}
          refresh={() => void showDetail(detail.item.id)}
          pending={pending}
        />
      )}
      {restore && (
        <Modal
          title={t("restoreTitle")}
          close={() => setRestore(null)}
          closeLabel={t("cancel")}
        >
          <p>{t("restoreDesc")}</p>
          <p className="path">{restore.originalPath}</p>
          <div className="modal-actions">
            <button className="secondary" onClick={() => setRestore(null)}>
              {t("cancel")}
            </button>
            <button
              className="primary"
              disabled={pending}
              onClick={() =>
                void run(() =>
                  command("restore_file", { id: restore.id }),
                ).then((ok) => {
                  if (ok) setRestore(null);
                })
              }
            >
              {t("restore")}
            </button>
          </div>
        </Modal>
      )}
    </>
  );
}
