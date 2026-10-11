export type Language = "ja" | "en" | "zh-CN";
export type Page = "home" | "scan" | "quarantine" | "history" | "settings";
export interface Settings {
  language: Language;
  autoUpdate: boolean;
  scheduleEnabled: boolean;
  scheduleHour: number;
  maxFileMb: number;
}
export interface Engine {
  path: string | null;
  version: string | null;
  definitions: boolean;
  databasePath: string | null;
  definitionsAt: string | null;
  checked: boolean;
}
export interface Finding {
  id: string;
  path: string;
  signature: string;
  digest: string | null;
  quarantined: boolean;
}
export interface Scan {
  id: string;
  kind: string;
  target: string;
  startedAt: string;
  finishedAt: string | null;
  status: string;
  scanned: number;
  errors: number;
  currentFile: string;
  findings: Finding[];
  message: string | null;
}
export interface QuarantineItem {
  id: string;
  originalPath: string;
  signature: string;
  digest: string;
  quarantinedAt: string;
  mode: number;
  status: string;
}
export interface Snapshot {
  engine: Engine;
  scan: Scan | null;
  history: Scan[];
  quarantine: QuarantineItem[];
  settings: Settings;
  activity: { at: string; code: string; detail: string }[];
  operation: {
    kind: string;
    stage: string;
    downloaded: number;
    total: number | null;
    message: string | null;
  } | null;
  activityFloating: boolean;
  activityDock: string;
  installerReady: boolean;
  dataDir: string;
}
