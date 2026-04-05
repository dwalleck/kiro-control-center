import { commands } from "$lib/bindings";
import type { ProjectInfo, Settings, DiscoveredProject } from "$lib/bindings";

// ---------------------------------------------------------------------------
// Reactive state (exported as $state object — Svelte 5 Pattern A)
//
// Components read properties directly: `store.projectPath`, `store.loading`.
// Property access on the $state proxy is reactive — no getters needed.
// See: https://svelte.dev/docs/svelte/$state#Exporting-state
// ---------------------------------------------------------------------------

export const store = $state({
  projectPath: null as string | null,
  projectInfo: null as ProjectInfo | null,
  projectError: null as string | null,
  settings: { scan_roots: [], last_project: null } as Settings,
  discoveredProjects: [] as DiscoveredProject[],
  loading: true,
});

// ---------------------------------------------------------------------------
// Actions (mutate the store object's properties)
// ---------------------------------------------------------------------------

export async function initialize() {
  store.loading = true;
  store.projectError = null;

  // Load settings.
  const settingsResult = await commands.getSettings();
  if (settingsResult.status === "ok") {
    store.settings = settingsResult.data;
  }

  // Discover projects.
  await refreshProjects();

  // Restore last project if it still exists on disk.
  if (store.settings.last_project) {
    const found = store.discoveredProjects.find(
      (p) => p.path === store.settings.last_project,
    );
    if (found) {
      await selectProject(store.settings.last_project);
    }
  }

  store.loading = false;
}

export async function selectProject(path: string) {
  store.projectError = null;
  const result = await commands.setActiveProject(path);
  if (result.status === "ok") {
    store.projectPath = result.data.path;
    store.projectInfo = result.data;
  } else {
    store.projectError = result.error.message;
  }
}

export async function refreshProjects() {
  const result = await commands.discoverProjects();
  if (result.status === "ok") {
    store.discoveredProjects = result.data;
  }
}

export async function addScanRoot(root: string) {
  const roots = [...(store.settings.scan_roots ?? []), root];
  const result = await commands.saveScanRoots(roots);
  if (result.status === "ok") {
    store.settings = { ...store.settings, scan_roots: roots };
    await refreshProjects();
  }
}

export async function removeScanRoot(root: string) {
  const roots = (store.settings.scan_roots ?? []).filter(
    (r: string) => r !== root,
  );
  const result = await commands.saveScanRoots(roots);
  if (result.status === "ok") {
    store.settings = { ...store.settings, scan_roots: roots };
    await refreshProjects();
  }
}

export function clearProject() {
  store.projectPath = null;
  store.projectInfo = null;
}
