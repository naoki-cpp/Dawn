# よくある設計違反パターン

> AI_DEVELOPMENT_GUIDE.md §12 の正典。ガイド本体には参照リンクのみを残す（ADR-0030）。

AIが陥りやすいアンチパターンとその修正方法を示す。

## パターン1: 「便利だから」とState同期を使う

```
状況: ノード間でPosition差分が発生した時、Stateを直接上書きで同期しようとする

違反コード:
  // "Eventより直接同期の方が速い" という誤った判断
  node_b.update_position(ship_id, node_a.get_position(ship_id))

正しい判断:
  EventをGossipで伝播させる。StateはEventから自動的に収束する。
  State直接同期は INV-001 と INV-002 を同時に破る。
```

## パターン2: テストをスキップして「後で書く」

```
状況: 実装が複雑でテストを後回しにしようとする

なぜ危険か:
  AIは次のセッションでコンテキストを持ち越さない。
  「後で書く」は「永遠に書かない」と等しい。
  テストなしのコードは次のAIセッションで意図せず破壊される。

対処:
  実装が複雑ならテストを先に書き、テストを通す最小実装を先に行う。
  テストが仕様書になる。
```

## パターン3: 新機能のためにdawn-coreを肥大化させる

```
状況: 新しい機能を追加するとき、dawn-coreに実装ロジックを追加しようとする

違反コード（dawn-core/src/position.rs）:
  impl Position {
      pub async fn broadcast_to_nodes(&self, nodes: &[NodeAddr]) { // ← ネットワーク処理
          ...
      }
  }

正しい判断:
  dawn-core はデータ定義のみ。
  ネットワーク処理は dawn-replication または dawn-sector-node に配置する。
```

## パターン4: Tickを物理時刻に「合わせる」最適化

```
状況: "Tickと実時間を合わせると分かりやすい" という理由で物理時刻を使おうとする

危険性:
  物理時刻に依存した瞬間、3ノード間で Tick の順序が非決定論的になる。
  テスト環境と本番環境でTick順序が変わる可能性がある。
  NTPのステップ補正で時刻が逆行した瞬間、システムが破綻する。

対処:
  Tick は論理カウンタのまま維持する。
  "人間が読みやすい時刻" は Observation Layer（ログ・メトリクス）でのみ使う。
  INV-005 を参照すること。
```

## パターン5: Sector Transitを「最適化」してRaftをスキップする

```
状況: "レイテンシ削減のため" Sector Transit を Raft なしで実装しようとする

違反の結果:
  2つのノードが同一Shipの所有権を同時に主張する状態（スプリットブレイン）
  → 両方のSectorが独立したShipMoveを処理し始める
  → 世界が分岐する（Single Shardの破壊）

対処:
  Sector Transit は必ず Raft を経由する。INV-003 を参照すること。
  レイテンシが問題なら Transit の頻度を下げる設計を検討する。
  ※ Raft は Phase 7（ADR-0014）で実装済み。Transit は Raft Log 経由で動作する。
```

## パターン6: FittingSnapshot をイベントに含めず ID だけ記録する

```
状況: "モジュールIDだけ保存してレジストリで引けば十分" という判断で
      ShipFitted イベントに ModuleId のリストだけを含めようとする

違反の結果:
  レジストリの内容が変わった場合（モジュールの stat が更新されるなど）、
  過去の Event を Replay すると当時と異なる stat が再現される。
  → INV-002 違反（Event Replay で世界が完全に再現されない）

正しい実装:
  ShipFitted イベントには FittingSnapshot（モジュール定義全体）を含める。
  Replay はレジストリに依存せず、イベントの内容だけで完結しなければならない。
  → ADR-0006 §1 参照
```

## パターン8: 状態変化をイベントとして表現する

```
状況: モジュールのオン/オフを表すイベントに is_active フラグを持たせようとする

違反コード:
  ModuleToggled { ship_id, module_id, is_active: bool, tick }
  // → is_active を見ないと何が起きたかわからない
  // → 状態の記述であって「事実」ではない

正しい実装:
  ModuleActivated   { ship_id, module_id, slot, tick }  // オンにした
  ModuleDeactivated { ship_id, module_id, slot, tick }  // オフにした
  // → イベント名自体が「何が起きたか」を表す

原則:
  Event は既に起きた事実（INV-006）。
  「状態がこうなった」ではなく「この動作が起きた」と命名する。
  過去形・動詞（Activated, Fired, Destroyed）を使う。
  is_*/has_* フラグをイベントのキーフィールドにしない。
```
