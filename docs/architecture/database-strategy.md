---
scope    : Database selection and migration strategy for Event Store, Station inventory, and Market
audience : AI Agent / Human Developer
update   : When persistence ownership, deployment topology, consistency requirements, or database adapters change
related  : architecture.md, ../adr/ADR-0003-local-first-development.md, ../adr/ADR-0017-snapshot-compaction.md, ../adr/ADR-0034-economy-foundations.md, ../adr/ADR-0038-station-inventory-sqlite.md
---

# Dawn Database Strategy

## 1. 結論

SQLite は、現在の Dawn における **Sector-local な Station inventory** と
**単一 `MarketRuntime` が所有する Market** には適している。今すぐ別の DB へ
移行しない。

ただし、SQLite を Dawn 全体の最終 DB とみなさない。用途ごとに次の方針を採る。

| 用途 | 現在の方針 | 将来の第一候補 |
|---|---|---|
| Sector Event Log | `FileEventStore` の append-only 2層ログを継続 | 現行方式を継続。DB 化は別の必要性が生じた場合だけ再評価 |
| Station inventory | Sector node ごとのローカル SQLite を継続 | 原則 SQLite 継続 |
| 単一プロセスの Market | SQLite を継続 | SQLite 継続 |
| 複数プロセスから共有する Market | 現在は対象外 | PostgreSQL |
| 世界全体の分散トランザクション | 現在は導入しない | 実測要件を得てから別 ADR で設計 |

重要なのは「どの DB が最も高性能か」ではなく、**誰が書き込みを所有するか**、
**ネットワーク越しに共有するか**、**どの状態を同一トランザクションに含める必要が
あるか**である。

この文書は現行 ADR を置き換えない。特に Station inventory の現在の挙動は
[ADR-0038](../adr/ADR-0038-station-inventory-sqlite.md) が権威を持つ。下記の整合性改善を
実装する場合は、ADR-0038 と Event Workflow に影響するため、新しい ADR と人間の承認が
必要になる。

## 2. 現在の永続化トポロジー

### 2.1 Sector Event Log

Sector の因果履歴は `dawn-event-store` が所有する。ホットログ、検証済み
スナップショット、コールドアーカイブの2層構成であり、通常復旧は
「スナップショット + ホットログ末尾」で行う（ADR-0017）。

この用途は順次追記が中心であり、現在のバイナリ append-only log は要件に合っている。
SQL検索や複数 writer を必要としていないため、「他の状態が SQLite だから」という理由
だけで Event Store を RDBMS へ移さない。

### 2.2 Station inventory

各 `SimulationNode` は自ノードの SQLite ファイルをローカルに開く。SQLite が耐久状態を
持ち、メモリには直近に触れた `(PlayerId, StationId)` だけを有界キャッシュする。

この構成が SQLite に適している理由:

- DB ファイルと writer が同一ホストにある
- `SimulationNode` が単一接続を所有し、writer の競合がない
- 読み取りはキャッシュ miss 時だけである
- 書き込みは Build / Assemble / Disassemble / Transfer などの低頻度操作である
- 外部 DB サーバーなしでテスト、ローカル実行、Raspberry Pi 配置を維持できる

同じ SQLite ファイルをネットワークファイルシステムに置き、複数ノードから直接開いては
ならない。SQLite のファイルロックと WAL は同一ホストでの利用を前提にする。

### 2.3 Market

`dawn-market::MarketDb` は注文帳、Currency、Bid の escrow を一つの SQLite DB に置く。
発注とキャンセルは DB transaction 内で処理されるため、**Market DB 内部だけ**を見れば
必要な原子性を得られている。

現在は一つの `MarketRuntime` が一つの接続を所有するので、SQLite の single-writer 特性は
制約になっていない。Market が `dawn-simulation` 内の単一 runtime である間は、この単純さを
維持する。現行の `orders` schemaでは `ship_id` を `NOT NULL` で保持する。9D-4以前の
pre-release SQLiteファイルは移行対象にせず、削除してcurrent schemaで再作成する。

## 3. SQLite の限界と移行トリガー

SQLite は同時に複数 reader を扱えるが、一つの DB ファイルに対する writer は同時に一つ
だけである。したがってデータ量だけではなく、**writer の所有形態**を移行判断に使う。

次のいずれかが現実の要件になったら、Market を PostgreSQL へ移す ADR を起票する。

- 複数の server / simulation プロセスが同じ Market へ書き込む
- Market を独立したネットワークプロセスとして運用する
- Market writer のフェイルオーバーまたは無停止運用が必要になる
- writer 待ちが実測上のレイテンシまたは throughput ボトルネックになる
- バックアップ、監視、PITR、RPO/RTO を外部 DB の運用機能で保証する必要がある
- DB ファイルを別ホストや共有ファイルシステムへ置きたくなる

最後の条件では SQLite ファイル共有に進まず、client/server DB に移る。SQLite 公式も、
ネットワーク越しの共有と多数の同時 writer には client/server DB を推奨している。

- [SQLite: Appropriate Uses For SQLite](https://sqlite.org/whentouse.html)
- [SQLite: Write-Ahead Logging](https://sqlite.org/wal.html)

## 4. DB 製品より先に解くべき整合性問題

### 4.1 Station inventory と Event Log の二重書き込み

現行の Station 操作は SQLite と `FileEventStore` という二つの耐久先に書き込む。
ADR-0038 が記録している通り、SQLite 更新後かつ Event append 前にクラッシュすると、
たとえば inventory にアイテムが増えた一方で、対応する Event が存在せず Ship が残る
可能性がある。

これは SQLite 固有の問題ではない。SQLite を PostgreSQL に置き換えても、別ファイルの
Event Log との間に一つの transaction は作れないため、同じ不整合ウィンドウが残る。

したがって DB 移行より先に、次のどちらを採るかを別 ADR で決める。

#### 選択肢 A: Event-first の耐久 Projection

Event Log を真実の情報源とし、Station inventory DB を再適用可能な耐久 Projection とする。

1. Event を耐久 append する
2. commit 済み Event を SQLite に適用する
3. SQLite に `last_applied_log_index` または一意な `operation_id` を記録する
4. 再起動時は未適用 Event だけを冪等に適用する

この方式は通常の Event Workflow と整合し、全 inventory をメモリへロードせずに済む。
一方、現在の `EventStore::append` のエラー表現、Projection 用の Event、compaction 後の
catch-up 起点を設計し直す必要がある。

#### 選択肢 B: 同一 DB transaction に統合

Sector Event と Station inventory 更新を同じ SQLite DB の transaction に入れる。
原子性は単純になるが、現在の2層 Event Log、コールドアーカイブ、snapshot/restore、
replication の実装に与える影響が大きい。単に `FileEventStore` を SQLite に置き換える
だけでは済まないため、選択肢 A より大きな変更として扱う。

現時点の推奨は **選択肢 A** である。ただし、現行 ADR を変更する実装には新しい ADR と
人間の承認が必要であり、この文書だけでは挙動を変更しない。

### 4.2 Market と Sector settlement

Market DB 内の注文・Currency・escrow は一つの transaction にできるが、Market から
Sector inventory へ送る `RemoveItemCommand` / `ReturnItemCommand` / `CreditItemCommand` は
別の所有領域である。PostgreSQL へ移行しても、この跨ぎは自動的に原子的にならない。

Market の独立プロセス化までに、次を導入する。

- Market transaction と同時に settlement intent を保存する transactional outbox
- 各 settlement に一意な ID を付ける
- Sector 側で同じ settlement ID の再適用を無害にする
- 配送済み状態を確認し、未配送 intent を再送できるようにする

分散 transaction を最初から導入せず、少なくとも一回配送 + 冪等適用で回復可能にする。

## 5. PostgreSQL を将来候補とする理由

共有 Market の第一候補は PostgreSQL とする。

- 複数 client/process からの同時更新をサーバー側で調停できる
- 注文、残高、escrow、outbox を一つの transaction に含められる
- Serializable isolation と retry により、複雑な同時更新の不整合を防げる
- backup、監視、standby、同期/非同期 replication の運用経路がある
- SQL、制約、index を維持でき、現在の Market モデルからの移行距離が短い

PostgreSQL も Sector Event Log との分散 transaction を提供するわけではない。
採用理由は「すべてを原子的にするから」ではなく、**共有 Market の writer 調停と運用性を
一つの専用サーバーへ移せるから**である。

- [PostgreSQL: Transactions](https://www.postgresql.org/docs/current/tutorial-transactions.html)
- [PostgreSQL: Transaction Isolation](https://www.postgresql.org/docs/current/transaction-iso.html)
- [PostgreSQL: High Availability, Load Balancing, and Replication](https://www.postgresql.org/docs/current/high-availability.html)

## 6. その他の候補

| 候補 | 評価 | 現時点で採用しない理由 |
|---|---|---|
| RocksDB / redb / LMDB | Sector-local KV には利用可能 | Market の複合検索、制約、schema migration を自前化する。SQLiteより明確な利益がない |
| rqlite / dqlite | SQLite 系の複製には利用可能 | Dawn 自身の Raft に加えて別の合意系が増える。Market-Sector の二重書き込みも解決しない |
| CockroachDB / FoundationDB | 分散 transaction が必要なら再評価可能 | 現在の負荷に対して運用、障害解析、latency、設計コストが大きすぎる |
| DuckDB | 分析・オフライン集計には利用可能 | OLTP の注文帳や inventory authority の用途ではない |
| MySQL / MariaDB | 共有 Market を実装可能 | PostgreSQLよりDawn固有の明確な利点が現時点でない。運用標準が別途あれば再評価する |

## 7. Module seam の方針

Station inventory は呼び出し側から SQLite 実装が隠れており、storage seam の内側で
adapter を変更できる。これは維持する。

一方、現在の `MarketDb` の public interface は `rusqlite::Result` を返すため、SQLite の
エラー型が呼び出し側へ露出している。PostgreSQL adapter を実際に追加する段階では、先に
永続化失敗を表す Market 固有の error へ変換し、呼び出し側が DB 製品を知らない interface
にする。

ただし、将来の可能性だけを理由に今すぐ `MarketStore` trait を追加しない。二つ目の adapter
を実装する時点で本物の seam として抽出し、SQLite と PostgreSQL の両 adapter を同じ
interface-level test で検証する。

## 8. 現在の実行方針

当面は次を維持する。

- `FileEventStore` の2層 append-only log を継続する
- Station inventory は node-local SQLite + bounded memory cache を継続する
- Market は単一 `MarketRuntime` + SQLite を継続する
- SQLite ファイルをネットワーク共有しない
- PostgreSQL や分散 DB を先行導入しない
- DB 製品の移行より、Event/Projection と Market/Sector settlement の回復可能性を優先する

再評価時はベンチマークだけでなく、writer 数、所有権、障害復旧、RPO/RTO、運用負荷を
入力として新しい ADR を作成する。
