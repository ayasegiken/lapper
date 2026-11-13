<!--
  計時担当の画面。手動の通過（計測ボックスや光電管の不具合のときに、ボタンを押した瞬間をそのゲートの通過として記録する）と、
  コース上の車のコースアウトの報告。
  どの車の通過になるかは、光電管の通過と同じく計測ロジックが順番で決める（ボタンには、次に通過すると見込まれる車を出す）。
-->
<script lang="ts">
  import type { Run } from './bindings/Run'
  import type { SessionInfo } from './bindings/SessionInfo'
  import type { Snapshot } from './bindings/Snapshot'
  import { formatClock, positionName } from './format'
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

  let strict = $derived(session.config.mode === 'Strict')
  let running = $derived(snapshot.outcome.runs.filter((r) => r.status.type === 'Running'))
  // 報告済みの走行（新しいものから 5 件）。報告の取り消し用
  let reported = $derived(
    snapshot.outcome.runs
      .filter((r) => r.status.type === 'Off' && r.status.cause.type === 'Reported')
      .toReversed()
      .slice(0, 5),
  )

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
    return run ? carName(run) : 'コース上に車なし'
  }

  function carName(run: Run): string {
    return run.car == null ? '車なし' : `#${run.car}`
  }

  /** 走行が最後に通過したポジション（スタートは必ず通過している）。 */
  function lastPosition(run: Run): number {
    return run.passes.findLastIndex((slot) => slot.type === 'Passed')
  }

  function reportOff(run: Run) {
    const last = lastPosition(run)
    const where = positionName(last, positions)
    if (!confirm(`${carName(run)} を、${where}の後でコースアウト / リタイアしたと報告しますか？`)) return
    outbox.send(session.id, [
      { event: { type: 'ReportOff', run: run.id, last_position: last, at: 0 }, atNow: true },
    ])
  }

  function cancelReport(run: Run) {
    outbox.send(session.id, [{ event: { type: 'CancelReport', run: run.id } }])
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

{#if strict}
  <p class="mode strict">
    このセッションは<strong>そのまま記録</strong>するモードです。通過は届いた順に割り当て、推定はしません。
    誤反応は通過記録の画面で無効にし、コースアウトは下の「コース上の車」ですぐに報告してください。
  </p>
{:else}
  <p class="mode">このセッションは、反応の抜けや誤反応、コースアウトを自動で推定するモードです。</p>
{/if}

<h2>手動の通過</h2>
<p class="help">
  押した瞬間を、そのゲートの通過として記録します。精度はおよそ 0.1 秒です。
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

<h2>コース上の車</h2>
{#if running.length === 0}
  <p class="help">走行中の車はありません。</p>
{:else}
  <ul class="runs">
    {#each running as run (run.id)}
      <li>
        <span class="car">{carName(run)}</span>
        <span class="where">
          {formatClock(run.start_time)} スタート、{positionName(lastPosition(run), positions)}まで通過
        </span>
        <button type="button" onclick={() => reportOff(run)}>コースアウト</button>
      </li>
    {/each}
  </ul>
{/if}

{#if reported.length > 0}
  <h3>報告済み</h3>
  <ul class="runs">
    {#each reported as run (run.id)}
      <li>
        <span class="car">{carName(run)}</span>
        <span class="where">{positionName(lastPosition(run), positions)}の後でコースアウト</span>
        <button type="button" onclick={() => cancelReport(run)}>報告を取り消す</button>
      </li>
    {/each}
  </ul>
{/if}

<style>
  h2 {
    margin: 1.5rem 0 0.5rem;
    font-size: 1.125rem;
  }
  h3 {
    margin: 1rem 0 0.5rem;
    font-size: 1rem;
  }
  .mode {
    margin: 0;
    padding: 0.5rem 0.75rem;
    border-radius: 0.5rem;
    background: var(--surface);
  }
  .mode.strict {
    background: var(--warn-surface);
  }
  .runs {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .runs li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.25rem 0.75rem;
    padding: 0.5rem 0;
    border-bottom: 1px solid var(--line);
  }
  .runs .car {
    min-width: 3rem;
    font-size: 1.5rem;
    font-weight: bold;
    font-variant-numeric: tabular-nums;
  }
  .runs .where {
    flex: 1;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }
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
