// 運営の操作をサーバーに送る。
//
// 送り手の ID（ページを開くたびに作る）と、操作ごとの番号を付けて送る。通信が切れて届いたか分からないときは、
// 同じ番号のまま送り直す（サーバーが届き済みの番号を無視するので、二重には入らない）。

import type { AppendEvents } from './bindings/AppendEvents'
import type { Event } from './bindings/Event'

/**
 * 送る操作。
 * - `atNow`: イベントの時刻（`at`、通過記録なら `time`）を、サーバーが操作した時点のコースの時刻で置き換える。
 * - `newId`: 通過記録の ID を、サーバーがまだ使われていない ID で置き換える。
 */
export interface Operation {
  event: Event
  atNow?: boolean
  newId?: boolean
}

interface Request {
  session: number
  body: AppendEvents
  /** 操作した時刻（`performance.now()`）。送り直したときに、操作してからの時間をサーバーに伝える。 */
  createdAt: number
}

type Result = { type: 'ok' } | { type: 'retry' } | { type: 'rejected'; message: string }

/** 送り直すまでの待ち時間の上限（ミリ秒）。 */
const MAX_DELAY = 5_000

class Outbox {
  /** 送っていない（送り直し中を含む）要求の数。 */
  pending = $state(0)
  /** 送り直しているか（通信できないか、サーバーのエラー）。 */
  retrying = $state(false)
  /** 最後にサーバーに断られた操作の理由。 */
  error = $state<string | null>(null)

  #source = newSourceId()
  #seq = 0
  #queue: Request[] = []
  #running = false

  send(session: number, operations: Operation[]) {
    const events = operations.map((op) => ({
      seq: this.#seq++,
      event: op.event,
      at_now: op.atNow ?? false,
      new_id: op.newId ?? false,
      age_ms: 0,
    }))
    this.#queue.push({
      session,
      body: { source: this.#source, events },
      createdAt: performance.now(),
    })
    this.pending = this.#queue.length
    this.error = null
    if (!this.#running) {
      void this.#run()
    }
  }

  dismissError() {
    this.error = null
  }

  // 1 件ずつ順番に送る（列の操作などは順番に意味があるので、前の要求が届くまで次を送らない）
  async #run() {
    this.#running = true
    let delay = 500
    while (this.#queue.length > 0) {
      const result = await post(this.#queue[0])
      if (result.type === 'retry') {
        this.retrying = true
        await new Promise((resolve) => setTimeout(resolve, delay))
        delay = Math.min(delay * 2, MAX_DELAY)
        continue
      }
      this.retrying = false
      delay = 500
      this.#queue.shift()
      this.pending = this.#queue.length
      if (result.type === 'rejected') {
        this.error = result.message
      }
    }
    this.#running = false
  }
}

async function post({ session, body, createdAt }: Request): Promise<Result> {
  const age = Math.round(performance.now() - createdAt)
  for (const event of body.events) {
    event.age_ms = age
  }
  let response: Response
  try {
    response = await fetch(`/api/sessions/${session}/events`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    })
  } catch {
    // 通信できなかった（届いたかどうか分からない）
    return { type: 'retry' }
  }
  if (response.ok) {
    return { type: 'ok' }
  }
  if (response.status >= 500) {
    return { type: 'retry' }
  }
  const message = (await response.text()) || `HTTP ${response.status}`
  return { type: 'rejected', message }
}

// crypto.randomUUID() は https か localhost でしか使えない（会場ではラズパイに http でつなぐ）ので、乱数から作る
function newSourceId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(8))
  return 'browser-' + Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('')
}

export const outbox = new Outbox()
