import { useState } from "react";
import { command } from "../bridge";
import type { Snapshot } from "../types";
import type { T } from "./shared";
import { bytes } from "./DownloadCache";
export default function IntegrityProgress({
  operation,
  t,
}: {
  operation: NonNullable<Snapshot["operation"]>;
  t: T;
}) {
  const [error, setError] = useState<string | null>(null);
  const stopping = operation.stage === "stoppingIntegrity";
  const finishing = operation.stage === "finishingIntegrity";
  const total = operation.total;
  const value =
    total === null
      ? operation.downloaded
      : Math.min(operation.downloaded, total);
  return (
    <section className="integrity-progress" role="status">
      <strong>
        {t(
          finishing
            ? "finishingIntegrity"
            : stopping
              ? "stoppingIntegrity"
              : "checkingIntegrity",
        )}
      </strong>
      <p>
        {bytes(operation.downloaded)}
        {total !== null ? ` / ${bytes(total)}` : ""}
        {total ? ` · ${Math.floor((value / total) * 100)}%` : ""}
      </p>
      {total !== null && total > 0 ? (
        <progress
          aria-label={t("integrityProgress")}
          value={value}
          max={total}
        />
      ) : (
        <progress aria-label={t("integrityProgress")} />
      )}
      <button
        className="secondary"
        disabled={stopping || finishing}
        onClick={() =>
          void command("stop_integrity_check").catch((e) => setError(String(e)))
        }
      >
        {t(stopping ? "stopping" : "stopIntegrity")}
      </button>
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
