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
  // category's message is joined here in fixed category order
  // (skill, steering, agent) — a single last-writer-wins slot would
  // silently drop all but one failure, and resolution-order joining
  // would render the survivors in a nondeterministic sequence.
  // Per-item failures are NOT surfaced here — those live inside each
  // category's payload (`failed` lists) and the caller composes them
  // into the summary banner.
  error: string | null;
};

// One leg's settled outcome: the typed payload on success, or the
// composed failure text. Exactly one side is non-null unless the leg
// was empty (both null). Returned per-leg (rather than pushed into a
// shared accumulator) so error assembly happens once, after
// Promise.all, in a deterministic order.
type LegOutcome<T> = {
  data: T | null;
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
  async function runOne<T>(
    category: "skill" | "steering" | "agent",
    batch: InstallBatch<T>,
  ): Promise<LegOutcome<T>> {
    if (batch.names.length === 0) return { data: null, error: null };
    try {
      const r = await batch.call();
      if (r.status === "ok") return { data: r.data, error: null };
      return {
        data: null,
        error: `Customize apply: ${category} install failed for ${marketplace}/${plugin}: ${r.error.message}`,
      };
    } catch (e) {
      const reason = e instanceof Error ? e.message : String(e);
      return {
        data: null,
        error: `Customize apply: ${category} install threw for ${marketplace}/${plugin}: ${reason}`,
      };
    }
  }

  const [skillsLeg, steeringLeg, agentsLeg] = await Promise.all([
    runOne("skill", batches.skills),
    runOne("steering", batches.steering),
    runOne("agent", batches.agents),
  ]);

  // Assemble errors once, after every leg has settled, in fixed
  // category order. Each leg returns its own outcome — no shared
  // mutable accumulator — so concurrently-resolving legs can neither
  // overwrite each other nor scramble the joined message's order.
  const errors = [skillsLeg.error, steeringLeg.error, agentsLeg.error].filter(
    (e): e is string => e !== null,
  );

  return {
    skills: skillsLeg.data,
    steering: steeringLeg.data,
    agents: agentsLeg.data,
    error: errors.length > 0 ? errors.join(" | ") : null,
  };
}
