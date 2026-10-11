import { describe, it, expect } from "vitest";
import { quarantineList } from "./quarantine-list";
import type { QuarantineItem } from "./types";
const item = (
  id: string,
  status: string,
  originalPath: string,
  quarantinedAt: string,
): QuarantineItem => ({
  id,
  status,
  originalPath,
  quarantinedAt,
  signature: "Test.Detection",
  digest: "hash",
  mode: 384,
});
const rows = [
  item("a", "active", "/Users/検証/ＡＢＣ.txt", "2026-10-10T00:00:00Z"),
  item(
    "b",
    "transfer_pending",
    "/Volumes/Drive/file.txt",
    "2026-10-09T00:00:00Z",
  ),
  item("c", "restored", "/Users/old.txt", "2026-10-11T00:00:00Z"),
];
describe("quarantine search and sorting", () => {
  it("combines normalized path, detection and localized status words without losing completion filter", () => {
    const label = (s: string) => (s === "active" ? "隔離済み" : "復旧待ち");
    expect(
      quarantineList(rows, "abc 隔離済み test", false, "newest", label).map(
        (r) => r.id,
      ),
    ).toEqual(["a"]);
    expect(quarantineList(rows, "old", false, "newest", label)).toEqual([]);
    expect(
      quarantineList(rows, "old", true, "newest", label).map((r) => r.id),
    ).toEqual(["c"]);
  });
  it("prioritizes recovery over recency and never mutates Rust's snapshot array", () => {
    expect(
      quarantineList(rows, "", true, "attention", (s) => s).map((r) => r.id),
    ).toEqual(["b", "a", "c"]);
    expect(
      quarantineList(rows, "", true, "oldest", (s) => s).map((r) => r.id),
    ).toEqual(["b", "a", "c"]);
    expect(
      quarantineList(rows, "", true, "newest", (s) => s).map((r) => r.id),
    ).toEqual(["c", "a", "b"]);
    expect(rows.map((r) => r.id)).toEqual(["a", "b", "c"]);
  });
});
