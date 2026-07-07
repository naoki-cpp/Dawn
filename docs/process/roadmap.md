---
scope    : 何を・どの順番で・なぜその順番で作るか。現在地と次のステップの明示
audience : AI Agent / Human Developer
update   : フェーズ完了時 / タスクが完了するたびに更新する
related  : ../architecture/architecture.md, AI_DEVELOPMENT_GUIDE.md, docs/process/roadmap-history.md（完了済みフェーズの詳細記録）
---

# Roadmap

## 1. このドキュメントの使い方

### 現在地の確認

「現在のフェーズ」セクションを見ること。  
次に着手すべきタスクは **1 つだけ** 太字で明記される。

### フェーズを飛ばしてはならない理由

各フェーズは次のフェーズの前提となる。  
例: Phase 1 の完了（単一ノードで 10,000 ships が動く）なしに
Phase 2（複数ノード）を実装すると、「動かない上に複雑」なコードになる。

### 完了基準の意味

完了基準は「感覚的に完成した」ではなく「このコマンドが成功する」で定義される。
曖昧な基準は採用しない。

---

## 2. 現在地

```
現在のフェーズ : Phase 8 ほぼ完了（8A/8C/8D/8E 完了・8B 一区切り）— スケール基盤+分散インフラが稼働
フェーズの状態 : 全 workspace グリーン。
                - 8A（durability）: 2 層ログ + snapshot 圧縮 + CheckpointScheduler（ADR-0017）
                - 8C（AoI）: 静的セルグリッド 3×3×3 + Enter/Leave 差分配信 + イベントフィルタ（ADR-0019）
                - 8B（一区切り）: 局所 TiDi コア（決定論的・非破壊・自動回復）+ 入場バックストップ
                  （生カウント・--pop-cap）。柱①の主要レバーが単一 Sector 内で出揃った（ADR-0018）。
                - 8D（分散インフラ）: dawn-replication + ネットワーク RaftTransport + dawn-sector-node
                  本番バイナリ + 永続化配線まで完了。**Raspberry Pi 3ノードクラスタでの実機検証を
                  2026-07-01 に実施し、reachability / tick-sla / failover の3項目とも PASS**
                  （docs/process/8d5-hardware-notes.md）。
                残り 8B（Fission / LoD=ADR-0020 deferred / 越境 TiDi / SLA イベント化）は
                いずれも現状は不要・§11 の負荷対応バックログへ切り出し済み。
                戦闘の深み（ADR-0016 §5）は Logistics（遠隔修理、ADR-0036）完了で一巡。
                次の前進先: Phase 9（Resource + Economy Context・§12）。
                Sector キャパシティの悪用対策は docs/design/game-design.md §8 を参照。
```

### 完了済みフェーズ

- ✅ Phase 0 — 基盤確立（`cargo test --workspace` 73テスト全パス）
- ✅ Phase 1 — Single Node シミュレーション検証（max 11,847 µs ≤ 16,000 µs 目標達成）
- ✅ Phase 2 — In-Memory Multi-Node（3ノード 63,000イベント整合性 ✓）
- ✅ Phase 3 — Event 永続化（Snapshot + Replay 再起動後の状態完全復元 ✓）
- ✅ Phase 4 — ゲーム開発ループ（Cycle 1〜3 完了 / 卒業基準 5/5 達成）
- ✅ Phase 5 — マルチプレイヤー基盤（ADR-0007 チェックリスト全完了 / 138テスト全パス）
- ✅ Phase 6 — ゲームループ改善（Capacitor / EVE命中率式 / タクティカルオーバーレイ / ボットAI / 154テスト全パス）
- ✅ Phase 7 — 分散コンセンサス（Raft / ADR-0014 / リーダー障害中の Transit 完遂を検証 / 223テスト全パス）
- ✅ Phase 7.5 — 星系間ナビゲーション（ADR-0009 / Jump Gate Raft パイプライン + Godot クライアント配線 / 241テスト全パス）
- ✅ 戦闘の深み — Warp（ADR-0022）/ Propulsion Physics（ADR-0023）/ Tackle（ADR-0024）実装済み（316テスト全パス）
- ✅ 天体（ADR-0025）— 恒星・惑星を静的天体として追加。WarpTarget::Body 対応、sun_direction シェーダー、Godot クライアント配線（W キー / クリック選択）まで完了。（316テスト全パス）
- ✅ Orbit / Keep at Range（ADR-0031・2026-06-23）— OrbitComp/KeepAtRangeComp + process_orbit
  /process_keep_at_range（Step 2.55/2.56）。Approach/Orbit/KeepAtRange 相互排他、Warp 中は拒否。
  O/K キー配線済み。これで戦闘の深み（Tackle → Signature → Orbit/Keep at Range）が完了し、
  Logistics の前段として Local Repair（ADR-0033）へ進んだ
- ✅ Local Repair（ADR-0033・2026-06-24）— Active Shield Booster / Armor Repairer、
  RepairSystem（Step 6.5）、RepairApplied、Godot 緑フラッシュまで実装済み
- ✅ Remote Repair / Logistics（ADR-0036・2026-07-03）— `ModuleKind::RemoteShieldBooster`/
  `RemoteArmorRepairer` を ADR-0035 の per-slot ターゲット・Range Gate System 基盤に乗せ、
  `repair_range_add`/`RepairCycle.target_ship_id` を追加。これで戦闘の深み
  （ADR-0016 §5）が一巡し、次は Phase 9（Resource + Economy Context）
- ✅ Godot クライアント構造リファクタ + テスト基盤（2026-06-21）— `main.gd` の god object を
  `HudManager`/`NavigationMarkerRenderer`/`ShipPicking`/`InputDecoder` の4クラスへ分割
  （1661→1094行）。`scripts/setup-godot.*` で pin 済み Godot CLI を取得し GdUnit4 を導入、
  計58ケースを実行確認（詳細: `docs/architecture/architecture-review-client.md`）。モジュール
  ON/OFF→CAP!誤表示のバグ修正も含む。Rust側は343テスト全パス
- ✅ Phase 8D — 分散インフラ（物理ノード）完了（2026-06-29）+ Raspberry Pi 実機検証（2026-07-01）—
  `dawn-replication`（ゴシップ配布 + アンチエントロピー + スナップショット転送）・ネットワーク
  `RaftTransport`・本番バイナリ `dawn-sector-node` を配線。**Pi 4/5 実機の3ノードクラスタで
  reachability / tick-sla / failover の3項目とも PASS**（`scripts/verify-pi-cluster.sh` で自動化・
  詳細は `docs/process/8d5-hardware-notes.md`）。永続化配線（FileEventStore + checkpoint +
  起動時リカバリ）も含めて配線済み

### Phase 4 卒業記録（ADR-0007 §6）

```
✅ 2クライアントが同時に接続できる
✅ 両クライアントの世界状態が同期している
✅ プレイヤーのロックオン操作が機能する
✅ 再接続後に InitialState で状態が復元される
✅ 基本的なゲームループでクラッシュしない
```

### Phase 5 完了記録（ADR-0007 実装チェックリスト）

```
✅ dawn-core: PlayerId(u64) 型追加
✅ dawn-core: DawnError::NotOwner 追加
✅ dawn-simulation/node.rs: player_ships HashMap / spawn_player_ship / 全コマンド所有権チェック
✅ dawn-simulation/ws_server.rs: Hello/Welcome/InitialState ハンドシェイク
✅ dawn-simulation/ws_server.rs: PlayerSession 構造体 / 複数クライアント同時接続
✅ dawn-simulation/ws_server.rs: AttackCommand JSON パーサー追加
✅ dawn-simulation/main.rs: ORIGIN シグナル処理を削除
✅ connection.gd: Hello 送信 / Welcome 受信 / InitialState 受信
✅ main.gd: ORIGIN シグナル送信削除 / Welcome シグナル処理
✅ 138テスト全パス
```

### 次に着手すべきタスク

**Phase 8（8A/8C/8D/8E 完了・8B 一区切り）は完了。** 残る 8B 保留項目（Fission / LoD=ADR-0020
deferred / 越境 TiDi / SLA イベント化）はいずれも現状は不要・**§11 負荷対応バックログ**へ
切り出し済み（トリガー待ちで通常のスプリント計画からは外す）。
8D（物理ノード分散）は Raspberry Pi 実機検証まで完了しており、次の前進先の選択肢からは外れた。

**戦闘システムの Logistics（遠隔修理）は完了（ADR-0036、2026-07-03）。**
戦闘システム（§9 参照）は Warp（✅）→ Tackle（✅）→ Signature Resolution（✅）→
Orbit/Keep at Range（✅）→ Local Repair（✅）→ **Remote Repair（✅）** と積み上がり、
Phase 8 発案時点の近期ロードマップ（ADR-0016 §5）が一巡した。

Logistics 本体は ADR-0035（Per-Slot Module Targeting、2026-07-02）の土台の上に、
新規 `ModuleKind::RemoteShieldBooster`/`RemoteArmorRepairer`（`requires_target()`
に追加）・`repair_range_add`/`ShipStatsComp.repair_range`（`tackle_range` と同じ
集計経路）・`RepairCycle.target_ship_id`（Capacitor System が
`slot.target_ship_id.unwrap_or(snap.ship_id)` で解決、Local/Remote 共通コード
パス）を積むだけで乗った。Range Gate System（Step 5.5）は2行追加で
Remote Repair にも対応。

**次に着手するのは Phase 9（Resource + Economy Context・§12）。** ただし戦闘システムは
Logistics で「完了」するわけではなく、§9 のとおりその後も継続的に深化していく対象であることに注意
（クライアント表現・Bot AI の Remote Repair 活用は ADR-0036 のスコープ外として残っている）。

#### Phase 6 完了タスク一覧

| 優先度 | タスク | 状況 | 理由 |
|---|---|---|---|
| ✅ 完了 | Capacitor 実装 | サイクルベース cap 管理まで完了（ADR-0011） | 「常時 ON で勝ち」問題の解消 |
| ✅ 完了 | セッションメトリクス出力 | --duel モード限定で実装済み（勝敗・経過Tick・cap枯渇回数をstdout出力） | 数値でバランスを判断できるようにする |
| ✅ 完了 | Godot: cap バー表示 | ProgressBar ウィジェット実装済み（青色バー + GJ表示） | cap 状態の視覚フィードバック |
| ✅ 完了 | EVE 命中率式（ADR-0012） | tracking/falloff/sig_radius 追加。hit_chance = 0.5^(追跡項²+射程項²) | ポジション管理が実質的な意味を持つ |
| ✅ 完了 | タクティカルオーバーレイ（ADR-0013） | Tab キーで射程リング（緑:最適/橙:フォールオフ）を表示 | 距離と射程の視覚的フィードバック |
| ✅ 完了 | StopCommand（S キー） | 逆推力で減速停止。ボット AI にも使用 | 精密なポジション制御を可能にする |
| ✅ 完了 | ボット AI 改善 | 射程内停止・ロックキュー・スポーン位置修正 | デュエルが成立するようにする |

---

## 3. 完了済みフェーズの詳細記録（Phase 0〜7・アーカイブ）

Phase 0〜7 は全て完了済み（要約は §2「完了済みフェーズ」参照）。各フェーズの完了基準・
タスク表・計測結果・Cycle 詳細（Phase 4）は **docs/process/roadmap-history.md** を参照する
（当時の判断根拠・計測値が必要なときだけ読む。常時の現在地確認は §2 で足りる）。

---

## 9. 継続的に開発するシステム（フェーズと独立に進捗する）

> 2026-07-02 追加。以下の §10〜§14 の「フェーズ」は**基盤構築の一度きりの通過点**であり、
> 完了基準を満たせば次のフェーズへ進む。一方で **「システム」は基盤ができた後も終わりなく
> 内容を追加し続ける対象**であり、特定フェーズの完了に縛られない。本節はこの2つの読み方を
> 区別するための場所であり、以下のシステムは「一区切りついた」状態はあっても「完了した」
> 状態にはならない。

### 戦闘システム（Combat System）

ADR-0016 §5（段階的拡張）で優先順位づけされ、Phase 6〜8 を跨いで継続的に深化してきた。
現状: Tackle（ADR-0024）→ Signature Resolution（ADR-0012）→ Orbit/Keep at Range（ADR-0031）→
Local Repair（ADR-0033）→ **Remote Repair / Logistics（ADR-0036）** まで実装済み。

Logistics 完了後も、戦闘システムとしての拡張は終わらない想定（新モジュール種・新ダメージ
タイプ・新戦術オプションなど）。個別の追加は都度 ADR を起票し、本節ではなく該当箇所
（game-design.md §4.1 実装済み一覧・event-catalog.md 等）に反映していく。

### 経済システム（Economy System）

Phase 9（§12・Resource + Economy Context）は**基盤構築フェーズ**であり、その完了基準
（Scrap Metal による建造コスト・Packaged Ship の Assemble/Disassemble・Market の指値
マッチングが機能する等、ADR-0034 参照）を満たせば「Phase 9 完了」とはなるが、**経済
システムとしての開発はそこで終わらない**。基盤ができた後も、新資源種・新構造物種
（9C）・新しい市場メカニクスなどを継続的に追加していく対象になる想定。

### 今後この節に加わりうる候補

グラフィック（Phase 11・§14）は現状「見た目の作り込み」というフェーズ的なタスクリストで
表現しているが、船種追加のたびに新モデルが要る等、実態としては継続的システムに近づく可能性
がある。着手後の実態を見て、フェーズ表記のままにするか本節へ移すかを判断する。

---

## 10. Phase 8 — スケール基盤 / 持続性（ADR-0017 / ADR-0018）— ほぼ完了

> 2026-07-02、ほぼ完了したため詳細タスク表を **docs/process/roadmap-history.md**（Phase 8 節）へ
> 移設し要約（Phase 0〜7 と同じ扱い）。保留中の項目のみ本節に残す。
> 対応 ADR: ADR-0017（スナップショット圧縮・2層ログ）, ADR-0018（局所 TiDi）,
> ADR-0019（AoI 静的セルグリッド）, ADR-0021/0027（複製）。

**完了基準**: 通常負荷で論理 Tick が SLA（≤32ms）を満たす／空間分離可能な負荷は劣化ゼロで捌ける／
分割不能な単一密戦闘は局所 TiDi で graceful に劣化し自動回復する／入場制限は最終バックストップのみ／
創世記 replay なしで failover・再起動できる（詳細: roadmap-history.md）。

| サブフェーズ | 状態 | 備考 |
|---|---|---|
| 8A イベントログ持続性 | ✅ 完了 | 2層ログ + snapshot 圧縮 + `CheckpointScheduler`（ADR-0017） |
| 8B 負荷制御 / Anti-TiDi | 🔶 一区切り（2026-06-15） | 局所 TiDi コア + 入場バックストップで柱①の主要レバーは完了。残りは下表参照 |
| 8C AoI 静的セルグリッド | ✅ ほぼ完了 | 3×3×3 隣接可視・Enter/Leave差分配信・イベントフィルタ（ADR-0019）。NPCオートロック連携のみ保留 |
| 8D 分散インフラ（物理ノード） | ✅ 完了 | postcard ワイヤ + `dawn-replication` + ネットワーク RaftTransport + `dawn-sector-node`。**Raspberry Pi 3ノードクラスタでの実機検証を 2026-07-01 に実施し、reachability / tick-sla / failover の3項目とも PASS**（[8d5-hardware-notes.md](./8d5-hardware-notes.md)）。永続化配線（FileEventStore/checkpoint/起動時リカバリ）も配線済み |
| 8E Transit consensus | ✅ 方針確定 | 単一 Raft グループ維持。バッチ提案は保留 |

保留中の負荷系項目（Fission / LoD / 越境TiDi 等）は **§11 負荷対応バックログ** に切り出した。
Phase 8 自体はこれらの着手を待たずに完了扱いとする。

### Phase 9 以降

```
Phase 9 : Resource + Economy Context（dawn-economy / FBD-008 撤廃により ADR で解禁）
Phase 10: Client 本格化（GDExtension 導入）
```

→ 詳細タスクは §12（Phase 9）/ §13（Phase 10）参照（2026-07-02 詳細化。旧版は方向性のみだった）。

### クライアント技術スタック・リポジトリ構成

2026-07-02、他ドキュメントとの重複のため本節から削除。参照先:

- クライアント技術選定（Godot 4 / GDScript / godot-rust の根拠）: `docs/adr/ADR-0004-client-technology.md`
- サーバー側 Cargo workspace のクレート構成・依存 DAG: `docs/architecture/architecture.md` §3
- クライアント側 `client/scripts/` の現在のファイル構成・責務・行数: `docs/architecture/architecture-review-client.md`

2026-07-02、「フェーズ横断の設計原則」節を削除（`AI_DEVELOPMENT_GUIDE.md`「Architecture Invariants」
と重複・そちらが正典）。

---

## 11. 負荷対応バックログ（低優先度・トリガー待ち）

> 2026-07-02、§10 から切り出し。Phase 8 の完了基準はすでに満たされており、以下は
> **いま着手する理由がない**、トリガーが来たときだけ拾う項目群。ロードマップの本流
> （§9 継続的システム → 次の作業）を読むときはこの節を無視してよい。定期的に「トリガーが
> 発火していないか」を確認する以外は、通常のスプリント計画の対象から外す。

| 項目 | 状態 | トリガー |
|---|---|---|
| 8B-2 Dynamic Sector Fission | ⏸️ 未着手・現状は不要 | 2026-07-02 に「8D着手時に設計」という前提を確認したところ誤りと判明: 8Dは物理ノードの**静的**分散配置であり、Fission（1 Sectorの**動的**分割）とは別の問題で、8D完了はFissionの着手を要求しなかった。本来のトリガーは tick-model.md §8「population_cap の80%到達」（現行 `POPULATION_CAP=100,000` なので80,000隻/Sector）。実測は aoi-bench で n=20,000・実プレイテストは数十隻規模で、閾値まで実測で3桁近い開きがある。要ADRだが、今すぐ着手する理由はない |
| 8B-3 Simulation LoD | ⏸️ deferred（ADR-0020） | go/no-go スパイク（idle反復がTick予算の有意割合か）次第 |
| 8B-6 構造化 SLA イベント | 🔶 部分実装 | 係数・継続Tick・engage/recoverログは実装済み。イベント化は後回し可 |
| 8B-8 差分 TiDi 越境 | ⬜ | 8B-2 Fission に依存（Fission 自体が不要な現状は着手不要） |
| 8C-5 NPCオートロック連携 | ⬜ | 全走査版と同一結果になるテストを用意してから着手 |
| 8D defer（Raftログ圧縮/InstallSnapshot・メンバーシップ変更・動的ノード発見・TLS/認証） | ⬜ ×4 | いずれも 2026-07-01 時点で未発火（詳細: roadmap-history.md） |
| 8E-2 バッチ提案（fleet jump） | ⬜ 保留 | fleet-jumpレイテンシが実測で問題化したら着手 |

---

## 12. Phase 9 — Resource + Economy Context

> 2026-07-02、`/grill-with-docs`（`/grilling` + `/domain-modeling`）で人間と対話しながら
> 具体化し、**ADR-0034（Economy Foundations）として起票済み**。以下のタスク表は
> ADR-0034 の決定を実装順に並べたもの。9C（プレイヤー設置インフラ）だけは ADR-0034 の
> スコープ外でまだ方向性のみ。
>
> 関連: ADR-0034（本節の決定はすべてここに記録）, ADR-0016 §4/§5（FBD-008 撤廃・段階的
> 拡張方針）, ADR-0032（`InventoryComp` の初出・ADR-0034 が一般化する）, CONTEXT.md
> （`Item`/`Packaged Ship`/`Station`/`Scrap Metal`/`Currency`）, ADR-0037（2026-07-05・
> Docked Ship Ownership — 9B の `Assemble` が player-level の owned ship / active ship /
> docked station context を要求すると判明したことを受けた決定。9B-5 参照）。

**前提**: 戦闘の深み（ADR-0016 §5 items 1–5: Tackle → Signature Resolution → Orbit/Keep at
Range → Local Repair → Logistics/Remote Repair・ADR-0036）は完了済み。Phase 9 はこの後に着手する。

**完了基準**:

- Scrap Metal が**建造コストとして機能**する（Packaged Ship の新規建造でのみ消費）。**受動採取
  （AFK 相当）は存在しない** — `ShipDestroyed` からの即時ドロップのみが取得経路（FBD-009 維持）。
- Packaged Ship ⇄ Ship の Assemble/Disassemble が Station（NPC提供）で機能し、無料修理の
  抜け穴（損傷した船のDisassemble）を防ぐバリデーションが効いている。
  Station は**Packaged Ship / Scrap Metal を置く最小インベントリ**も持つ。
- Market（`dawn-market` クレート）が SQL を独自の権威として持ち、指値（bid/ask）マッチングで
  価格が決まる。Currency は Item ではなく `PlayerId` 単位の台帳（船を失っても消えない）。
- プレイヤー設置インフラ（Smart Assembly 相当）のアクセス制御は、Tick パイプライン内で
  決定論的に評価される **述語関数 `can_use(actor, structure) -> bool`** として表現される
  （ブロックチェーン/wallet 不要・INV-005 決定論と整合。eve-reference §7.3。9Cはまだ未確定）。

### 9A. Item 一般化 + Scrap Metal（ADR-0034 §1/§3、資源シンクの基礎）

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | `dawn-core`: `ItemId` enum（`Module`/`PackagedShip`/`ScrapMetal`） | Currency は含まない（ADR-0034 §5） | ✅ |
| 2 | `dawn-ecs`: `InventoryComp.items` を `Vec<ModuleId>` → `BTreeMap<ItemId, u64>` へ一般化 | ADR-0032 のデータモデルを置き換え。既存の `fit_module_owned`/`unfit_module_owned` の呼び出し側修正を伴う | ✅ |
| 3 | `dawn-sector`: `ShipDestroyed` 発生時に Scrap Metal を撃破者へ即時加算 | 新規 Wreck エンティティは作らない（ADR-0034 却下代替案）。**現状は MVP として 1 kill = 1 Scrap Metal の固定値** | ✅ |
| 4 | スナップショット永続化（`ShipSnapshot.inventory` の型変更に追従） | ADR-0032 の `#[serde(default)]` 後方互換パターンを踏襲 | ✅ |
| 5 | 「受動採取ではない」ことのチェック項目化 | **現状は不要**。取得経路が `ShipDestroyed` 以外にも増えたときに、AFK/受動導線が混入していないかを再点検する | ⏸️ |

### 9B. Station / Packaged Ship / Assemble・Disassemble（ADR-0034 §2）

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | Station（NPC提供の最小実装） | `StationId` / `StationDef` と galaxy TOML の `npc_stations` を追加。各 sector に最小 NPC station を1つ配置 | ✅ |
| 2 | Dock/Undock + Station 利用可否判定 | `DockCommand` / `UndockCommand` と docked 状態を追加。`can_use_station(player_id, station_id)` は「半径内」ではなく「その station に docked 済み」を見る。player-level の docked context を保持し、station access が active ship lookup に依存しないようにする | ✅ |
| 3 | Station インベントリ（最小保管先） | `PlayerId -> BTreeMap<ItemId, u64>` の最小 station inventory を追加。snapshot restore 対応済み。**これは MVP の in-memory 実装であり、将来は hot-memory + durable storage の二層へ進める** | ✅ |
| 4 | Station系イベントの土台 | `ShipDocked` / `ShipUndocked` / `PackagedShipBuilt` / `ShipDisassembled` / `ShipAssembled` すべて実装済み | ✅ |
| 5 | Assemble コマンド + バリデーション（Packaged Ship が未艤装であること） | 2026-07-07 実装完了。入力は Station インベントリ上の `PackagedShip`。**docked 中のみ**実行可。艤装情報は Packaged Ship 側に持たせず、Assemble 後に既存の Fit 経路で艤装する。`AssembleCommand`/`ShipAssembled`、`SimulationNode::assemble_ship_owned`（`Result<ShipId, StationOperationRejection>`）を新設。`active_ship` は自動変更しない（ADR-0037）ため、唯一の船をDisassembleして詰んだプレイヤーは Assemble → `SelectActiveShipCommand` → Undock で復帰可能に（詳細・修正した副次バグは `docs/architecture/ownership.md` §7-8 参照）。`cargo test --workspace` 全件通過・GdUnit4 未変更（クライアントUIはタスク8の方針通り未着手） | ✅ |
| 6 | Disassemble コマンド + バリデーション（Ship が無傷・未艤装であること） | 出力は Station インベントリ上の `PackagedShip`。**docked 中のみ**実行可。無傷チェックは無料修理の抜け穴防止（Local Repair・ADR-0033 の価値を守る） | ✅ |
| 7 | Packaged Ship 建造コマンド（Scrap Metal 消費 → Packaged Ship 生成） | Scrap Metal を Station インベントリから消費し、生成物も Station インベントリへ置く。**docked 中のみ**実行可能。現状コストは MVP として `1 Scrap Metal / 1 hull` の固定値 | ✅ |
| 8 | client実装の開始条件を固定 | **基本方針: client UI は 5〜7 の server 側本体（Assemble / Disassemble / Build）が揃ってから実装する。** 先に UI だけ作って wire 先行にならないようにする | ⬜ |
| 9 | client: Dock/Undock + Station操作UI | 入港状態の表示と `D` / `U` / `B` / `Y` 操作は実装済み。client は `ShipDocked` / `ShipUndocked` と `PlayerLoadout` の両方から dock state を受けるため、順序逆転で古い loadout が HUD を巻き戻さないこと、undock 後の `null` dock context を `station_id=0` と誤解しないことを維持条件とする。ship inventory と station inventory の見分けがつくことは引き続き必要 | ◐ |
| 10 | client: Packaged Ship のインベントリ表示・Assemble/Disassemble/建造UI | station UI の上に載せる。Ship側 inventory と Station側 inventory が混ざらないこと | ⬜ |

#### 9B 補足: Station inventory の保存戦略

- Market と違って、Station inventory は Sector command validation のホットパスにある。
- そのため、即時の権威状態を毎回 SQL 直読みにする設計は採らない。
- 方向性は **実行中はメモリ、耐久保存と容量対策は DB/スナップショット** の二層。
- 将来の大量入港対策は、dock 中 / 最近使った player の inventory を lazy load /
  write-back cache として扱える seam を切ることで進める。
- 現状は raw `BTreeMap` 直参照を helper 経由へ寄せ始めた段階で、backend 差し替えの
  足場だけ先に整えている。

### 9C. プレイヤー設置インフラ（Smart Assembly 相当・ADR-0034 の範囲外）

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | 新規 ADR 起票（構造物エンティティ・所有権モデル） | 「キャラクター」同様、エンティティとしては解禁済み（ADR-0016 §4）。育成要素は持たせない | ⬜ 要 ADR |
| 2 | アクセス制御を `can_use(actor, structure) -> bool` 述語として設計 | Tick パイプライン内で決定論的に評価。Smart Turret の「自トライブ以外を自動攻撃」は dawn の Bot System の設置型版として設計できる（eve-reference §7.3） | ⬜ 要 ADR |
| 3 | 構造物の Sector 所有権・Transit との関係（構造物は Transit しない前提の確認） | ownership.md の状態遷移図に追記が必要 | ⬜ |
| 4 | Station（9B）をプレイヤー建造可能にする | NPC提供の最小Stationから拡張。9C の構造物モデルが前提 | ⬜ |

### 9D. Market / Currency（ADR-0034 §4/§5/§6）

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | 新規クレート `dawn-market` の Dependency DAG 上の位置を確定 | SQL権威・Tick非依存の別ドメインとして切り離す（ADR-0034 §4） | ⬜ |
| 2 | SQLite バックエンドの指値注文帳（bid/ask マッチング） | アルゴリズム価格ではなくプレイヤーの指値で決まる（ADR-0034 §6） | ⬜ |
| 3 | `PlayerId` 単位の Currency 台帳 | Itemではない。ShipDestroyedで失われない（ADR-0034 §5） | ⬜ |
| 4 | `RemoveItemCommand`/`ReturnItemCommand`/`CreditItemCommand`（List/Cancel/Settle） | 常に片側1Sectorだけへ発行。Transit/Raft合意は不要（ADR-0034 §4） | ⬜ |
| 5 | client: Market閲覧UI（指値注文の発注・Currency残高表示） | | ⬜ |

### 9E. 経済ループの検証・バランス

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | 資源希少性が実際に判断/対立を生んでいるかのプレイテスト（playtest-guide.md 拡張） | 「数値を積むだけ」になっていないかを人間の観察で判定 | ⬜ |
| 2 | 受動蓄積ゼロの回帰チェック | 9A-5 が「自動テストで守るべき性質」に育った段階で CI へ昇格する。現状はチェック項目として運用 | ⏸️ |

---

## 13. Phase 10 — Client 本格化（GDExtension 導入）

> ADR-0004（クライアント技術選定）で既定路線として決定済み。Phase 9 より独立して着手可能
> （経済ループの有無に依らずクライアント性能の話のため）だが、優先度は Phase 9 より低い
> （§10「戦闘の深み」「分散インフラ」が柱①②④に直結するのに対し、本フェーズは体験の
> 滑らかさの改善であり新しいプレイヤー決定を増やさない）。

**完了基準**: レイテンシを隠した滑らかな操作感（Client-Side Prediction 導入後、体感の
入力遅延が軽減されたことをプレイテストで確認する）。

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | `client/gdextension/` に godot-rust プロジェクトを新設 | 技術選定の根拠は ADR-0004 参照 | ⬜ |
| 2 | `dawn-core` 型を GDExtension 経由で Godot へ直接公開（型共有の切り替え） | 現行の「チャンネル/JSON 変換」から「直接 import」への移行。ADR-0007（通信方式）の型共有部分を更新する新規 ADR が必要 | ⬜ 要 ADR |
| 3 | Client-Side Prediction を Rust 側に実装（サーバー権威の再現ロジックをクライアントでも動かす） | `dawn-ecs` の Movement/Warp システムをクライアント側でも再利用できるかが鍵。サーバーとの分岐（reconciliation）設計が必要 | ⬜ 要 ADR |
| 4 | WebSocket + JSON からの通信方式移行を再検討（gRPC 等） | ADR-0007 で「Phase 9 以降で再検討」と明記済み。GDExtension 導入で型共有が変わるため、このタイミングでの見直しが自然 | ⬜ 要 ADR |

---

## 14. Phase 11 — グラフィックの深化

> Phase 9（経済）/ Phase 10（GDExtension・Client-Side Prediction）とは独立した方向性として
> 2026-07-02 に追加。**サーバー権威・イベントスキーマ・ゲームルールには触れない、純粋に
> クライアント側の描画品質の話**であり、Phase 9/10 のように個別 ADR は必須ではない
> （挙動変更を伴わない見た目の変更は AI_DEVELOPMENT_GUIDE.md の変更ワークフロー通りの
> 小さな差分で進められる。ただしアセットパイプラインや外部ツール依存の追加など、
> 判断の重い項目は軽量 ADR を起票する）。
>
> 根拠: ADR-0004「EVE Online レベルの宇宙グラフィックス（宇宙船・ネビュラ・エフェクト）」を
> Godot 4 選定の理由に掲げているが、現状の船は `client/scenes/ship.tscn` の
> `CylinderMesh`（Hull）+ `SphereMesh`（EngineGlow）というプレースホルダ形状のままで、
> この理由に見合う品質にはまだ到達していない。game-design.md の非スコープ「×
> グラフィックスエンジン外部依存」は**エンジンの差し替えを禁じる**もので、Godot 内での
> 描画品質向上（本フェーズ）とは矛盾しない。
>
> 2026-07-01、EVE Online を動かす Carbon エンジンが MIT ライセンスで全面オープンソース化
> （`github.com/carbonengine`）。グラフィックス側モジュール Trinity のシェーダー/エフェクト
> 実装を参考に Godot 側を書き直す（コードの直接移植ではなく、表現手法の翻訳）。対象3件を
> 優先度付きで下記タスク#3・#5・#1に注記する。

**完了基準**: 主要な船種（`data/ship_types.toml`）が固有の3Dモデルで表示され、戦闘の主要
アクション（発砲・被弾・爆発・ワープ・モジュール発動）に対応する視覚エフェクトが付く。
既存の空間シェーダー（`space_sky.gdshader`）基準の見た目を壊さない。

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | 船種ごとの3Dモデル（glTF）を用意し `ship.tscn` のプレースホルダ形状を置き換え | `client/assets/models/` は Phase 4 のリポジトリ構成表に予約済みだが未使用。アセット調達方針（自作/外部アセット購入/AI生成）は軽量 ADR で決める。**Trinity参考・優先度3**: 船体マテリアル（金属質感・発光パターン）の表現手法を参照 | ⬜ |
| 2 | 武器発射・被弾・爆発のパーティクルエフェクト | `WeaponFired`/`DamageTaken`/`ShipDestroyed` イベント受信時にトリガー。イベントスキーマ変更は不要（クライアント側の描画のみ） | ⬜ |
| 3 | ワープ突入/離脱エフェクトの拡充 | 既存の `warp_tunnel_effect.gd`（フルスクリーン ColorRect シェーダー）を土台に、開始/終了の演出を追加。**Trinity参考・優先度1**: ワープトンネル表現の参照実装として最初に着手 | ⬜ |
| 4 | モジュール発動時の視覚フィードバック拡充 | 現状は Local Repair の緑フラッシュ（`flash_repair`）のみ。他モジュール種（武器・推進）にも同様のフィードバックを広げる | ⬜ |
| 5 | 天体（恒星・惑星）の見た目の作り込み | ADR-0025 で静的天体は導入済みだが、恒星は実体メッシュを撤廃し空シェーダーの方向ベース描画のみ。惑星の質感・大気表現などは今後の余地。**Trinity参考・優先度2**: ネビュラ背景の表現手法を参照 | ⬜ |
| 6 | ライティング/ポストプロセッシング（Bloom・トーンマッピング等）の調整 | Godot 4 の WorldEnvironment 設定の調整のみで着手可能 | ⬜ |
| 7 | パフォーマンス回帰の確認（多数の視覚エフェクトが密戦闘のフレームレートを壊さないか） | Phase 8B の局所 TiDi・AoI が支える大規模戦闘の体験を、クライアント側の描画負荷で壊さないことを確認 | ⬜ |

---

## 15. 廃止・変更された計画の記録

2026-06-14（Phase 8 前提の3設計判断・ADR-0016/0017/0018）と 2026-06-04
（Phase 4〜11 開発戦略の2段階変更）の詳細は **docs/process/roadmap-history.md** の
「廃止・変更された計画の記録」を参照。
