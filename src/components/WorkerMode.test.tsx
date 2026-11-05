import { expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { translate } from "../i18n";
import type { Snapshot } from "../types";
vi.mock("../bridge", () => ({ desktop: false, command: vi.fn() }));
import WorkerMode from "./WorkerMode";

it.each(["ja", "en", "zh-CN"] as const)("renders unknown OS state before native data arrives (%s)", (language) => {
  const data: Snapshot = {
    engine: { path: null, version: null, definitions: false, databasePath: null, definitionsAt: null, checked: false },
    scan: null, history: [], quarantine: [], activity: [], operation: null,
    settings: { language, autoUpdate: false, scheduleEnabled: false, scheduleHour: 12, maxFileMb: 100, downloadRetentionDays: 30, downloadCacheMb: 1024 },
    activityFloating: false, activityDock: "right", installerReady: false, dataDir: "—", diagnostics: [],
  };
  const html = renderToStaticMarkup(<WorkerMode data={data} t={(key) => translate(language, key)} run={async () => false} pending={false} navigate={() => {}} setup={() => {}} />);
  expect(html).toContain(translate(language, "loginStartup"));
  expect(html).toContain(translate(language, "loginPermission"));
  expect(html).toContain(translate(language, "loginPermissionUnknown"));
  expect(html).toContain(translate(language, "loginLoadUnknown"));
  expect(html).toContain("disabled");
});
