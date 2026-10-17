import IntegrityProgress from "./components/IntegrityProgress";
import { useEffect, useState, useCallback } from "react";
import { createPortal } from "react-dom";
import {
  Home as HomeIcon,
  Search,
  Archive,
  Clock,
  Settings2,
  Shield,
  Globe2,
  RefreshCw,
  X,
  LoaderCircle,
  AlertCircle,
} from "lucide-react";
import { command, connect, desktop, useCore } from "./bridge";
import { getText, type Run, type ViewProps } from "./components/shared";
import { isCopyKey, type CopyKey } from "./i18n";
import type { Page, Language } from "./types";
import Home from "./pages/Home";
import Scanner from "./pages/Scanner";
import Quarantine from "./pages/Quarantine";
import History from "./pages/History";
import Settings from "./pages/Settings";
import ActivityPanel from "./components/ActivityPanel";
import Installer from "./components/Installer";
const pages = [
  { id: "home", icon: HomeIcon },
  { id: "scan", icon: Search },
  { id: "quarantine", icon: Archive },
  { id: "history", icon: Clock },
  { id: "settings", icon: Settings2 },
] as const;
export default function App() {
  const data = useCore();
  const t = getText(data);
  const [page, setPage] = useState<Page>("home");
  const [pending, setPending] = useState(false);
  const [installer, setInstaller] = useState(false);
  const [notice, setNotice] = useState<{
    key: CopyKey;
    detail?: string;
    error?: boolean;
  } | null>(null);
  const floating =
    new URLSearchParams(window.location.search).get("panel") === "activity";
  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | undefined;
    void connect((connected) => {
      if (disposed) return;
      if (!connected) setNotice({ key: "workerDisconnected", error: true });
      else setNotice((previous) => previous?.key === "workerDisconnected" ? null : previous);
    })
      .then((fn) => {
        if (disposed) fn();
        else cleanup = fn;
      })
      .catch((e) =>
        setNotice({ key: "operationError", detail: String(e), error: true }),
      );
    return () => {
      disposed = true;
      cleanup?.();
    };
  }, []);
  useEffect(() => {
    document.documentElement.lang = data.settings.language;
  }, [data.settings.language]);
  const run: Run = useCallback(async (work, success) => {
    setPending(true);
    try {
      await work();
      if (success) setNotice({ key: success });
      return true;
    } catch (e) {
      const detail = e instanceof Error ? e.message : String(e);
      const key = detail.includes("worker_prepared_installer")
        ? "workerPreparedInstaller"
        : /^worker_(start|stop)_/.test(detail)
        ? "workerSwitchFailed"
        : detail.includes("worker_operation_unavailable")
        ? "workerOperationUnavailable"
        : detail.includes("diagnostic_retry_wait")
        ? "rateLimitWait"
        : detail.includes("export_protected_path")
          ? "exportProtectedPath"
          : detail.includes("download_cache_limit_exceeded")
            ? "cacheLimitExceeded"
            : detail.includes("transfer_insufficient_space")
              ? "insufficientSpace"
              : detail === "desktop_required"
                ? "desktopRequired"
                : detail.includes("file_changed")
                  ? "fileChanged"
                  : detail.includes("restore_destination_exists")
                    ? "destinationExists"
                    : detail === "busy"
                      ? "busy"
                      : "operationError";
      setNotice({ key, detail, error: true });
      return false;
    } finally {
      setPending(false);
    }
  }, []);
  const props: ViewProps = {
    data,
    t,
    run,
    pending,
    navigate: setPage,
    setup: () => setInstaller(true),
  };
  const noticeView = notice && (
    <div
      className={`toast ${notice.error ? "error" : ""}`}
      role={notice.error ? "alert" : "status"}
    >
      <div>
        <strong>{t(notice.key)}</strong>
        {notice.detail && (
          <details>
            <summary>{t("operationError")}</summary>
            <pre>{notice.detail}</pre>
          </details>
        )}
      </div>
      <button
        className="icon-button"
        aria-label={t("dismiss")}
        onClick={() => setNotice(null)}
      >
        <X size={18} />
      </button>
    </div>
  );
  const modal = document.querySelector("dialog[open]");
  return (
    <>
      {floating ? (
        <main className="floating-shell">
          <ActivityPanel {...props} floating />
        </main>
      ) : (
        <div className="app-shell">
          <aside className="sidebar">
            <div className="brand">
              <Shield size={39} strokeWidth={1.8} />
              <strong>ClamAV Desktop</strong>
            </div>
            <nav aria-label="ClamAV Desktop">
              {pages.map(({ id, icon: Icon }) => (
                <button
                  key={id}
                  aria-current={page === id ? "page" : undefined}
                  className={page === id ? "active" : ""}
                  onClick={() => setPage(id)}
                >
                  <Icon size={21} strokeWidth={1.8} />
                  <span>{t(id)}</span>
                  {id === "quarantine" &&
                    data.quarantine.filter((q) => q.status === "active")
                      .length > 0 && (
                      <small className="count">
                        {
                          data.quarantine.filter((q) => q.status === "active")
                            .length
                        }
                      </small>
                    )}
                </button>
              ))}
            </nav>
            <p className="brand-note">{t("brandNote")}</p>
          </aside>
          <main className="main-content">
            <div className="toolbar">
              <div className="language-control">
                <Globe2 size={17} />
                <select
                  aria-label={t("language")}
                  value={data.settings.language}
                  disabled={pending}
                  onChange={(e) =>
                    void run(() =>
                      command("save_settings", {
                        settings: {
                          ...data.settings,
                          language: e.target.value as Language,
                        },
                      }),
                    )
                  }
                >
                  <option value="ja">日本語</option>
                  <option value="en">English</option>
                  <option value="zh-CN">简体中文</option>
                </select>
              </div>
              <button
                className="icon-button bordered"
                title={t("refresh")}
                aria-label={t("refresh")}
                disabled={pending || !desktop}
                onClick={() => void run(() => command("refresh_engine"))}
              >
                <RefreshCw size={19} />
              </button>
            </div>
            {!desktop && (
              <div className="preview-note">
                <AlertCircle size={15} />
                <span>{t("preview")}</span>
              </div>
            )}
            {data.operation?.kind === "integrity" ? (
              <IntegrityProgress operation={data.operation} t={t} />
            ) : (
              data.operation && (
                <section className="operation" aria-live="polite">
                  <LoaderCircle size={21} className="spin" />
                  <div>
                    <strong>
                      {isCopyKey(data.operation.stage)
                        ? t(data.operation.stage)
                        : data.operation.stage}
                    </strong>
                    {data.operation.stage === "downloading" && (
                      <small>
                        {(data.operation.downloaded / 1024 / 1024).toFixed(1)}{" "}
                        MB
                        {data.operation.total
                          ? ` / ${(data.operation.total / 1024 / 1024).toFixed(1)} MB`
                          : ""}
                      </small>
                    )}
                    {data.operation.message && <p>{data.operation.message}</p>}
                  </div>
                  {data.operation.stage === "downloading" &&
                    data.operation.total && (
                      <progress
                        value={data.operation.downloaded}
                        max={data.operation.total}
                      />
                    )}
                </section>
              )
            )}
            {page === "home" ? (
              <Home {...props} />
            ) : page === "scan" ? (
              <Scanner {...props} />
            ) : page === "quarantine" ? (
              <Quarantine {...props} />
            ) : page === "history" ? (
              <History {...props} />
            ) : (
              <Settings key={data.settings.language} {...props} />
            )}
          </main>
        </div>
      )}
      {installer && <Installer {...props} close={() => setInstaller(false)} />}{" "}
      {noticeView && (modal ? createPortal(noticeView, modal) : noticeView)}
    </>
  );
}
