import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, test } from "bun:test";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
globalThis.window = globalThis;
eval(readFileSync(join(root, "src/common.js"), "utf8"));
const QB = globalThis.QB;

describe("tokenMix", () => {
  test("splits cached input from the lifetime total and does not invent output tokens", () => {
    const mix = QB.tokenMix({
      total_tokens: 21049533448,
      cached_input_tokens: 20925721382,
      request_count: 97938,
      total_cost_usd: 49999.9915
    });
    expect(mix.total).toBe(21049533448);
    expect(mix.cachedInput).toBe(20925721382);
    expect(mix.other).toBe(21049533448 - 20925721382);
    expect(mix.cacheShare).toBeCloseTo((20925721382 / 21049533448) * 100, 8);
    expect(mix.perRequest).toBeCloseTo(21049533448 / 97938, 6);
    expect(mix.usdPerMillion).toBeCloseTo(49999.9915 / (21049533448 / 1e6), 8);
  });

  test("returns null when total_tokens is missing", () => {
    expect(QB.tokenMix({ cached_input_tokens: 1 })).toBeNull();
  });
});

describe("dailyCapUsd", () => {
  test("uses the all-models daily max, not the fable daily max or the settings fallback", () => {
    const limits = [
      { window: "3h", fable: false, max: 3500 },
      { window: "daily", fable: false, max: 5600 },
      { window: "daily", fable: true, max: 2800 }
    ];
    expect(QB.dailyCapUsd(limits, 6400)).toBe(5600);
  });

  test("falls back to the settings cap when the proxy sent no daily all-models limit", () => {
    expect(QB.dailyCapUsd([{ window: "weekly", fable: false, max: 16800 }], 6400)).toBe(6400);
    expect(QB.dailyCapUsd([], 0)).toBeNull();
  });
});

describe("hoursToEmpty / pickBinding", () => {
  test("does not empty a fable limit with all-model burn", () => {
    const fable = {
      fable: true,
      left: 10,
      usedPct: 99,
      locked: false,
      resetIn: 86400,
      windowSecs: 86400
    };
    expect(QB.hoursToEmpty(fable, 400)).toBeNull();
    const all = {
      fable: false,
      left: 5331,
      usedPct: 4.79,
      locked: false,
      resetIn: 86400,
      windowSecs: 86400
    };
    expect(QB.hoursToEmpty(all, 400)).toBeCloseTo(5331 / 400, 8);
    expect(QB.pickBinding([fable, all], 400)).toBe(all);
  });
});
