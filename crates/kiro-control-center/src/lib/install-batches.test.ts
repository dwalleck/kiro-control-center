import { describe, expect, it, vi } from "vitest";

import { runInstallBatches, type InstallBatch } from "./install-batches";

function okBatch<T>(names: readonly string[], data: T): InstallBatch<T> {
  return {
    names,
    call: vi.fn().mockResolvedValue({ status: "ok", data }),
  };
}

function errorBatch<T>(
  names: readonly string[],
  message: string,
  remediation: string | null = null,
): InstallBatch<T> {
  return {
    names,
    call: vi.fn().mockResolvedValue({
      status: "error",
      error: { message, error_type: "validation", remediation },
    }),
  };
}

function throwingBatch<T>(names: readonly string[], e: unknown): InstallBatch<T> {
  return {
    names,
    call: vi.fn().mockRejectedValue(e),
  };
}

function emptyBatch<T>(): InstallBatch<T> {
  return { names: [], call: vi.fn() };
}

describe("runInstallBatches", () => {
  it("returns each batch's payload on success and no error", async () => {
    const result = await runInstallBatches("acme", "demo", {
      skills: okBatch(["s1"], { installed: ["s1"] }),
      steering: okBatch(["t1"], { installed: ["t1"] }),
      agents: okBatch(["a1"], { installed: ["a1"] }),
    });
    expect(result.skills).toEqual({ installed: ["s1"] });
    expect(result.steering).toEqual({ installed: ["t1"] });
    expect(result.agents).toEqual({ installed: ["a1"] });
    expect(result.error).toBeNull();
  });

  it("skips empty batches without invoking their call", async () => {
    const skills = emptyBatch<{ installed: string[] }>();
    const steering = emptyBatch<{ installed: string[] }>();
    const agents = okBatch(["a1"], { installed: ["a1"] });
    const result = await runInstallBatches("acme", "demo", {
      skills,
      steering,
      agents,
    });
    expect(skills.call).not.toHaveBeenCalled();
    expect(steering.call).not.toHaveBeenCalled();
    expect(result.skills).toBeNull();
    expect(result.steering).toBeNull();
    expect(result.agents).toEqual({ installed: ["a1"] });
    expect(result.error).toBeNull();
  });

  it("surfaces a single wrapper-level status=error with category and plugin", async () => {
    const result = await runInstallBatches("acme", "demo", {
      skills: errorBatch(["s1"], "tracking file locked"),
      steering: emptyBatch(),
      agents: emptyBatch(),
    });
    expect(result.skills).toBeNull();
    expect(result.error).toBe(
      "Customize apply: skill install failed for acme/demo: tracking file locked",
    );
  });

  it("appends remediation to a wrapper-level status=error", async () => {
    const result = await runInstallBatches("acme", "demo", {
      skills: errorBatch(
        ["s1"],
        "plugin is not available locally",
        "open the plugin detail to clone it",
      ),
      steering: emptyBatch(),
      agents: emptyBatch(),
    });

    expect(result.error).toBe(
      "Customize apply: skill install failed for acme/demo: "
        + "plugin is not available locally — open the plugin detail to clone it",
    );
  });

  it("surfaces a single wrapper-level throw with category and plugin", async () => {
    const result = await runInstallBatches("acme", "demo", {
      skills: emptyBatch(),
      steering: throwingBatch(["t1"], new Error("ipc channel closed")),
      agents: emptyBatch(),
    });
    expect(result.steering).toBeNull();
    expect(result.error).toBe(
      "Customize apply: steering install threw for acme/demo: ipc channel closed",
    );
  });

  it("stringifies non-Error throw values", async () => {
    const result = await runInstallBatches("acme", "demo", {
      skills: emptyBatch(),
      steering: emptyBatch(),
      agents: throwingBatch(["a1"], "raw string rejection"),
    });
    expect(result.error).toBe(
      "Customize apply: agent install threw for acme/demo: raw string rejection",
    );
  });

  it("one failing batch does not discard another batch's success payload", async () => {
    const result = await runInstallBatches("acme", "demo", {
      skills: errorBatch(["s1"], "skills exploded"),
      steering: okBatch(["t1"], { installed: ["t1"] }),
      agents: emptyBatch(),
    });
    expect(result.skills).toBeNull();
    expect(result.steering).toEqual({ installed: ["t1"] });
    expect(result.error).toContain("skills exploded");
  });

  it("accumulates all wrapper-level batch failures", async () => {
    const result = await runInstallBatches("acme", "demo", {
      skills: errorBatch(["s1"], "skills exploded"),
      steering: throwingBatch(["t1"], new Error("steering exploded")),
      agents: errorBatch(["a1"], "agents exploded"),
    });
    // Every failing category's message must be visible to the user, in
    // fixed category order — resolution order must decide neither which
    // failure survives nor the joined message's sequence.
    expect(result.error).toBe(
      "Customize apply: skill install failed for acme/demo: skills exploded"
        + " | Customize apply: steering install threw for acme/demo: steering exploded"
        + " | Customize apply: agent install failed for acme/demo: agents exploded",
    );
  });

  it("accumulates both failures when exactly two batches fail", async () => {
    const result = await runInstallBatches("acme", "demo", {
      skills: errorBatch(["s1"], "skills exploded"),
      steering: okBatch(["t1"], { installed: ["t1"] }),
      agents: errorBatch(["a1"], "agents exploded"),
    });
    const visible = result.error ?? "";
    expect(visible).toContain("skills exploded");
    expect(visible).toContain("agents exploded");
    expect(result.steering).toEqual({ installed: ["t1"] });
  });
});
