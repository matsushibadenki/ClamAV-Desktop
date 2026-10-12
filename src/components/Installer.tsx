import InfoHint from "./InfoHint";
import useRetryBlock from "../useRetryBlock";
import { CheckCircle2, Download, ShieldCheck } from "lucide-react";
import { command, desktop } from "../bridge";
import type { ViewProps } from "./shared";
import { isCopyKey } from "../i18n";
import Modal from "./Modal";
import Diagnostics from "./Diagnostics";
export default function Installer({
  data,
  t,
  run,
  pending,
  setup,
  navigate,
  close,
}: ViewProps & { close: () => void }) {
  const retryBlocked = useRetryBlock(data.diagnostics);
  return (
    <Modal title={t("installerTitle")} close={close} closeLabel={t("dismiss")}>
      <InfoHint text={t("installerDesc")} label={t("moreInfo")} />
      <div className="installer-flow">
        <Download size={20} />
        <span>{t("installerSteps")}</span>
      </div>
      <p className="helper">ClamAV 1.5.4 · Universal PKG · arm64 / x86_64</p>
      {data.engine.path && <p className="notice">{t("existingEngine")}</p>}
      <InfoHint text={t("installerAuth")} label={t("moreInfo")} />
      {data.operation?.kind === "installer" && (
        <div className="installer-progress" role="status">
          <strong>
            {isCopyKey(data.operation.stage)
              ? t(data.operation.stage)
              : data.operation.stage}
          </strong>
          {data.operation.stage === "downloading" && (
            <>
              <p>
                {(data.operation.downloaded / 1024 / 1024).toFixed(1)} MB
                {data.operation.total
                  ? ` / ${(data.operation.total / 1024 / 1024).toFixed(1)} MB`
                  : ""}
              </p>
              {data.operation.total && (
                <progress
                  aria-label={t("downloading")}
                  value={data.operation.downloaded}
                  max={data.operation.total}
                />
              )}
            </>
          )}
        </div>
      )}
      {data.installerReady && (
        <p className="positive">
          <CheckCircle2 size={18} />
          {t("verified")}
        </p>
      )}
      <Diagnostics
        data={data}
        t={t}
        run={run}
        pending={pending}
        setup={setup}
        navigate={navigate}
        compact
        kinds={["engine_install", "engine_launch"]}
      />
      <div className="modal-actions">
        <button className="secondary" onClick={close}>
          {t("cancel")}
        </button>
        <button
          className="primary"
          disabled={
            pending ||
            !!data.operation ||
            !desktop ||
            (!data.installerReady && retryBlocked("engine_install"))
          }
          onClick={() =>
            void run(() =>
              command(
                data.installerReady ? "open_installer" : "prepare_installer",
              ),
            ).then((ok) => {
              if (ok && data.installerReady) close();
            })
          }
        >
          <ShieldCheck size={18} />
          {t(
            data.installerReady
              ? "openInstaller"
              : pending
                ? "working"
                : "downloadVerify",
          )}
        </button>
      </div>
    </Modal>
  );
}
