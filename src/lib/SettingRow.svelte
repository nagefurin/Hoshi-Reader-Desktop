<script lang="ts">
  import type { Snippet } from "svelte";

  let { label, compact = false, labelWidth, children }: { label: string; compact?: boolean; labelWidth?: string; children: Snippet } = $props();
</script>

<div class="flex items-center justify-between {compact ? 'gap-2' : 'gap-4'}">
  <span class="text-sm" style:width={labelWidth} style:min-width={labelWidth}>{label}</span>
  {@render children()}
</div>
