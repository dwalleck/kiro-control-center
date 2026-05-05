<script lang="ts" generics="K extends string">
  import type { SvelteMap } from "svelte/reactivity";

  // Extracted from BrowseTab.svelte (the prior render block lived at
  // lines 1066-1132). The shared component owns: 3-cap-with-overflow
  // rendering for the `errors` map, dismiss buttons (calls back via
  // `ondismiss`), and the green/amber/red color treatment for the
  // remaining three banner channels.
  //
  // The `errors` map is generic over its key type via Svelte 5's
  // `generics="K extends string"` script-tag attribute — `K` flows
  // through the dismiss callback so the consumer can branch type-
  // safely on which key was dismissed without a cast.
  type Props = {
    errors: SvelteMap<K, string>;
    message: string | null;
    warning: string | null;
    fatalError: string | null;
    // `errLabel` is per-tab because the screen-reader label depends
    // on the consumer's ErrorSource taxonomy (e.g. "Dismiss error
    // for acme/foo" vs "Dismiss installed-plugins error").
    errLabel: (key: K) => string;
    // Svelte 5 callback prop, not the legacy `on:dismiss` directive.
    ondismiss: (key: K) => void;
    onmessageDismiss?: () => void;
    onwarningDismiss?: () => void;
    onfatalErrorDismiss?: () => void;
  };

  let {
    errors,
    message,
    warning,
    fatalError,
    errLabel,
    ondismiss,
    onmessageDismiss,
    onwarningDismiss,
    onfatalErrorDismiss,
  }: Props = $props();
</script>

<!-- Banners render newest-first (reverse insertion order) and cap at 3 so
     a storm of broken plugins doesn't push the grid off-screen. Dismissing
     a banner or resolving its source surfaces the next-newest below. -->
{#each [...errors].reverse().slice(0, 3) as [key, msg] (key)}
  <div
    data-testid="fetch-error"
    class="mx-4 mt-3 px-4 py-3 rounded-md bg-kiro-error/10 border border-kiro-error/30 flex items-start gap-3"
  >
    <p class="text-sm text-kiro-error flex-1">{msg}</p>
    <button
      type="button"
      onclick={() => ondismiss(key)}
      aria-label={errLabel(key)}
      class="text-kiro-error/70 hover:text-kiro-error text-lg leading-none flex-shrink-0 focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
    >
      ×
    </button>
  </div>
{/each}
{#if errors.size > 3}
  <div
    data-testid="fetch-error-overflow"
    class="mx-4 mt-3 px-4 py-2 text-xs text-kiro-subtle text-center border border-kiro-muted/50 rounded-md bg-kiro-surface/30"
  >
    +{errors.size - 3} more {errors.size - 3 === 1 ? "error" : "errors"} — dismiss or resolve above to see the rest
  </div>
{/if}

{#if fatalError}
  <div
    data-testid="install-error"
    class="mx-4 mt-3 px-4 py-3 rounded-md bg-kiro-error/10 border border-kiro-error/30 flex items-start gap-3"
  >
    <p class="text-sm text-kiro-error flex-1">{fatalError}</p>
    <button
      type="button"
      onclick={() => onfatalErrorDismiss?.()}
      aria-label="Dismiss install error"
      class="text-kiro-error/70 hover:text-kiro-error text-lg leading-none flex-shrink-0 focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
    >
      ×
    </button>
  </div>
{/if}

{#if message}
  <div class="mx-4 mt-3 px-4 py-3 rounded-md bg-kiro-success/10 border border-kiro-success/30 flex items-start gap-3">
    <p class="text-sm text-kiro-success flex-1">{message}</p>
    {#if onmessageDismiss}
      <button
        type="button"
        onclick={() => onmessageDismiss?.()}
        aria-label="Dismiss success message"
        class="text-kiro-success/70 hover:text-kiro-success text-lg leading-none flex-shrink-0 focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
      >
        ×
      </button>
    {/if}
  </div>
{/if}

{#if warning}
  <div
    data-testid="install-warning"
    class="mx-4 mt-3 px-4 py-3 rounded-md bg-kiro-warning/10 border border-kiro-warning/30 flex items-start gap-3"
  >
    <p class="text-sm text-kiro-warning flex-1">{warning}</p>
    <button
      type="button"
      onclick={() => onwarningDismiss?.()}
      aria-label="Dismiss install warning"
      class="text-kiro-warning/70 hover:text-kiro-warning text-lg leading-none flex-shrink-0 focus:outline-none focus:ring-2 focus:ring-kiro-accent-500 rounded"
    >
      ×
    </button>
  </div>
{/if}
