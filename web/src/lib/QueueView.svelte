<!-- 出走待ちの列。先頭（次にスタートする車）を大きく表示し、スタート地点の表示を兼ねる。 -->
<script lang="ts">
  import type { SessionInfo } from './bindings/SessionInfo'
  import type { Snapshot } from './bindings/Snapshot'
  import { parseCar } from './car'
  import { outbox } from './outbox.svelte'

  let {
    session,
    snapshot,
    operating,
  }: { session: SessionInfo; snapshot: Snapshot; operating: boolean } = $props()

  let queue = $derived(snapshot.outcome.queue)
  let carInput = $state<number | null>(null)

  function enqueue(event: SubmitEvent) {
    event.preventDefault()
    const car = parseCar(carInput)
    if (car === null) return
    outbox.send(session.id, [{ event: { type: 'Enqueue', car, at: 0 }, atNow: true }])
    carInput = null
  }

  function dequeue(car: number) {
    if (!confirm(`#${car} を出走待ちの列から外しますか？`)) return
    outbox.send(session.id, [{ event: { type: 'Dequeue', car, at: 0 }, atNow: true }])
  }
</script>

{#snippet remove(car: number)}
  {#if operating}
    <button type="button" class="remove" aria-label="#{car} を列から外す" onclick={() => dequeue(car)}>×</button>
  {/if}
{/snippet}

{#if operating}
  <form onsubmit={enqueue}>
    <input
      type="number"
      inputmode="numeric"
      min="0"
      placeholder="車番"
      aria-label="列に入れる車番"
      bind:value={carInput}
      required
    />
    <button disabled={parseCar(carInput) === null}>列に入れる</button>
  </form>
{/if}

{#if queue.length === 0}
  <p class="empty">出走待ちの車はありません。</p>
{:else}
  <section class="next" aria-label="次の車">
    <span class="label">次</span>
    <span class="car">{queue[0]}</span>
    {@render remove(queue[0])}
  </section>
  {#if queue.length > 1}
    <ol start="2">
      {#each queue.slice(1) as car, i (i)}
        <li>{car}{@render remove(car)}</li>
      {/each}
    </ol>
  {/if}
  <p class="count">{queue.length} 台待ち</p>
{/if}

<style>
  form {
    display: flex;
    gap: 0.5rem;
    margin-bottom: 1rem;
  }
  form input {
    width: 7rem;
  }
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
    display: flex;
    align-items: center;
    gap: 0.25rem;
    min-width: 3.5rem;
    padding: 0.5rem 0.75rem;
    border: 1px solid var(--line);
    border-radius: 0.5rem;
    font-size: 1.75rem;
    justify-content: center;
    font-variant-numeric: tabular-nums;
  }
  .remove {
    padding: 0 0.5rem;
    font-size: 1rem;
    line-height: 1.5;
  }
  .count,
  .empty {
    color: var(--muted);
  }
</style>
