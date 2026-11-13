<!-- 通過記録の一覧。光電管の反応 1 回ごとに、計測ロジックでの扱いを表示する（新しい順）。 -->
<script lang="ts">
  import type { Disposition } from './bindings/Disposition'
  import type { Run } from './bindings/Run'
  import type { RunId } from './bindings/RunId'
  import type { SessionInfo } from './bindings/SessionInfo'
  import type { Snapshot } from './bindings/Snapshot'
  import { formatClock, formatTime, positionName } from './format'

  let { session, snapshot }: { session: SessionInfo; snapshot: Snapshot } = $props()

  let passages = $derived(snapshot.outcome.passages.toReversed())
  let runs = $derived(new Map<RunId, Run>(snapshot.outcome.runs.map((r) => [r.id, r])))
  let positions = $derived(session.config.course.length)

  function gateName(gate: number): string {
    return session.gate_names[gate] ?? `ゲート ${gate}`
  }

  function describe(d: Disposition): string {
    switch (d.type) {
      case 'Assigned': {
        const car = runs.get(d.run)?.car
        const who = car == null ? '車なし' : `#${car}`
        return `${who} の${positionName(d.position, positions)}`
      }
      case 'Debounced':
        return `二重反応（No.${d.by} の直後）`
      case 'Void':
        return '無効'
      case 'Orphan':
        return '未割当'
    }
  }
</script>

{#if passages.length === 0}
  <p class="empty">通過記録はまだありません。</p>
{:else}
  <table>
    <thead>
      <tr>
        <th>No.</th>
        <th>時刻</th>
        <th>ゲート</th>
        <th>扱い</th>
      </tr>
    </thead>
    <tbody>
      {#each passages as p (p.id)}
        <tr class={p.disposition.type}>
          <td class="num">{p.id}</td>
          <td class="num">
            {formatClock(p.time)}
            {#if p.time !== p.raw_time}
              <small title="センサーの遅れを補正した量">−{formatTime(p.raw_time - p.time)}</small>
            {/if}
          </td>
          <td>
            {gateName(p.gate)}
            {#if p.source === 'Manual'}<small>手動</small>{/if}
          </td>
          <td>{describe(p.disposition)}</td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

<style>
  table {
    width: 100%;
    border-collapse: collapse;
  }
  th,
  td {
    padding: 0.375rem 0.5rem;
    border-bottom: 1px solid var(--line);
    text-align: left;
  }
  th {
    font-size: 0.875rem;
    color: var(--muted);
  }
  .num {
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  small {
    margin-left: 0.25rem;
    color: var(--muted);
  }
  /* 割り当てられなかった反応を目立たせる */
  tr.Debounced,
  tr.Void {
    color: var(--muted);
  }
  tr.Orphan td {
    background: var(--warn-surface);
  }
  .empty {
    color: var(--muted);
  }
</style>
