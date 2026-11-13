<!-- 通過記録の一覧。光電管の反応 1 回ごとに、計測ロジックでの扱いを表示する（新しい順）。 -->
<script lang="ts">
  import type { Disposition } from './bindings/Disposition'
  import type { Run } from './bindings/Run'
  import type { RunId } from './bindings/RunId'
  import type { SessionInfo } from './bindings/SessionInfo'
  import type { CarChoice } from './bindings/CarChoice'
  import type { PassageReport } from './bindings/PassageReport'
  import type { Snapshot } from './bindings/Snapshot'
  import { parseCar } from './car'
  import { formatClock, formatTime, positionName } from './format'
  import { outbox } from './outbox.svelte'

  let {
    session,
    snapshot,
    operating,
  }: { session: SessionInfo; snapshot: Snapshot; operating: boolean } = $props()

  /** 車を直している通過記録（その通過が割り当てられた走行の車を直す）。 */
  let editing = $state<{ passage: number; run: RunId } | null>(null)
  let carInput = $state<number | null>(null)

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

  function describeCar(run: Run): string {
    switch (run.car_source) {
      case 'Queue':
        return `#${run.car}（出走待ちの列から）`
      case 'Manual':
        return run.car == null ? '車なし（指定）' : `#${run.car}（指定）`
      case 'Unassigned':
        return '車なし'
    }
  }

  function setVoid(p: PassageReport, value: boolean) {
    outbox.send(session.id, [{ event: { type: 'SetVoid', passage: p.id, void: value } }])
  }

  function edit(p: PassageReport, run: RunId) {
    editing = { passage: p.id, run }
    carInput = null
  }

  function setCar(car: CarChoice) {
    if (editing === null) return
    outbox.send(session.id, [{ event: { type: 'SetCar', run: editing.run, car } }])
    editing = null
  }

  function setCarFromInput(event: SubmitEvent) {
    event.preventDefault()
    const car = parseCar(carInput)
    if (car !== null) {
      setCar({ type: 'Car', car })
    }
  }
</script>

{#if passages.length === 0}
  <p class="empty">通過記録はまだありません。</p>
{:else}
  <!-- 操作の列で幅が足りない画面では、表だけを横にスクロールさせる -->
  <div class="scroll">
    <table>
      <thead>
        <tr>
          <th>No.</th>
          <th>時刻</th>
          <th>ゲート</th>
          <th>扱い</th>
          {#if operating}<th>操作</th>{/if}
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
            {#if operating}
              {@const d = p.disposition}
              <td class="actions">
                {#if d.type === 'Void'}
                  <button type="button" onclick={() => setVoid(p, false)}>無効を取り消す</button>
                {:else}
                  <button type="button" onclick={() => setVoid(p, true)}>無効にする</button>
                {/if}
                {#if d.type === 'Assigned'}
                  <button type="button" onclick={() => edit(p, d.run)}>車を直す</button>
                {/if}
              </td>
            {/if}
          </tr>
          {#if operating && editing?.passage === p.id}
            {@const run = runs.get(editing.run)}
            <tr class="editor">
              <td colspan="5">
                <form onsubmit={setCarFromInput}>
                  <span>走行 No.{editing.run} の車（いま: {run ? describeCar(run) : '不明'}）</span>
                  <input
                    type="number"
                    inputmode="numeric"
                    min="0"
                    placeholder="車番"
                    aria-label="この走行の車番"
                    bind:value={carInput}
                  />
                  <button disabled={parseCar(carInput) === null}>この車にする</button>
                  <button type="button" onclick={() => setCar({ type: 'NoCar' })}>車なし</button>
                  <button type="button" onclick={() => setCar({ type: 'Auto' })}>列からの割り当てに戻す</button>
                  <button type="button" onclick={() => (editing = null)}>やめる</button>
                </form>
              </td>
            </tr>
          {/if}
        {/each}
      </tbody>
    </table>
  </div>
{/if}

<style>
  .scroll {
    overflow-x: auto;
  }
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
  .actions {
    white-space: nowrap;
  }
  .actions button + button {
    margin-left: 0.25rem;
  }
  tr.editor td {
    background: var(--surface);
  }
  .editor form {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem;
  }
  .editor input {
    width: 6rem;
  }
  .empty {
    color: var(--muted);
  }
</style>
