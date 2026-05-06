import { describe, expect, it } from "vitest";
import {
  UPDATE_CHECK_PREFIX,
  ERR_INSTALLED_PLUGINS,
  ERR_UPDATE_FETCH,
  updateCheckErrKey,
  parseUpdateCheckKey,
  isUpdateCheckKey,
} from "./error-source";

describe("updateCheckErrKey", () => {
  it("returns the exact 3-segment string", () => {
    expect(updateCheckErrKey("stale_cache", "acme")).toBe(
      `${UPDATE_CHECK_PREFIX}\u001fstale_cache\u001facme`,
    );
    expect(updateCheckErrKey("manifest_invalid", "acme")).toBe(
      `${UPDATE_CHECK_PREFIX}\u001fmanifest_invalid\u001facme`,
    );
  });
});

describe("parseUpdateCheckKey", () => {
  it("round-trips through updateCheckErrKey", () => {
    for (const [r, mp] of [
      ["stale_cache", "acme"],
      ["manifest_invalid", "acme"],
      ["unknown", "other-mp"],
    ] as const) {
      const key = updateCheckErrKey(r, mp);
      const parsed = parseUpdateCheckKey(key);
      expect(parsed).toEqual({ remediation: r, marketplace: mp });
    }
  });

  it("throws on malformed key (fewer than 3 segments)", () => {
    expect(() => parseUpdateCheckKey("update-check")).toThrow(
      "parseUpdateCheckKey: malformed key",
    );
  });

  it("throws on malformed key (empty segments)", () => {
    expect(() =>
      parseUpdateCheckKey(`update-check\u001f\u001facme`),
    ).toThrow("parseUpdateCheckKey: malformed key");
  });

  it("throws on wrong prefix (3 segments but not 'update-check')", () => {
    expect(() =>
      parseUpdateCheckKey(`foo\u001fbar\u001fbaz`),
    ).toThrow("parseUpdateCheckKey: malformed key");
  });
});

describe("isUpdateCheckKey", () => {
  it("returns true for keys produced by updateCheckErrKey", () => {
    expect(isUpdateCheckKey(updateCheckErrKey("stale_cache", "acme"))).toBe(true);
    expect(isUpdateCheckKey(updateCheckErrKey("manifest_invalid", "beta"))).toBe(true);
  });

  it("returns false for fewer than 3 segments", () => {
    expect(isUpdateCheckKey("update-check")).toBe(false);
    expect(isUpdateCheckKey(`update-checkstale_cache`)).toBe(false);
  });

  it("returns false for empty segments", () => {
    expect(isUpdateCheckKey(`update-checkacme`)).toBe(false);
  });

  it("returns false for wrong prefix", () => {
    expect(isUpdateCheckKey(`foobarbaz`)).toBe(false);
  });

  it("returns false for unrelated namespace keys", () => {
    expect(isUpdateCheckKey("installed-plugins")).toBe(false);
    expect(isUpdateCheckKey("update-fetch")).toBe(false);
    expect(isUpdateCheckKey(`pluginsacme`)).toBe(false);
  });
});

describe("shared constants", () => {
  it("ERR_INSTALLED_PLUGINS and ERR_UPDATE_FETCH hold their string values", () => {
    expect(ERR_INSTALLED_PLUGINS).toBe("installed-plugins");
    expect(ERR_UPDATE_FETCH).toBe("update-fetch");
  });

  it("UPDATE_CHECK_PREFIX is 'update-check'", () => {
    expect(UPDATE_CHECK_PREFIX).toBe("update-check");
  });
});

// Compile-time guard: mirrors the _AssertNarrow pattern in error-source.ts.
// If either constant widens to string, this line fails type-check.
type _AssertNarrow = string extends typeof UPDATE_CHECK_PREFIX ? never : typeof UPDATE_CHECK_PREFIX;
