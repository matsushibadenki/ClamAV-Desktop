import { useEffect, useState } from "react";
import type { DiagnosticAttempt } from "./types";
export default function useRetryBlock(attempts: DiagnosticAttempt[]) {
  const [now, setNow] = useState(Date.now());
  const nextExpiry = attempts.reduce((next, a) => {
    const time =
      a.status === "failed" && a.retryNotBefore
        ? Date.parse(a.retryNotBefore)
        : 0;
    return time > now ? Math.min(next, time) : next;
  }, Infinity);
  useEffect(() => {
    if (!Number.isFinite(nextExpiry)) return;
    const timer = setTimeout(
      () => setNow(Date.now()),
      Math.max(1, Math.min(nextExpiry - Date.now(), 60000)),
    );
    return () => clearTimeout(timer);
  }, [nextExpiry, now]);
  return (kind: string) =>
    attempts.some(
      (a) =>
        a.kind === kind &&
        a.status === "failed" &&
        !!a.retryNotBefore &&
        Date.parse(a.retryNotBefore) > now,
    );
}
