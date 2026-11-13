<script lang="ts">
  import { fetchSessions, subscribe, type ConnectionState } from './lib/api'
  import { outbox } from './lib/outbox.svelte'
  import type { SessionInfo } from './lib/bindings/SessionInfo'
  import type { Snapshot } from './lib/bindings/Snapshot'
  import QueueView from './lib/QueueView.svelte'
  import PassagesView from './lib/PassagesView.svelte'
  import ManualView from './lib/ManualView.svelte'

  const views = [
    { hash: '#queue', label: '出走待ち' },
    { hash: '#passages', label: '通過記録' },
  ]
  // 「操作する」のときだけ出す
  const manualView = { hash: '#manual', label: '手動計時' }

  let sessions = $state<SessionInfo[]>([])
  let selectedId = $state<number | null>(null)
  // 結果は丸ごと差し替えるだけなので、中身まで追跡しない $state.raw にする
  let snapshot = $state.raw<Snapshot | null>(null)
  let connection = $state<ConnectionState>('connecting')
  let loadError = $state<string | null>(null)
  let hash = $state(location.hash || '#queue')
  // 運営の操作のボタンを出すか。見るだけの人が誤って押さないよう、既定では出さない（権限の仕組みではない）
  let operating = $state(loadOperating())

  let session = $derived(sessions.find((s) => s.id === selectedId) ?? null)

  async function loadSessions() {
    try {
      sessions = await fetchSessions()
      loadError = null
      // 未選択なら最新のセッションを選ぶ
      if (selectedId === null && sessions.length > 0) {
        selectedId = sessions[0].id
      }
    } catch (e) {
      loadError = String(e)
    }
  }

  function loadOperating(): boolean {
    try {
      return localStorage.getItem('lapper.operating') === 'true'
    } catch {
      return false
    }
  }

  $effect(() => {
    try {
      localStorage.setItem('lapper.operating', String(operating))
    } catch {
      // 保存できなくても、このページの中では使える
    }
  })

  // 起動時と、その後 10 秒ごとにセッションの一覧を読み直す（新しいセッションに気付くため）
  $effect(() => {
    loadSessions()
    const timer = setInterval(loadSessions, 10_000)
    return () => clearInterval(timer)
  })

  // 選んだセッションの結果を購読する。選び直すと、前の購読をやめてから新しく購読する
  $effect(() => {
    if (selectedId === null) return
    snapshot = null
    return subscribe(
      selectedId,
      (s) => (snapshot = s),
      (state) => (connection = state),
    )
  })
</script>

<svelte:window onhashchange={() => (hash = location.hash || '#queue')} />

<header>
  <h1>lapper</h1>
  {#if sessions.length > 0}
    <select bind:value={selectedId} aria-label="セッション">
      {#each sessions as s (s.id)}
        <option value={s.id}>{s.id}. {s.name}</option>
      {/each}
    </select>
  {/if}
  <label class="operating">
    <input type="checkbox" bind:checked={operating} />
    操作する
  </label>
  {#if outbox.pending > 0}
    <span class="sending" class:retrying={outbox.retrying}>
      {outbox.retrying ? '送り直しています' : '送信中'}（{outbox.pending}）
    </span>
  {/if}
  <span class="connection {connection}">
    {connection === 'open' ? '接続中' : connection === 'connecting' ? '接続しています' : '再接続しています'}
  </span>
</header>

{#if outbox.error}
  <p class="rejected" role="alert">
    操作を受け付けられませんでした（{outbox.error}）
    <button type="button" onclick={() => outbox.dismissError()}>閉じる</button>
  </p>
{/if}

<nav>
  {#each operating ? [...views, manualView] : views as view (view.hash)}
    <a href={view.hash} class:active={hash === view.hash}>{view.label}</a>
  {/each}
</nav>

<main>
  {#if loadError}
    <p class="message">サーバーに接続できません（{loadError}）</p>
  {:else if sessions.length === 0}
    <p class="message">セッションがありません。</p>
  {:else if !session || !snapshot}
    <p class="message">読み込んでいます…</p>
  {:else if hash === '#passages'}
    <PassagesView {session} {snapshot} {operating} />
  {:else if hash === '#manual'}
    {#if operating}
      <ManualView {session} {snapshot} />
    {:else}
      <p class="message">手動計時は、ヘッダーの「操作する」を入れると使えます。</p>
    {/if}
  {:else}
    <QueueView {session} {snapshot} {operating} />
  {/if}
</main>

<style>
  header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 0.5rem 1rem;
    padding: 0.75rem 1rem;
    border-bottom: 1px solid var(--line);
  }
  h1 {
    margin: 0;
    font-size: 1.25rem;
  }
  select {
    max-width: 100%;
    font: inherit;
  }
  .operating {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    margin-left: auto;
    font-size: 0.875rem;
  }
  .sending {
    font-size: 0.875rem;
    color: var(--muted);
  }
  .sending.retrying {
    color: var(--warn);
  }
  .rejected {
    margin: 0;
    padding: 0.5rem 1rem;
    background: var(--warn-surface);
  }
  .connection {
    font-size: 0.875rem;
    color: var(--muted);
  }
  .connection.reconnecting {
    color: var(--warn);
  }
  nav {
    display: flex;
    gap: 0.25rem;
    padding: 0.5rem 1rem 0;
    border-bottom: 1px solid var(--line);
  }
  nav a {
    padding: 0.5rem 1rem;
    border-radius: 0.375rem 0.375rem 0 0;
    color: inherit;
    text-decoration: none;
  }
  nav a.active {
    background: var(--surface);
    font-weight: bold;
  }
  main {
    padding: 1rem;
  }
  .message {
    color: var(--muted);
  }
</style>
