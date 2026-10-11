import Modal from "./Modal";
import type { FileState, Language, QuarantineDetail } from "../types";
import { date, isCopyKey } from "../i18n";
import type { T } from "./shared";
import { bytes } from "./DownloadCache";
export function statusLabel(status: string, t: T) {
  const key = `state_${status}`;
  return isCopyKey(key) ? t(key) : t("state_unknown");
}
function FileInfo({
  file,
  label,
  t,
}: {
  file: FileState;
  label: string;
  t: T;
}) {
  return (
    <div>
      <dt>{label}</dt>
      <dd>
        <span>
          {statusLabel(file.state, t)}
          {file.bytes !== null ? ` · ${bytes(file.bytes)}` : ""}
        </span>
        <p className="path">{file.path}</p>
      </dd>
    </div>
  );
}
export default function QuarantineDetails({
  detail,
  t,
  language,
  close,
  refresh,
  pending,
}: {
  detail: QuarantineDetail;
  t: T;
  language: Language;
  close: () => void;
  refresh: () => void;
  pending: boolean;
}) {
  return (
    <Modal title={t("quarantineDetails")} close={close} closeLabel={t("close")}>
      <p className="helper">{t("detailCaution")}</p>
      <dl className="detail-list">
        <div>
          <dt>{t("status")}</dt>
          <dd>{statusLabel(detail.item.status, t)}</dd>
        </div>
        <div>
          <dt>{t("detection")}</dt>
          <dd>{detail.item.signature}</dd>
        </div>
        <div>
          <dt>{t("quarantinedAt")}</dt>
          <dd>{date(detail.item.quarantinedAt, language)}</dd>
        </div>
        <div>
          <dt>{t("recordedHash")}</dt>
          <dd className="path">{detail.item.digest}</dd>
        </div>
        <FileInfo file={detail.original} label={t("originalFile")} t={t} />
        <FileInfo file={detail.stored} label={t("storedFile")} t={t} />
      </dl>
      <h3>{t("transferDetails")}</h3>
      {detail.transfers.length ? (
        detail.transfers.map((transfer, i) => (
          <section key={i} className="transfer-detail">
            <p>
              {transfer.kind === "restore" ? t("restore") : t("quarantine")} ·{" "}
              {transfer.published ? t("copyPublished") : t("copyUncommitted")}
            </p>
            <dl className="detail-list">
              <FileInfo
                file={transfer.source}
                label={t("transferSource")}
                t={t}
              />
              <FileInfo
                file={transfer.destination}
                label={t("transferDestination")}
                t={t}
              />
              <FileInfo
                file={transfer.staged}
                label={t("stagedSource")}
                t={t}
              />
              <FileInfo
                file={transfer.temporary}
                label={t("temporaryCopy")}
                t={t}
              />
            </dl>
          </section>
        ))
      ) : (
        <p>{t("noTransfer")}</p>
      )}
      <div className="modal-actions">
        <button className="secondary" disabled={pending} onClick={refresh}>
          {t("refreshDetails")}
        </button>
        <button className="primary" onClick={close}>
          {t("close")}
        </button>
      </div>
    </Modal>
  );
}
