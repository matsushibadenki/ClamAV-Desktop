export type Language = "ja" | "en" | "zh-CN";
export type Page = "home" | "scan" | "quarantine" | "history" | "settings";
export interface Settings {
  language: Language;
  autoUpdate: boolean;
  scheduleEnabled: boolean;
  scheduleHour: number;
  maxFileMb: number;
  downloadRetentionDays: number;
  downloadCacheMb: number;
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

export interface CacheReport {
  bytes: number;
  reclaimableBytes: number;
  attempts: number;
  removable: number;
  skipped: number;
}
export interface FileState {
  path: string;
  state: string;
  bytes: number | null;
}
export interface QuarantineDetail {
  item: QuarantineItem;
  original: FileState;
  stored: FileState;
  integrity: {
    checkedAt: string;
    state: string;
    actualDigest: string | null;
    error: string | null;
  } | null;
  transfers: {
    kind: string;
    published: boolean;
    lastError: string | null;
    attemptedAt: string | null;
    source: FileState;
    destination: FileState;
    staged: FileState;
    temporary: FileState;
  }[];
}
