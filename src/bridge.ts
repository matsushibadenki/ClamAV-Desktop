import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useSyncExternalStore } from "react";
import type { Snapshot, Settings } from "./types";
export const desktop = isTauri();
let snapshot: Snapshot = {
  engine: {
    path: null,
    version: null,
    definitions: false,
    databasePath: null,
    definitionsAt: null,
    checked: !desktop,
  },
  scan: null,
  history: [],
  quarantine: [],
  settings: {
    language: "ja",
    autoUpdate: false,
    scheduleEnabled: false,
    scheduleHour: 12,
    maxFileMb: 100,
  },
  activity: [],
  operation: null,
  activityFloating: false,
  activityDock: "right",
  installerReady: false,
  dataDir: "—",
};
const listeners = new Set<() => void>();
let revision = 0;
function publish(next: Snapshot) {
  snapshot = next;
  revision++;
  listeners.forEach((l) => l());
}
export async function connect() {
  if (!desktop) return () => {};
  const unlisten = await listen<Snapshot>("core:state", (e) =>
    publish(e.payload),
  );
  const before = revision;
  const next = await invoke<Snapshot>("get_snapshot");
  if (revision === before) publish(next);
  return unlisten;
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
export function useCore() {
  return useSyncExternalStore(subscribe, () => snapshot);
}
type Command =
  | "refresh_engine"
  | "prepare_installer"
  | "open_installer"
  | "update_definitions"
  | "start_scan"
  | "stop_scan"
  | "choose_folder"
  | "quarantine_file"
  | "restore_file"
  | "save_settings"
  | "set_activity_panel"
  | "export_history"
  | "open_privacy_settings";
export async function command<T = void>(
  name: Command,
  args?: Record<string, unknown>,
): Promise<T> {
  if (!desktop) {
    if (name === "save_settings") {
      publish({ ...snapshot, settings: args!.settings as Settings });
      return undefined as T;
    }
    throw new Error("desktop_required");
  }
  return invoke<T>(name, args);
}
