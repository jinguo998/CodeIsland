import { describe, expect, it } from "vitest";
import enUS from "../i18n/en-US.json";
import zhCN from "../i18n/zh-CN.json";

function flattenKeys(value: unknown, prefix = ""): string[] {
  if (value !== null && typeof value === "object" && !Array.isArray(value)) {
    return Object.entries(value as Record<string, unknown>).flatMap(
      ([key, child]) =>
        flattenKeys(child, prefix ? `${prefix}.${key}` : key),
    );
  }
  return [prefix];
}

function flattenLeaves(value: unknown, prefix = ""): Array<[string, unknown]> {
  if (value !== null && typeof value === "object" && !Array.isArray(value)) {
    return Object.entries(value as Record<string, unknown>).flatMap(
      ([key, child]) =>
        flattenLeaves(child, prefix ? `${prefix}.${key}` : key),
    );
  }
  return [[prefix, value]];
}

describe("i18n resources", () => {
  it("en-US and zh-CN expose identical key sets", () => {
    const en = flattenKeys(enUS).sort();
    const zh = flattenKeys(zhCN).sort();
    expect(zh).toEqual(en);
  });

  it("has no empty translations", () => {
    for (const [locale, resource] of [
      ["en-US", enUS],
      ["zh-CN", zhCN],
    ] as const) {
      const empty = flattenLeaves(resource).filter(
        ([, value]) => typeof value !== "string" || value.trim() === "",
      );
      expect(empty, `${locale} has empty values`).toEqual([]);
    }
  });
});
