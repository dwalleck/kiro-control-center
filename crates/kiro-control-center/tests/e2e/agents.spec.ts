import { expect, test, type Page } from "@playwright/test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

// Slice S17 — end-to-end CRUD round-trip for the Workflows > Agents
// view. Mirrors the gating pattern from `app.spec.ts` (FIXTURE_*
// env vars), but for agents the fixture IS a fresh Node-side tmpdir
// the test creates and cleans up. The Tauri runtime is started once
// by Playwright's webServer config (`cargo tauri dev`); each test
// in this file runs serially so the active-project handoff between
// tests doesn't race.
//
// **Tauri IPC access**: tests call `window.__TAURI_INTERNALS__.invoke`
// directly via `page.evaluate`. Going through the typed `commands`
// object would require importing it into the page context, which
// the renderer's module system doesn't expose to test code. The
// `__TAURI_INTERNALS__` global is the canonical Tauri 2 entrypoint
// for renderer-side IPC.

test.describe.configure({ mode: "serial" });

interface TauriWindow {
  __TAURI_INTERNALS__: {
    invoke<T = unknown>(cmd: string, args?: Record<string, unknown>): Promise<T>;
  };
}

/**
 * Create a tempdir with a `.kiro/` subdirectory and tell the running
 * Tauri app to use it as the active project. Reloads the page so
 * `+page.svelte`'s onMount picks up the new active project. Returns
 * the tmpdir path so the caller can do filesystem assertions and
 * clean up.
 *
 * The path is canonicalised by `setActiveProject` in the backend
 * (it returns `result.data.path`), so callers should compare files
 * using the returned `activePath` rather than the raw tmpdir.
 */
async function setupFreshProject(
  page: Page,
): Promise<{ tmpdir: string; activePath: string }> {
  const tmpdir = fs.mkdtempSync(path.join(os.tmpdir(), "kiro-e2e-agents-"));
  fs.mkdirSync(path.join(tmpdir, ".kiro"));

  await page.goto("/");
  // Defer the IPC call until the Tauri internals are available — the
  // renderer mounts its globals during the bootstrap phase, not on
  // first paint.
  await page.waitForFunction(
    () => typeof (window as unknown as TauriWindow).__TAURI_INTERNALS__?.invoke === "function",
  );

  const activePath = await page.evaluate(async (p) => {
    const w = window as unknown as TauriWindow;
    const result = await w.__TAURI_INTERNALS__.invoke<{
      path: string;
      kiro_initialized: boolean;
      installed_skill_count: number;
    }>("set_active_project", { path: p });
    return result.path;
  }, tmpdir);

  await page.reload();
  // Wait for the project-loaded UI (NavRail's "Agents" button is a
  // proxy for "the main app rendered, not the ProjectPicker").
  await expect(page.getByRole("button", { name: "Agents", exact: true })).toBeVisible({
    timeout: 15_000,
  });

  return { tmpdir, activePath };
}

function cleanup(tmpdir: string) {
  try {
    fs.rmSync(tmpdir, { recursive: true, force: true });
  } catch (e) {
    // The Tauri dev server may still hold an open file handle on
    // Windows. Don't fail the test on cleanup — the OS will reap
    // the directory eventually.
    console.warn(`cleanup: rm ${tmpdir} failed:`, e);
  }
}

test.describe("Agents view — user-authored CRUD round-trip", () => {
  test("create → edit → duplicate → delete", async ({ page }) => {
    const { tmpdir, activePath } = await setupFreshProject(page);
    try {
      // Navigate to Agents — empty state expected.
      await page.getByRole("button", { name: "Agents", exact: true }).click();
      await expect(page.getByText("No agents yet.")).toBeVisible();

      // ---- CREATE ----
      await page.getByRole("button", { name: /Create Agent/ }).click();
      // Editor is open in new mode. Identity panel exposes the name input.
      await page.getByPlaceholder("code-reviewer").fill("e2e-test");
      await page.getByRole("button", { name: "Create Agent", exact: true }).click();

      // Wait for return to list + toast.
      await expect(page.getByText(/Created.+e2e-test/)).toBeVisible({ timeout: 10_000 });

      const agentPath = path.join(activePath, ".kiro/agents/e2e-test.json");
      expect(fs.existsSync(agentPath)).toBe(true);

      // **Adversarial** — user-authored creates must NOT touch
      // installed-agents.json. A bug that copy-pasted from the
      // marketplace install path would silently track the new
      // agent and confuse the lineage badge logic.
      expect(
        fs.existsSync(path.join(activePath, ".kiro/installed-agents.json")),
        "user-authored create must not write installed-agents.json",
      ).toBe(false);

      // ---- EDIT ----
      await page.getByRole("button", { name: "Edit", exact: true }).click();
      // Editor is in edit mode; wait for the load to finish (the
      // Identity input is populated when load resolves).
      const nameInput = page.getByPlaceholder("code-reviewer");
      await expect(nameInput).toHaveValue("e2e-test", { timeout: 10_000 });

      // Add a description.
      await page
        .getByPlaceholder("Short description for humans")
        .fill("an e2e fixture");
      await page.getByRole("button", { name: "Save Changes", exact: true }).click();

      await expect(page.getByText(/Saved.+e2e-test/)).toBeVisible({ timeout: 10_000 });

      // Filesystem: description landed on disk.
      const updatedJson = JSON.parse(fs.readFileSync(agentPath, "utf8"));
      expect(updatedJson.description).toBe("an e2e fixture");
      expect(updatedJson.name).toBe("e2e-test");

      // ---- DUPLICATE ----
      await page.getByRole("button", { name: "Duplicate e2e-test" }).click();
      await expect(page.getByText(/Duplicated as.+e2e-test-copy/)).toBeVisible({
        timeout: 10_000,
      });

      const copyPath = path.join(activePath, ".kiro/agents/e2e-test-copy.json");
      expect(fs.existsSync(copyPath)).toBe(true);
      const copyJson = JSON.parse(fs.readFileSync(copyPath, "utf8"));
      expect(copyJson.name).toBe("e2e-test-copy");

      // ---- DELETE ----
      // The list's delete uses window.confirm() as the placeholder
      // affordance — accept it.
      page.once("dialog", (dialog) => dialog.accept());
      await page.getByRole("button", { name: "Delete e2e-test-copy" }).click();
      await expect(page.getByText(/Deleted.+e2e-test-copy/)).toBeVisible({
        timeout: 10_000,
      });
      expect(fs.existsSync(copyPath)).toBe(false);
      // Original survives.
      expect(fs.existsSync(agentPath)).toBe(true);
    } finally {
      cleanup(tmpdir);
    }
  });
});

test.describe("Agents view — marketplace-tracked save flow", () => {
  // Stub a tracked agent on disk so the editor's save flow opens
  // the keep-linked-vs-detach modal. Both subtests share the same
  // setup shape but each creates a fresh tmpdir to keep state
  // disjoint.
  const STUB_AGENT_NAME = "e2e-fixture-agent";
  const ZERO_HASH =
    "blake3:0000000000000000000000000000000000000000000000000000000000000000";

  function stubMarketplaceAgent(activePath: string) {
    const agentsDir = path.join(activePath, ".kiro/agents");
    fs.mkdirSync(agentsDir, { recursive: true });
    fs.writeFileSync(
      path.join(agentsDir, `${STUB_AGENT_NAME}.json`),
      JSON.stringify(
        {
          name: STUB_AGENT_NAME,
          description: "stubbed marketplace agent",
          model: "claude-sonnet-4-5",
          prompt: "Stubbed prompt for the e2e test.",
          tools: [],
          allowedTools: [],
          mcpServers: {},
          resources: [],
          hooks: {},
        },
        null,
        2,
      ),
    );
    fs.writeFileSync(
      path.join(activePath, ".kiro/installed-agents.json"),
      JSON.stringify(
        {
          agents: {
            [STUB_AGENT_NAME]: {
              marketplace: "fixture-market",
              plugin: "fixture-plugin",
              version: "1.0.0",
              installed_at: "2026-05-22T21:00:00.000000Z",
              dialect: "native",
              source_path: `agents/${STUB_AGENT_NAME}.json`,
              source_hash: ZERO_HASH,
              installed_hash: ZERO_HASH,
            },
          },
          native_companions: {},
        },
        null,
        2,
      ),
    );
  }

  test("Keep linked preserves the installed-agents.json entry", async ({
    page,
  }) => {
    const { tmpdir, activePath } = await setupFreshProject(page);
    stubMarketplaceAgent(activePath);
    try {
      // Reload so the list picks up the seeded agent.
      await page.reload();
      await page.getByRole("button", { name: "Agents", exact: true }).click();
      await expect(page.getByText(STUB_AGENT_NAME)).toBeVisible({ timeout: 10_000 });

      await page.getByRole("button", { name: "Edit", exact: true }).click();
      // Wait for load.
      await expect(page.getByPlaceholder("code-reviewer")).toHaveValue(
        STUB_AGENT_NAME,
        { timeout: 10_000 },
      );

      await page.getByRole("button", { name: "Save Changes", exact: true }).click();

      // Modal opens. Keep linked is auto-focused but click it explicitly
      // so the test reads as an intentional choice.
      await expect(
        page.getByRole("dialog", { name: /Keep marketplace link/ }),
      ).toBeVisible({ timeout: 5_000 });
      await page.getByRole("button", { name: "Keep linked", exact: true }).click();

      await expect(page.getByText(/Saved.+e2e-fixture-agent/)).toBeVisible({
        timeout: 10_000,
      });

      // Tracking entry preserved.
      const tracking = JSON.parse(
        fs.readFileSync(
          path.join(activePath, ".kiro/installed-agents.json"),
          "utf8",
        ),
      );
      expect(tracking.agents[STUB_AGENT_NAME]).toBeDefined();
      expect(tracking.agents[STUB_AGENT_NAME].marketplace).toBe("fixture-market");
    } finally {
      cleanup(tmpdir);
    }
  });

  test("Detach removes the installed-agents.json entry", async ({ page }) => {
    const { tmpdir, activePath } = await setupFreshProject(page);
    stubMarketplaceAgent(activePath);
    try {
      await page.reload();
      await page.getByRole("button", { name: "Agents", exact: true }).click();
      await expect(page.getByText(STUB_AGENT_NAME)).toBeVisible({ timeout: 10_000 });

      await page.getByRole("button", { name: "Edit", exact: true }).click();
      await expect(page.getByPlaceholder("code-reviewer")).toHaveValue(
        STUB_AGENT_NAME,
        { timeout: 10_000 },
      );

      await page.getByRole("button", { name: "Save Changes", exact: true }).click();

      await expect(
        page.getByRole("dialog", { name: /Keep marketplace link/ }),
      ).toBeVisible({ timeout: 5_000 });
      await page.getByRole("button", { name: "Detach", exact: true }).click();

      await expect(page.getByText(/Saved.+e2e-fixture-agent/)).toBeVisible({
        timeout: 10_000,
      });

      // Tracking entry gone — the file may now have agents: {}, OR the
      // file might be rewritten with the entry absent. Both states
      // satisfy the contract; assert by `agents` key membership.
      const tracking = JSON.parse(
        fs.readFileSync(
          path.join(activePath, ".kiro/installed-agents.json"),
          "utf8",
        ),
      );
      expect(tracking.agents[STUB_AGENT_NAME]).toBeUndefined();
    } finally {
      cleanup(tmpdir);
    }
  });
});

test.describe("Agents view — IPC validation rejects malformed input (A2)", () => {
  // Per amendment A2: createUserAgent / saveUserAgent wrappers reject
  // non-JSON or oversized payloads at the IPC boundary, BEFORE any
  // filesystem write. The editor's own serializer can never emit
  // non-JSON in practice, so this exercises the wrapper guard
  // directly via window.__TAURI_INTERNALS__.invoke. The wrapper's
  // job is to defend against a compromised or buggy renderer, not
  // against the legitimate happy path.

  test("createUserAgent rejects non-JSON draft with ParseError", async ({
    page,
  }) => {
    const { tmpdir, activePath } = await setupFreshProject(page);
    try {
      const error = await page.evaluate(
        async ([projectPath]) => {
          const w = window as unknown as TauriWindow;
          try {
            await w.__TAURI_INTERNALS__.invoke("create_user_agent", {
              name: "victim",
              draftJson: "{ not valid json",
              projectPath,
            });
            return { thrown: false, error: null };
          } catch (e) {
            return { thrown: true, error: e };
          }
        },
        [activePath],
      );

      expect(error.thrown).toBe(true);
      // The structured CommandError has shape { message, error_type }.
      // The wrapper guard fires for non-JSON before any FS write.
      expect((error.error as { error_type: string }).error_type).toBe(
        "parse_error",
      );

      // Filesystem assertion: no file landed.
      expect(
        fs.existsSync(path.join(activePath, ".kiro/agents/victim.json")),
        "wrapper rejection must happen before any FS write",
      ).toBe(false);
    } finally {
      cleanup(tmpdir);
    }
  });

  test("saveUserAgent rejects non-JSON draft with ParseError", async ({
    page,
  }) => {
    const { tmpdir, activePath } = await setupFreshProject(page);
    try {
      // Seed an existing agent so save_user_agent has a from_name to
      // edit. Without this the call would fail on AgentName::new
      // for from_name first; we want the parse-error path.
      const existingDir = path.join(activePath, ".kiro/agents");
      fs.mkdirSync(existingDir, { recursive: true });
      fs.writeFileSync(
        path.join(existingDir, "existing.json"),
        '{"name":"existing"}',
      );

      const preBytes = fs.readFileSync(
        path.join(existingDir, "existing.json"),
      );

      const error = await page.evaluate(
        async ([projectPath]) => {
          const w = window as unknown as TauriWindow;
          try {
            await w.__TAURI_INTERNALS__.invoke("save_user_agent", {
              fromName: "existing",
              draftName: "existing",
              draftJson: "{ not valid json",
              detach: false,
              projectPath,
            });
            return { thrown: false, error: null };
          } catch (e) {
            return { thrown: true, error: e };
          }
        },
        [activePath],
      );

      expect(error.thrown).toBe(true);
      expect((error.error as { error_type: string }).error_type).toBe(
        "parse_error",
      );

      // Existing file unchanged — the rejection must happen before
      // the atomic write replaces the bytes on disk.
      const postBytes = fs.readFileSync(
        path.join(existingDir, "existing.json"),
      );
      expect(postBytes.equals(preBytes)).toBe(true);
    } finally {
      cleanup(tmpdir);
    }
  });
});

test.describe("Agents view — Tools section round-trip (slice S7)", () => {
  // Exercises slice S5+S6's Tools panel through the save pipeline:
  // toggle a native tool (via the by-category grid checkbox), add an
  // external MCP entry (via the External form), save, and verify the
  // three tool fields round-trip through serde_json::Value untouched.
  //
  // Falsifier shape: a regression where ToolsPanel mutates `draft.tools`
  // but AgentEditor's save handler omits the field from the serialized
  // payload fails the assertion that the saved JSON contains the
  // toggled name. A regression where addExternalTool accidentally
  // appends to `allowedTools[]` (the A1 amendment removed that
  // side-effect) fails the `allowedTools = []` assertion.
  test("create agent, toggle native + add MCP, save, assert JSON shape", async ({
    page,
  }) => {
    const { tmpdir, activePath } = await setupFreshProject(page);
    try {
      await page.getByRole("button", { name: "Agents", exact: true }).click();
      await page.getByRole("button", { name: /Create Agent/ }).click();
      await page.getByPlaceholder("code-reviewer").fill("tools-test");
      await page.getByRole("button", { name: "Create Agent", exact: true }).click();
      await expect(page.getByText(/Created.+tools-test/)).toBeVisible({
        timeout: 10_000,
      });

      // Re-open in edit mode to reach the Tools section.
      await page.getByRole("button", { name: "Edit", exact: true }).click();
      await expect(page.getByPlaceholder("code-reviewer")).toHaveValue(
        "tools-test",
        { timeout: 10_000 },
      );

      // Navigate to Tools section.
      await page.getByRole("button", { name: "Tools", exact: true }).click();
      // The "Available tools" subhead is the marker that ToolsPanel
      // rendered, separating us from a stale Identity / Prompt view.
      await expect(page.getByText("Available tools")).toBeVisible();

      // Toggle a native tool ON. The by-category grid renders each tool
      // as a button with the tool name + its summary. Clicking the
      // button toggles the checkbox.
      await page
        .getByRole("button", { name: /^fs_read.*Read files/ })
        .click();

      // Add an external (MCP) tool via the External form. The form
      // surfaces an input + Add button at the bottom of the section.
      await page
        .getByPlaceholder(/@terraform-mcp\/plan/)
        .fill("@svc/foo");
      await page.getByRole("button", { name: "Add", exact: true }).click();

      // Both visible in the External list.
      await expect(page.getByText("@svc/foo", { exact: true })).toBeVisible();

      // Save.
      await page
        .getByRole("button", { name: "Save Changes", exact: true })
        .click();
      await expect(page.getByText(/Saved.+tools-test/)).toBeVisible({
        timeout: 10_000,
      });

      // Filesystem assertions — the round-trip through serde_json::Value
      // must preserve the three tool fields as the panel emitted them.
      const agentPath = path.join(activePath, ".kiro/agents/tools-test.json");
      const saved = JSON.parse(fs.readFileSync(agentPath, "utf8"));
      expect(saved.tools).toEqual(expect.arrayContaining(["fs_read", "@svc/foo"]));
      // Per A1 amendment: addExternalTool does NOT side-effect
      // allowedTools[]. addAllowed was never invoked in this test
      // (we didn't open the auto-allow picker), so allowedTools is
      // empty.
      expect(saved.allowedTools).toEqual([]);
      // No alias UI exercised — toolAliases stays empty.
      expect(saved.toolAliases ?? {}).toEqual({});
    } finally {
      cleanup(tmpdir);
    }
  });
});
