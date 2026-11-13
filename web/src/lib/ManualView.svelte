<!--
  手動計時。計測ボックスや光電管の不具合のときに、ボタンを押した瞬間を、そのゲートの通過として記録する。
  どの車の通過になるかは、光電管の通過と同じく計測ロジックが順番で決める（ボタンには、次に通過すると見込まれる車を出す）。
-->
<script lang="ts">
  import type { SessionInfo } from './bindings/SessionInfo'
  import type { Snapshot } from './bindings/Snapshot'
  import { positionName } from './format'
  import { outbox } from './outbox.svelte'

  let { session, snapshot }: { session: SessionInfo; snapshot: Snapshot } = $props()

  /** ゲートと、そのゲートのポジション（コースの順）。スタートとゴールが同じゲートなら、1 つのゲートに 2 つのポジション。 */
  let gates = $derived.by(() => {
    const result: { gate: number; positions: number[] }[] = []
    session.config.course.forEach((gate, position) => {
      const entry = result.find((g) => g.gate === gate)
      if (entry) {
        entry.positions.push(position)
      } else {
        result.push({ gate, positions: [position] })
      }
    })
    return result
  })
  let positions = $derived(session.config.course.length)

  /** 最後に記録した通過（押したことが分かるように表示する）。 */
  let last = $state<string | null>(null)

  function gateName(gate: number): string {
    return session.gate_names[gate] ?? `ゲート ${gate}`
  }

  /** 次にポジション `position` を通過すると見込まれる車。 */
  function expected(position: number): string {
    if (position === 0) {
      const next = snapshot.outcome.queue[0]
      return next == null ? '列が空（車なしで記録）' : `#${next}`
    }
    // 走行はスタートの時刻順なので、最初に見つかったものがいちばん先にスタートした車
    const run = snapshot.outcome.runs.find(
      (r) =>
        r.status.type === 'Running' && r.passes.slice(position).every((slot) => slot.type !== 'Passed'),
    )
    if (!run) {
      return 'コース上に車なし'
    }
    return run.car == null ? '車なし' : `#${run.car}`
  }

  function pass(gate: number) {
    outbox.send(session.id, [
      {
        event: { type: 'Passage', id: 0, gate, time: 0, source: 'Manual' },
        atNow: true,
        newId: true,
      },
    ])
    last = `${gateName(gate)} の通過を記録しました（${new Date().toLocaleTimeString()}）`
  }
</script>

<p class="help">
  押した瞬間を、そのゲートの通過として記録します（手動の通過）。精度はおよそ 0.1 秒です。
  間違えたときは、通過記録の画面で無効にしてください。
</p>

<div class="gates">
  {#each gates as { gate, positions: at } (gate)}
    <button type="button" class="gate" onclick={() => pass(gate)}>
      <span class="name">{gateName(gate)}</span>
      {#each at as position (position)}
        <span class="expected">{positionName(position, positions)}: {expected(position)}</span>
      {/each}
    </button>
  {/each}
</div>

{#if last}
  <p class="last" role="status">{last}</p>
{/if}

<style>
  .help {
    color: var(--muted);
  }
  .gates {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(10rem, 1fr));
    gap: 0.75rem;
  }
  .gate {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.25rem;
    min-height: 8rem;
    padding: 1rem;
    border-radius: 0.75rem;
  }
  .gate:active {
    background: var(--line);
  }
  .name {
    font-size: 2.5rem;
    font-weight: bold;
    line-height: 1.2;
  }
  .expected {
    color: var(--muted);
  }
  .last {
    font-variant-numeric: tabular-nums;
  }
</style>
