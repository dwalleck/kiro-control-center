import { commands } from "$lib/bindings";
import type { ProjectInfo, Settings, DiscoveredProject } from "$lib/bindings";

// ---------------------------------------------------------------------------
// Reactive state (exported as a const $state object — Svelte 5 module pattern)
//
// We export a const object rather than reassignable $state variables because
// the Svelte compiler only transforms $state references within the declaring
// file. Importing a reassignable $state variable from another module yields
// the raw signal object, not the reactive value.
//
// By exporting a const object, property mutations (e.g. store.loading = true)
// go through the deep state proxy and trigger reactive updates in any
// component that reads those properties.
//
// See: https://svelte.dev/docs/svelte/$state ("Passing state across modules")
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

  try {
    // Load settings.
    const settingsResult = await commands.getSettings();
    if (settingsResult.status === "ok") {
      store.settings = settingsResult.data;
    } else {
      console.error("Failed to load settings:", settingsResult.error.message);
      store.projectError = `Could not load settings: ${settingsResult.error.message}`;
      return; // Don't continue with dependent operations.
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
  } catch (e) {
    console.error("Initialization failed unexpectedly:", e);
    store.projectError = "Failed to initialize. Please restart the application.";
  } finally {
    store.loading = false;
  }
}

export async function selectProject(path: string) {
  store.projectError = null;
  const result = await commands.setActiveProject(path);
  if (result.status === "ok") {
    store.projectPath = result.data.path;
    store.projectInfo = result.data;
  } else {
    // Clear stale project state so the UI doesn't show the old project.
    store.projectPath = null;
    store.projectInfo = null;
    store.projectError = result.error.message;
  }
}

export async function refreshProjects() {
  const result = await commands.discoverProjects();
  if (result.status === "ok") {
    store.discoveredProjects = result.data;
  } else {
    console.error("Failed to discover projects:", result.error.message);
    store.projectError = `Could not scan for projects: ${result.error.message}`;
  }
}

export async function addScanRoot(root: string) {
  const existing = store.settings.scan_roots ?? [];
  if (existing.includes(root)) return; // Already added
  const roots = [...existing, root];
  const result = await commands.saveScanRoots(roots);
  if (result.status === "ok") {
    store.settings = { ...store.settings, scan_roots: roots };
    await refreshProjects();
  } else {
    console.error("Failed to save scan roots:", result.error.message);
    store.projectError = `Could not save settings: ${result.error.message}`;
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
  } else {
    console.error("Failed to save scan roots:", result.error.message);
    store.projectError = `Could not save settings: ${result.error.message}`;
  }
}

export function clearProject() {
  store.projectPath = null;
  store.projectInfo = null;
}
