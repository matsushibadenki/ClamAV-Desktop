import IntegrityProgress from "./IntegrityProgress";
import Modal from "./Modal";
import type { FileState, Language, QuarantineDetail, Snapshot } from "../types";
import { date, isCopyKey } from "../i18n";
import type { T } from "./shared";
import { bytes } from "./DownloadCache";
export function statusLabel(status: string, t: T) {
  const key = `state_${status}`;
  return isCopyKey(key) ? t(key) : t("state_unknown");
}
function recoveryReason(reason: string, t: T) {
  if (
    reason.includes("transfer_source_changed") ||
    reason.includes("transfer_destination_changed") ||
    reason.includes("transfer_temporary_changed")
  )
    return t("recoveryChanged");
  if (reason.includes("transfer_needs_review")) return t("recoveryAmbiguous");
  if (reason.includes("file_move_failed")) return t("recoveryConflict");
  if (reason.includes("symlink")) return t("recoveryLink");
  if (
    reason.includes("transfer_source_unavailable") ||
    reason.includes("os error 2)")
  )
    return t("recoveryUnavailable");
  return t("recoveryAccess");
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
  verify,
  recover,
  operation,
}: {
  detail: QuarantineDetail;
  t: T;
  language: Language;
  close: () => void;
  refresh: () => void;
  pending: boolean;
  verify: () => void;
  recover: () => void;
  operation: Snapshot["operation"];
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
      <section className="transfer-detail">
        <h3>{t("integrityTitle")}</h3>
        <p className="helper">{t("integrityHelp")}</p>
        {detail.integrity ? (
          <dl className="detail-list">
            <div>
              <dt>{t("status")}</dt>
              <dd>{statusLabel(detail.integrity.state, t)}</dd>
            </div>
            <div>
              <dt>{t("lastIntegrityCheck")}</dt>
              <dd>{date(detail.integrity.checkedAt, language)}</dd>
            </div>
            {detail.integrity.actualDigest && (
              <div>
                <dt>{t("actualHash")}</dt>
                <dd className="path">{detail.integrity.actualDigest}</dd>
              </div>
            )}
            {detail.integrity.totalBytes !== null && (
              <div>
                <dt>{t("checkedBytes")}</dt>
                <dd>
                  {bytes(detail.integrity.bytesChecked)} /{" "}
                  {bytes(detail.integrity.totalBytes)}
                </dd>
              </div>
            )}
            {detail.integrity.error && (
              <div>
                <dt>{t("diagnosticDetails")}</dt>
                <dd className="path">{detail.integrity.error}</dd>
              </div>
            )}
          </dl>
        ) : (
          <p>{t("integrityNotChecked")}</p>
        )}
        {operation?.kind === "integrity" &&
          operation.targetId === detail.item.id && (
            <IntegrityProgress operation={operation} t={t} />
          )}
        <button
          className="secondary"
          disabled={pending || !!operation}
          onClick={verify}
        >
          {t("verifyIntegrity")}
        </button>
      </section>
      <h3>{t("transferDetails")}</h3>
      {detail.transfers.length ? (
        detail.transfers.map((transfer, i) => (
          <section key={i} className="transfer-detail">
            <p>
              {transfer.kind === "restore" ? t("restore") : t("quarantine")} ·{" "}
              {transfer.published ? t("copyPublished") : t("copyUncommitted")}
            </p>
            {transfer.lastError && (
              <div className="recovery-reason" role="status">
                <strong>{t("recoveryReason")}</strong>
                <p>{recoveryReason(transfer.lastError, t)}</p>
                {transfer.attemptedAt && (
                  <small>{date(transfer.attemptedAt, language)}</small>
                )}
                <details>
                  <summary>{t("diagnosticDetails")}</summary>
                  <p className="path">{transfer.lastError}</p>
                </details>
              </div>
            )}
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
      {detail.transfers.length > 0 && (
        <button className="secondary" disabled={pending} onClick={recover}>
          {t("retryRecovery")}
        </button>
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
