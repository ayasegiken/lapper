# lapper

ダートトライアル・ジムカーナなどのタイムトライアル競技向けのラップタイム計測システムです（開発初期段階）。

A lap timing system for time trial motorsports such as dirt trial and gymkhana (autocross-style) events. Early development.

## 構成

| クレート | 内容 |
|---|---|
| [`lapper-core`](crates/lapper-core) | 計測ロジック: コース定義、通過の割り当て、中間計時、未割当の走行、ペナルティ、コースアウトの処理 |
| [`lapper-sim`](crates/lapper-sim) | シミュレーター: 障害（反応の抜け、二重反応、誤反応、コースアウト、列の取り違え）を入れたセッションを作り、計測ロジックの結果を正解と照らし合わせる |

計測ロジックの詳細は [docs/timing-logic.md](docs/timing-logic.md) を参照してください。

## 使い方

```bash
cargo test
```

```bash
cargo run --release -p lapper-sim -- --preset gymkhana --faults light --seed 1 --verbose
```

```bash
cargo run --release -p lapper-sim -- --preset dirt --faults heavy --sweep 200
```

| オプション | 内容 |
|---|---|
| `--preset` | `gymkhana`（スタート / 中間×2 / ゴール、最大 4 台）、`shared`（スタートとゴールが同じ光電管、1 台ずつ）、`dirt`（長いコース、最大 2 台） |
| `--faults` | `none` / `light` / `heavy` |
| `--seed N` / `--sweep N` | 乱数のシード / シード 1〜N をまとめて集計 |
| `--verbose` / `--passages` | 走行ごと / 通過記録ごとの結果を表示 |

## ライセンス

[MIT](LICENSE)
