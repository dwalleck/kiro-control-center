import { describe, expect, test } from "vitest";

import { isValidAgentName, validateAgentNameForSave } from "./agent-name";

describe("isValidAgentName", () => {
  // The 9 plan cases pin the strict regex `^[a-z0-9][a-z0-9-]*$`.
  // A naive `name.length > 0` check falsifies cases 3-9; a
  // case-insensitive regex (`/i` flag) falsifies case 3.
  test("empty string is rejected", () => {
    expect(isValidAgentName("")).toBe(false);
  });

  test("kebab-case name is accepted", () => {
    expect(isValidAgentName("good-name")).toBe(true);
  });

  test("uppercase first letter is rejected", () => {
    expect(isValidAgentName("Bad")).toBe(false);
  });

  test("leading hyphen is rejected", () => {
    expect(isValidAgentName("-leads")).toBe(false);
  });

  test("single-char name is accepted", () => {
    expect(isValidAgentName("a")).toBe(true);
  });

  test("long-but-valid name is accepted", () => {
    expect(isValidAgentName("a".repeat(200))).toBe(true);
  });

  test("internal whitespace is rejected", () => {
    expect(isValidAgentName("has space")).toBe(false);
  });

  test("dot is rejected", () => {
    expect(isValidAgentName("with.dot")).toBe(false);
  });

  test("Unicode is rejected", () => {
    expect(isValidAgentName("naïve")).toBe(false);
  });
});

describe("validateAgentNameForSave (split-policy per kiro-k9ok)", () => {
  // **New-agent mode** — originalName is "". Every draft name must
  // pass the regex.
  test("new mode: empty name returns 'Name is required.'", () => {
    expect(validateAgentNameForSave("", "")).toBe("Name is required.");
  });

  test("new mode: kebab-case name returns null", () => {
    expect(validateAgentNameForSave("code-reviewer", "")).toBeNull();
  });

  test("new mode: regex-violating name returns the kebab message", () => {
    expect(validateAgentNameForSave("Terraform Agent", "")).toMatch(
      /lowercase letters/,
    );
  });

  // **Edit mode, unchanged name** — escape hatch lets a marketplace
  // agent save without renaming, even if the name violates the
  // regex. Pinned by kiro-k9ok decision.
  test("edit mode: unchanged regex-passing name returns null", () => {
    expect(
      validateAgentNameForSave("good-name", "good-name"),
    ).toBeNull();
  });

  test("edit mode: unchanged regex-violating name returns null (escape hatch)", () => {
    // The bug class this defeats: a strict-only check would block
    // saving a marketplace-installed "Terraform Agent" without
    // renaming, forcing the user to either rename or edit the JSON
    // by hand outside the Control Center.
    expect(
      validateAgentNameForSave("Terraform Agent", "Terraform Agent"),
    ).toBeNull();
  });

  // **Edit mode, rename** — new name must pass the regex even if
  // the original didn't.
  test("edit mode: rename to valid kebab-case returns null", () => {
    expect(
      validateAgentNameForSave("new-name", "old-name"),
    ).toBeNull();
  });

  test("edit mode: rename FROM permissive name TO regex-violating name is rejected", () => {
    // Rename Terraform Agent → "Bad Name" must fail the regex even
    // though the original name didn't match either. The escape
    // hatch is for keeping the name, not for renaming through it.
    expect(
      validateAgentNameForSave("Bad Name", "Terraform Agent"),
    ).toMatch(/lowercase letters/);
  });

  test("edit mode: rename TO empty is rejected with required-name message", () => {
    expect(validateAgentNameForSave("", "old-name")).toBe(
      "Name is required.",
    );
  });

  test("edit mode: rename to valid kebab from regex-violating original", () => {
    // The user IS allowed to rename a permissive name to a kebab one.
    expect(
      validateAgentNameForSave("terraform-agent", "Terraform Agent"),
    ).toBeNull();
  });
});
