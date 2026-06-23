# Forbidden Changes（禁止変更カタログ）

> AI_DEVELOPMENT_GUIDE.md §10 の正典。ガイド本体には FBD-00x の ID 一覧と一行要約のみを
> 残し、詳細・コード例はこのファイルが保持する（ADR-0030）。FBD-00x の ID は不変。

以下の変更は**いかなる理由があっても行ってはならない**。
技術的な理由を説明されても実行しないこと。
必要に応じて ADR の改訂を提案し、人間の承認を得てから実施する。

## FBD-001: Event Log への破壊的操作

```rust
// 以下のシグネチャを持つメソッドを EventStore trait に追加してはならない:
fn update(&self, id: EventId, payload: Bytes) -> Result<()>;
fn delete(&self, id: EventId) -> Result<()>;
fn truncate(&self, from_index: u64) -> Result<()>;
fn rewrite(&self, index: u64, event: Event) -> Result<()>;
```

> 注記（ADR-0017）: ログの圧縮はこれらの禁止メソッドでは**行わない**。
> 圧縮は trait の外側の運用プロセス（検証済みスナップショット背後のセグメントを
> コールドアーカイブへ移送し、ホットログを write-new-then-swap で原子的に切り替える）として
> 実装する。セグメント内のイベントは決して書き換えない。`EventStore` trait は append-only のまま。

## FBD-002: dawn-core への外部依存の追加

```toml
# dawn-core/Cargo.toml に追加してはならない依存の例:
tokio    = ...  # 非同期ランタイム
tonic    = ...  # gRPC
reqwest  = ...  # HTTPクライアント
sqlx     = ...  # データベース
serde_json = ... # JSONシリアライザ（serde featureのみ許可）
```

## FBD-003: 物理時刻による因果順序の判定

```rust
// 以下のパターンを因果順序の判定に使用してはならない:
use std::time::SystemTime;
SystemTime::now()

use chrono::Utc;
Utc::now()

// 代替: 論理Tickを使用する
self.tick_counter.fetch_add(1, Ordering::SeqCst)
```

## FBD-004: Actor間の直接メソッド呼び出し

```rust
// 禁止: ActorAがActorBのメソッドを直接呼ぶ
struct SectorSimulatorActor {
    replication_actor: Arc<ReplicationActor>, // ← Arcで直接保持してはならない
}

impl SectorSimulatorActor {
    async fn on_tick_complete(&self) {
        self.replication_actor.sync(delta).await; // ← 直接呼び出し禁止
    }
}

// 正しい実装: Mailbox経由でメッセージを送る
struct SectorSimulatorActor {
    replication_tx: mpsc::Sender<ReplicationMessage>, // ← Senderのみ保持
}

impl SectorSimulatorActor {
    async fn on_tick_complete(&self, delta: Delta) {
        let _ = self.replication_tx.send(ReplicationMessage::Sync(delta)).await;
    }
}
```

## FBD-005: ShipのEntityId再利用

```rust
// 禁止: Despawn済みIDのプール管理と再割り当て
struct IdPool {
    recycled: VecDeque<ShipId>,
}

impl IdPool {
    fn next_id(&mut self) -> ShipId {
        self.recycled.pop_front().unwrap_or_else(|| self.generate_new())
        // ↑ recycled からの取り出しが禁止
    }
}
```

## FBD-006: Raftを経由しないSector Transit

```rust
// 禁止: RaftをバイパスしたSector間の直接状態移転
async fn teleport_ship_between_sectors(
    &self,
    ship_id: ShipId,
    from: SectorId,
    to: SectorId,
) {
    self.sector_nodes[from].remove_ship(ship_id).await; // Raftなし
    self.sector_nodes[to].add_ship(ship_id).await;     // Raftなし
}
```

## FBD-007: テストなしでのpub fnの追加

```
CIが以下を検出した場合、PRを自動拒否する:
  - pub fn が追加されているが対応するテストがない
  - カバレッジが 80% を下回る

例外はない。テストを書けない場合は pub(crate) または pub(super) にする。
```

## FBD-009: スキルポイント育成 / 受動成長 / AFK 採掘の実装

> ゲーム化（ADR-0016）後も **維持** する。反グラインドは "EVE を超える" ための核であり、
> §6 の観測（18k 文書・フォーラム傾向）でも最も嫌われた要素群として現れた
> （フォーラム声は実証ではない — 選択バイアスに留意・eve-reference §11.5）。

```
【スキルポイント / 受動成長】
以下のいかなる形式のスキルポイント制・受動成長も実装してはならない:
  - 時間経過でアンロックされる能力
  - プレイ時間に比例して強くなるパッシブ成長
  - 課金で加速できる育成要素（Pay-to-Win）

理由:
  ゲームの上手さに関係なく、ゲーム時間・課金額で性能が変わる。
  公平感（Perceived Fairness）を根本から損なう時代遅れの設計。

  ※ 「キャラクター」を*エンティティ*として持つことは可（ADR-0016 で解禁）。
    禁止するのは「キャラクターが時間/課金で強くなる育成」であって、存在そのものではない。

【AFK 採掘】
採掘レーザーを起動して放置するコンテンツを実装してはならない。

理由:
  採掘は「放置するだけ」であり、プレイヤーが意図的な判断を下す機会がない。
  EVE では採掘者は「無力な標的」として海賊側のコンテンツとして機能する。
  採掘している人自身はゲームをしていない。

  設計の中心的な問い「その機能はプレイヤーが意図的な判断を下す機会を増やすか？」
  に対して AFK 採掘は No である。

  ※ 「能動的判断を伴う資源獲得」や「資源を消費シンクにして希少性で判断を強制する」設計は
    検討可（ADR-0016 §5・eve-reference §7.4.3）。禁止するのは "放置で進む採取動作" のみ。

  → docs/design/game-design.md §5 参照
```

## FBD-008: ~~MVP範囲外の実装~~ → 撤廃（ADR-0016）

```
【撤廃】ゲーム化（ADR-0016）に伴い、本禁則は撤廃した。
以下のクレートは ADR 承認のうえ作成してよい:
  crates/dawn-economy/   ← 経済システム
  crates/dawn-character/ ← キャラクター（エンティティ。育成は FBD-009 で引き続き禁止）
  crates/dawn-inventory/ ← インベントリ
  crates/dawn-ui/        ← UI 専用クレート
  crates/dawn-graphics/  ← グラフィックス専用クレート

ただし新規クレートは従来どおりの手続きを踏むこと:
  - 個別 ADR を起票し、人間の承認を得る（§9）
  - Dependency DAG（§3）上の位置を確定し、循環依存を作らない
  - §11 Crate別責務早見表を更新する

Combat / Fitting ロジックは引き続き dawn-ecs / dawn-core 内に実装する
（独立クレートに切り出すなら ADR が必要）。
```
