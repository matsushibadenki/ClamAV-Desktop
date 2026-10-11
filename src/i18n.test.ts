import { describe, it, expect } from "vitest";
import { translate, date } from "./i18n";
describe("language contract", () => {
  it("provides the three required languages and localized dates", () => {
    expect(translate("ja", "setup")).toBe("ClamAVを準備");
    expect(translate("en", "setup")).toBe("Set up ClamAV");
    expect(translate("zh-CN", "setup")).toBe("准备 ClamAV");
    for (const lang of ["ja", "en", "zh-CN"] as const) {
      expect(date("2026-10-11T06:00:00Z", lang)).toContain("2026");
      expect(translate(lang, "onDemand")).not.toBe("");
    }
  });
});
