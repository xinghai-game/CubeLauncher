<script lang="ts">
  import type { Snippet } from 'svelte';
  import { ChevronLeft, ChevronRight } from 'lucide-svelte';
  import { ui, type Route } from '../lib/state.svelte';
  import { back, go } from '../lib/actions';

  /** Crumb trail of the current page; every entry except the last can be clicked. */
  let { items, actions }: {
    items: { label: string; to?: Route }[];
    actions?: Snippet;
  } = $props();
</script>

<nav class="breadcrumb">
  <button class="back-button" title="返回上一级" disabled={ui.history.length === 0} onclick={back}>
    <ChevronLeft size={16}/>
  </button>
  {#each items as item, index (index)}
    {#if index > 0}<ChevronRight class="crumb-sep" size={13}/>{/if}
    {#if item.to && index < items.length - 1}
      <button class="crumb link" onclick={() => item.to && go(item.to)}>{item.label}</button>
    {:else}
      <span class="crumb current">{item.label}</span>
    {/if}
  {/each}
  <span class="crumb-spacer"></span>
  {@render actions?.()}
</nav>
