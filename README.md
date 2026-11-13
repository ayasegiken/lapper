# lapper

ダートトライアル・ジムカーナなどのタイムトライアル競技向けのラップタイム計測システムです（開発初期段階）。

A lap timing system for time trial motorsports such as dirt trial and gymkhana (autocross-style) events. Early development.

## 構成

| クレート | 内容 |
|---|---|
| [`lapper-core`](crates/lapper-core) | 計測ロジック: コース定義、通過の割り当て、中間計時、未割当の走行、ペナルティ、コースアウトの処理 |
| [`lapper-sim`](crates/lapper-sim) | シミュレーター: 障害（反応の抜け、二重反応、誤反応、コースアウト、列の取り違え）を入れたセッションを作り、計測ロジックの結果を正解と照らし合わせる |
| [`lapper-server`](crates/lapper-server) | Web サーバー: イベントのログを SQLite に保存し、計算結果をブラウザーに SSE で配る。計測ボックスからの入力を TCP で受ける |
| [`lapper-protocol`](crates/lapper-protocol) | 計測ボックスとサーバーのやり取りの形式 |
| [`lapper-vbox`](crates/lapper-vbox) | 仮想の計測ボックス: キーボードの数字キーを光電管の入力に見立てて、本物と同じ形式でサーバーに送る |

画面（Svelte + TypeScript）は [`web/`](web) にあります。

計測ロジックの詳細は [docs/timing-logic.md](docs/timing-logic.md)、サーバーと画面は [docs/server.md](docs/server.md)、
計測ボックスとのやり取りは [docs/box-protocol.md](docs/box-protocol.md) を参照してください。

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
| `--logic-max-on-course N\|none` | 計測ロジックの同時走行の上限だけを上書き（シミュレーションのスターターはプリセットのまま） |
| `--logic-max-segment R\|none` | 計測ロジックの最大区間タイムを、標準タイムに対する割合で上書き |

### サーバーと画面

画面をビルドしてから、シミュレーターの入力でサーバーを起動します（Node.js と pnpm が必要）。

```bash
cd web && pnpm install && pnpm build && cd ..
```

```bash
cargo run --release -p lapper-server -- --simulate gymkhana --speed 5
```

ブラウザーで http://localhost:3000 を開きます。詳しくは [docs/server.md](docs/server.md) を参照してください。

### 仮想の計測ボックス

ハードウェアの代わりに、キーボードから通過を入れられます。サーバーを、ボックスの入力を受ける設定と空のセッションで起動します。

```bash
cargo run --release -p lapper-server -- --box 127.0.0.1:4000 --new-session gymkhana
```

別の端末で仮想ボックスを起動し、数字キーを押すと、その入力端子の通過として送ります（`1` が入力 0）。

```bash
cargo run --release -p lapper-vbox -- --server 127.0.0.1:4000
```

| キー | 内容 |
|---|---|
| `1`〜`9` | 入力 0〜8 の通過（`--inputs N` で使う数を決める。既定は 4） |
| `d` | 切断 / 再接続（切断中の通過は溜めておき、再接続したら送り直す） |
| `q`、`Esc`、`Ctrl-C` | 終了 |

計測ボックスがなくても、画面の「手動計時」（ヘッダーの「操作する」を入れると出る）から、ボタンで通過を記録できます。

## ライセンス

[MIT](LICENSE)
