# サーバーと画面

`lapper-server` と `web/` のリファレンス。

## 構成

```
入力元（シミュレーター / 計測ボックス / 運営の操作）
   │ イベント
[lapper-server]
   ├─ SQLite: セッションと、追記だけのイベントのログ
   ├─ 追記のたびに lapper_core::compute で結果を計算し直す
   └─ 結果が変わったら SSE で配る
   │
[ブラウザー（web/）]
```

- 1 回の計測の単位を**セッション**と呼ぶ（ヒートや練習会の 1 枠）。セッションは計測ロジックの設定（`Config`）と、
  ゲートの表示名と、イベントのログを持つ。
- イベントは書き換えも削除もしない。訂正も新しいイベントとして追記する（[timing-logic.md](timing-logic.md)）。
- サーバーを再起動すると、保存されているログから結果を計算し直す。

## 起動

```bash
cargo run --release -p lapper-server -- [options]
```

| オプション | 内容 | 既定値 |
|---|---|---|
| `--db PATH` | SQLite のファイル | `lapper.db` |
| `--listen ADDR` | 待ち受けるアドレス | `0.0.0.0:3000` |
| `--web DIR` | 画面のビルド結果。なければ API だけを提供する | `web/dist` |
| `--simulate NAME` | 新しいセッションを作り、シミュレーターで生成したイベントを流し込む（`gymkhana` / `shared` / `dirt`） | なし |
| `--faults NAME` | シミュレーションの障害の程度（`none` / `light` / `heavy`） | `light` |
| `--seed N` | シミュレーションの乱数のシード | `1` |
| `--speed X` | シミュレーションの再生速度（倍） | `1` |

シミュレーションは、各イベントが届いた時刻になったら追記し、コースの時計を 0.1 秒ごとに知らせる。

## コースの時計

時刻はすべてコースの時計（計測ボックスの時計）のマイクロ秒。サーバーの時計とは別のもの。

入力元は、コースの時計の現在の時刻を定期的に知らせる（`POST /api/sessions/{id}/clock`）。
最大走行時間などの時間切れは、この時刻を基準に判断する（`compute` の `now`）。時刻は戻らない（古い時刻は無視する）。

## API

| メソッド | パス | 本文 | 応答 |
|---|---|---|---|
| GET | `/api/sessions` | | `SessionInfo[]`（新しい順） |
| POST | `/api/sessions` | `NewSession` | `SessionInfo`（201） |
| GET | `/api/sessions/{id}` | | `SessionInfo` |
| POST | `/api/sessions/{id}/events` | `AppendEvents` | `Appended`（新しく追記した数と、追記後のログの数） |
| POST | `/api/sessions/{id}/clock` | `ClockSync` | 204 |
| GET | `/api/sessions/{id}/snapshot` | | `Snapshot` |
| GET | `/api/sessions/{id}/stream` | | SSE |

- エラーは、セッションがない場合 404、入力の内容がおかしい場合 400 か 422。
- `/api` 以外のパスでは、画面のファイル（`--web`）を返す。`Cache-Control: no-cache` を付ける。

### イベントの追記と再送

イベントを追記する入力元（**送り手**）は、自分の ID（`source`）を持ち、イベントごとに自分の中で重ならない番号（`seq`）を付ける。
サーバーは送り手と番号の組で重複を除き、届き済みの組のイベントは無視する（エラーにはしない）。

- 応答が返ってこず届いたか分からないときは、同じ番号のまま送り直せばよい。二重には入らない。
- 番号は連続していなくても、順番どおりでなくてもよい。0 〜 2^63 - 1。
- 送り手の ID の例: 計測ボックスなら機器ごとの ID、ブラウザーならページを開くたびに作るランダムな ID（UUID など）と、
  そのページの中で数える番号。シミュレーター（`--simulate`）は `simulator` で、番号はログの中での位置。
- 重複の判定はセッションごと。再起動しても、届き済みの組は保存されたログから分かる。

```json
{
  "source": "box-1",
  "events": [
    { "seq": 41, "event": { "type": "Passage", "id": 41, "gate": 0, "time": 10000000, "source": "Sensor" } }
  ]
}
```

### SSE

`/api/sessions/{id}/stream` は、接続した時点の結果と、その後結果が変わるたびに、`snapshot` イベントで `Snapshot` の JSON を送る。
結果が変わらない時刻の通知では送らない。

```
event: snapshot
data: {"session":1,"events":52,"outcome":{...}}
```

### JSON の形

型は `lapper-core` と `lapper-server` の Rust の型そのもの。フィールド名は Rust と同じ（`snake_case`）。

- ID（`GateId`、`CarId`、`PassageId`、`RunId`）は数値。
- 時刻と時間（`Micros`）はマイクロ秒の数値。
- 列挙型は `type` で種類を表す。例: `{ "type": "Passage", "id": 1, "gate": 0, "time": 10000000, "source": "Sensor" }`、
  `{ "type": "Time", "time": 62345000 }`。中身を持たない種類だけの列挙型（`PassageSource`、`CarSource`）は文字列。
- コース（`Course`）はゲートの ID の配列。ポジションが 2 つ未満なら受け付けない。
- `Config.gates` のようにゲートをキーにした対応表は、キーが数字の文字列のオブジェクト（`{ "3": {...} }`）。

例: イベントの追記

```bash
curl -X POST http://localhost:3000/api/sessions/1/events \
  -H 'Content-Type: application/json' \
  -d '{"source":"curl","events":[{"seq":0,"event":{"type":"Enqueue","car":7,"at":0}}]}'
```

## TypeScript の型

画面で使う型は、Rust の型から [ts-rs](https://github.com/Aleph-Alpha/ts-rs) で生成し、`web/src/lib/bindings/` に置く（リポジトリに含める）。
Rust の型を変えたら、次のコマンドで生成し直す。

```bash
cargo test -p lapper-server
```

設定は `.cargo/config.toml`（出力先と、`i64` を `number` にする設定）。

## 保存形式（SQLite）

| テーブル | 列 |
|---|---|
| `sessions` | `id`、`name`、`config`（`Config` の JSON）、`gate_names`（JSON）、`created_at`（UNIX 時刻、ミリ秒） |
| `events` | `session_id`、`seq`（セッションの中での連番）、`source` と `source_seq`（送り手の ID と、送り手が付けた番号。セッションの中で一意）、`received_at`（サーバーが受け取った UNIX 時刻、ミリ秒）、`event`（`Event` の JSON） |

ジャーナルは WAL。

形式の版を `PRAGMA user_version` に持つ（今は `1`）。版が合わないデータベースは、起動時にエラーにする（変換はしない）。

## 画面（web/）

Svelte 5 + TypeScript + Vite。

| 画面 | URL | 内容 |
|---|---|---|
| 出走待ち | `#queue` | 出走待ちの列。先頭（次にスタートする車）を大きく表示する |
| 通過記録 | `#passages` | 通過記録を新しい順に。時刻、ゲート、扱い（どの車のどのポジションか、二重反応で無視、無効、未割当） |

ヘッダーでセッションを選ぶ（既定は最新）。セッションの一覧は 10 秒ごとに読み直す。

### 開発

画面を変更しながら確かめるときは、サーバーと Vite の開発サーバーを別々に起動する。Vite は `/api` をサーバー（`127.0.0.1:3000`）に中継する。

```bash
cargo run -p lapper-server -- --simulate gymkhana --speed 5
```

```bash
cd web && pnpm dev
```

型のチェックとビルド:

```bash
cd web && pnpm check && pnpm build
```
