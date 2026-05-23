import { describe, expect, test } from "vitest";

import {
  buildFilePrompt,
  clearPromptOnModeSwitch,
  detectPromptMode,
  filePathFromPrompt,
} from "./prompt-mode";

describe("detectPromptMode", () => {
  // Plan cases 1-6.
  test("null is inline (default)", () => {
    expect(detectPromptMode(null)).toBe("inline");
  });

  test("empty string is inline", () => {
    expect(detectPromptMode("")).toBe("inline");
  });

  test("regular text is inline", () => {
    expect(detectPromptMode("Hello")).toBe("inline");
  });

  test("full file:// URL is file", () => {
    expect(detectPromptMode("file://path/to/file.md")).toBe("file");
  });

  test("bare file:// scheme is file (matches startsWith)", () => {
    // Weird but pinned: a value of just `"file://"` (the result of
    // a fresh mode switch) MUST render as file mode so the user can
    // start typing the path. A `startsWith` check that also requires
    // a non-empty path component would falsify this.
    expect(detectPromptMode("file://")).toBe("file");
  });

  test("uppercase scheme is inline (case-sensitive)", () => {
    // Adversarial: a future change to use `.toLowerCase().startsWith(...)`
    // would falsify this, allowing an inline prompt that happens to
    // start with literal "File://" to render as if it were a path
    // chip. The wire format is canonical lowercase.
    expect(detectPromptMode("File://X")).toBe("inline");
  });

  test("only-prefix-substring is inline (must match exactly)", () => {
    expect(detectPromptMode("file:")).toBe("inline");
    expect(detectPromptMode("file:/")).toBe("inline");
  });
});

describe("clearPromptOnModeSwitch", () => {
  // Plan cases 7-8.
  test("switching to file returns 'file://'", () => {
    expect(clearPromptOnModeSwitch("file")).toBe("file://");
  });

  test("switching to inline returns empty string", () => {
    expect(clearPromptOnModeSwitch("inline")).toBe("");
  });
});

describe("filePathFromPrompt / buildFilePrompt round-trip", () => {
  // Round-trip property: filePathFromPrompt(buildFilePrompt(p)) === p
  // for every plausible path.
  test.each([
    "",
    "rel/path.md",
    "/abs/path.md",
    "C:\\Windows\\style.md",
    "with spaces and Unicode 📝.md",
  ])("round-trip preserves %s", (path) => {
    expect(filePathFromPrompt(buildFilePrompt(path))).toBe(path);
  });

  test("filePathFromPrompt returns empty string for inline content", () => {
    // Defensive — the panel only calls this in file-mode branches,
    // but a bug that called it on inline content must NOT slice 7
    // chars off arbitrary text.
    expect(filePathFromPrompt("just inline text")).toBe("");
    expect(filePathFromPrompt("")).toBe("");
  });

  test("buildFilePrompt('') yields the bare scheme", () => {
    // Matches the post-mode-switch state from
    // clearPromptOnModeSwitch('file').
    expect(buildFilePrompt("")).toBe("file://");
  });
});
