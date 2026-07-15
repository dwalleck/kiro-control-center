// The customize drawer's install fan-out: three category batches
// (skills / steering / agents) run in parallel, each wrapping one
// `commands.*` Tauri call. Extracted from BrowseTab.svelte's
// applyDrawerDiff so vitest can cover the wrapper-level error
// composition without jsdom or a Tauri runtime.
//
// Callers inject the IPC calls as closures — this module never imports
// `commands`, matching the injection pattern on PluginActionContext /
// PluginRemoveContext (tests construct fakes directly, no module mocks).

// Mirrors the shape produced by `typedError<T, CommandError>` in
// bindings.ts. Re-declared here so this module stays pure-logic.
type IpcResult<T> =
  | { status: "ok"; data: T }
  | { status: "error"; error: { message: string } };

export type InstallBatch<T> = {
  // Names selected for install in this category. An empty list means
  // "nothing to do" — `call` is never invoked and the payload stays null.
  names: readonly string[];
  call: () => Promise<IpcResult<T>>;
};

export type InstallBatchesResult<S, St, A> = {
  skills: S | null;
  steering: St | null;
  agents: A | null;
  // Wrapper-level (whole-batch) failure text, or null when every batch
  // either succeeded or was empty. When several batches fail, every
  // category's message is joined here — the batches resolve in
  // nondeterministic order, so a single last-writer-wins slot would
  // silently drop all but one failure. Per-item failures are NOT
  // surfaced here — those live inside each category's payload
  // (`failed` lists) and the caller composes them into the summary
  // banner.
  error: string | null;
};

// Run all three install batches in parallel. Each call is independent
// (different tracking files, no shared lock). A wrapper-level failure on
// one category surfaces in `error` but doesn't abort the others — the
// caller's remove loops still run because they're user-requested
// removals. Locking the error-message wording in one place keeps the
// three category strings ("skill", "steering", "agent") from drifting.
export async function runInstallBatches<S, St, A>(
  marketplace: string,
  plugin: string,
  batches: {
    skills: InstallBatch<S>;
    steering: InstallBatch<St>;
    agents: InstallBatch<A>;
  },
): Promise<InstallBatchesResult<S, St, A>> {
  // Accumulate rather than assign: pushes from concurrently-resolving
  // legs never overwrite each other, so a multi-category failure keeps
  // every message. Joined once after all legs settle. Push order follows
  // resolution order (not the fixed category order), which is acceptable
  // — the invariant is that every failing category's message is visible,
  // not that they render in a particular sequence.
  const errors: string[] = [];

  async function runOne<T>(
    category: "skill" | "steering" | "agent",
    batch: InstallBatch<T>,
  ): Promise<T | null> {
    if (batch.names.length === 0) return null;
    try {
      const r = await batch.call();
      if (r.status === "ok") return r.data;
      errors.push(
        `Customize apply: ${category} install failed for ${marketplace}/${plugin}: ${r.error.message}`,
      );
      return null;
    } catch (e) {
      const reason = e instanceof Error ? e.message : String(e);
      errors.push(
        `Customize apply: ${category} install threw for ${marketplace}/${plugin}: ${reason}`,
      );
      return null;
    }
  }

  const [skills, steering, agents] = await Promise.all([
    runOne("skill", batches.skills),
    runOne("steering", batches.steering),
    runOne("agent", batches.agents),
  ]);

  return {
    skills,
    steering,
    agents,
    error: errors.length > 0 ? errors.join(" | ") : null,
  };
}
