// 表示用の整形。時刻と時間はすべてマイクロ秒（Rust の `Micros`）。

/** 時間を `秒.ミリ秒`（例: `63.042`）にする。Rust の `format_time` と同じ。 */
export function formatTime(micros: number): string {
  const sign = micros < 0 ? '-' : ''
  const ms = Math.floor(Math.abs(micros) / 1000)
  return `${sign}${Math.floor(ms / 1000)}.${String(ms % 1000).padStart(3, '0')}`
}

/** コースの時計の時刻を `時:分:秒.ミリ秒`（1 時間未満は `分:秒.ミリ秒`）にする。 */
export function formatClock(micros: number): string {
  const ms = Math.floor(micros / 1000)
  const h = Math.floor(ms / 3_600_000)
  const m = Math.floor(ms / 60_000) % 60
  const s = Math.floor(ms / 1000) % 60
  const rest = `${String(s).padStart(2, '0')}.${String(ms % 1000).padStart(3, '0')}`
  return h > 0 ? `${h}:${String(m).padStart(2, '0')}:${rest}` : `${m}:${rest}`
}

/** コースのポジションの名前（スタート、中間 1、ゴール）。 */
export function positionName(position: number, positions: number): string {
  if (position === 0) return 'スタート'
  if (position === positions - 1) return 'ゴール'
  return `中間 ${position}`
}
