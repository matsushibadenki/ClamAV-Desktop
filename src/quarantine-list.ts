import type { QuarantineItem } from "./types";
export type QuarantineSort = "newest" | "oldest" | "path" | "attention";
const normalize = (text: string) => text.normalize("NFKC").toLowerCase();
const priority: Record<string, number> = {
  transfer_pending: 0,
  integrity_error: 1,
  missing: 2,
  review: 3,
  moving: 4,
  restoring: 4,
  active: 5,
  move_failed: 6,
  restored: 7,
};
export function quarantineList(
  items: QuarantineItem[],
  query: string,
  completed: boolean,
  sort: QuarantineSort,
  statusLabel: (status: string) => string,
) {
  const words = normalize(query).trim().split(/\s+/).filter(Boolean);
  return items
    .filter(
      (item) =>
        (completed || !["restored", "move_failed"].includes(item.status)) &&
        words.every((word) =>
          normalize(
            `${item.originalPath} ${item.signature} ${statusLabel(item.status)}`,
          ).includes(word),
        ),
    )
    .sort((a, b) => {
      const newest =
        b.quarantinedAt.localeCompare(a.quarantinedAt) ||
        a.id.localeCompare(b.id);
      if (sort === "oldest") return -newest;
      if (sort === "path")
        return a.originalPath.localeCompare(b.originalPath) || newest;
      if (sort === "attention")
        return (priority[a.status] ?? 0) - (priority[b.status] ?? 0) || newest;
      return newest;
    });
}
