// 車番の入力。

/** 入力された車番（ゼッケン）を確かめる。CarId は 0 〜 2^32 - 1 の整数。 */
export function parseCar(value: number | null | undefined): number | null {
  if (value == null || !Number.isInteger(value) || value < 0 || value > 0xffff_ffff) {
    return null
  }
  return value
}
