<!-- 出走待ちの列。先頭（次にスタートする車）を大きく表示し、スタート地点の表示を兼ねる。 -->
<script lang="ts">
  import type { Snapshot } from './bindings/Snapshot'

  let { snapshot }: { snapshot: Snapshot } = $props()

  let queue = $derived(snapshot.outcome.queue)
</script>

{#if queue.length === 0}
  <p class="empty">出走待ちの車はありません。</p>
{:else}
  <section class="next" aria-label="次の車">
    <span class="label">次</span>
    <span class="car">{queue[0]}</span>
  </section>
  {#if queue.length > 1}
    <ol start="2">
      {#each queue.slice(1) as car, i (i)}
        <li>{car}</li>
      {/each}
    </ol>
  {/if}
  <p class="count">{queue.length} 台待ち</p>
{/if}

<style>
  .next {
    display: flex;
    align-items: baseline;
    gap: 1rem;
    padding: 1rem 1.5rem;
    border-radius: 0.75rem;
    background: var(--surface);
  }
  .label {
    font-size: 1.5rem;
    color: var(--muted);
  }
  .car {
    font-size: clamp(4rem, 20vw, 10rem);
    font-weight: bold;
    line-height: 1;
    font-variant-numeric: tabular-nums;
  }
  ol {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    padding: 0;
    list-style: none;
  }
  li {
    min-width: 3.5rem;
    padding: 0.5rem 0.75rem;
    border: 1px solid var(--line);
    border-radius: 0.5rem;
    font-size: 1.75rem;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  .count,
  .empty {
    color: var(--muted);
  }
</style>
