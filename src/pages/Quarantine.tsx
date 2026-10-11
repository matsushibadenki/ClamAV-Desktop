import { Archive, RotateCcw } from "lucide-react";
import { useState } from "react";
import { command } from "../bridge";
import { date } from "../i18n";
import { Empty, type ViewProps } from "../components/shared";
import Modal from "../components/Modal";
import type { QuarantineItem } from "../types";
export default function Quarantine({
  data,
  t,
  run,
  pending,
  navigate,
}: ViewProps) {
  const [restore, setRestore] = useState<QuarantineItem | null>(null);
  const items = data.quarantine.filter(
    (q) => q.status !== "restored" && q.status !== "move_failed",
  );
  return (
    <>
      <header className="page-heading">
        <h1>{t("quarantineTitle")}</h1>
        <p>{t("quarantineDesc")}</p>
      </header>
      <section className="panel">
        {items.length === 0 ? (
          <Empty
            icon="shield"
            title={t("emptyQuarantine")}
            description={t("emptyQuarantineDesc")}
          >
            <button className="secondary" onClick={() => navigate("scan")}>
              {t("startScan")}
            </button>
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
                    {item.status !== "active" && ` · ${t("review")}`}
                  </small>
                </div>
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
              </article>
            ))}
          </div>
        )}
      </section>
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
