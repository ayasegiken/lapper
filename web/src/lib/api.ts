// lapper-server の API とのやり取り。

import type { SessionInfo } from './bindings/SessionInfo'
import type { Snapshot } from './bindings/Snapshot'

export async function fetchSessions(): Promise<SessionInfo[]> {
  const response = await fetch('/api/sessions')
  if (!response.ok) {
    throw new Error(`GET /api/sessions: ${response.status}`)
  }
  return response.json()
}

export type ConnectionState = 'connecting' | 'open' | 'reconnecting'

/**
 * セッションの計算結果を購読する（SSE）。接続時と、結果が変わるたびに `onSnapshot` が呼ばれる。
 * 接続が切れるとブラウザーが自動でつなぎ直す。戻り値の関数で購読をやめる。
 */
export function subscribe(
  session: number,
  onSnapshot: (snapshot: Snapshot) => void,
  onState: (state: ConnectionState) => void,
): () => void {
  onState('connecting')
  const source = new EventSource(`/api/sessions/${session}/stream`)
  source.onopen = () => onState('open')
  source.onerror = () => onState('reconnecting')
  source.addEventListener('snapshot', (event) => {
    onSnapshot(JSON.parse(event.data))
  })
  return () => source.close()
}
