---
id      : ADR-0019
title   : AoI のための静的セルグリッド（3×3×3 隣接可視）
status  : accepted
date    : 2026-06-15
deciders: [human, ai-agent]
related : ADR-0018（負荷ヒエラルキー / 局所 TiDi）, ADR-0016（柱① 大規模リアルタイム戦闘）, AI_DEVELOPMENT_GUIDE.md「Architecture Invariants」（INV-002 / INV-MOVE / INV-003）, docs/architecture/architecture.md §5-B（Interest Management）, docs/reference/eve-reference.md
---

# ADR-0019 — AoI のための静的セルグリッド（3×3×3 隣接可視）

> **ステータス注記**: 本 ADR は **accepted**（人間承認済み・2026-06-15）。
> 実装は `dawn-sector::aoi` と `dawn-sector::aoi_frame::AoiFrame` に置き、
> single-process、clustered simulation、production Sector Node の全経路が同じ配信実装を使う。
> 新クレートを作らず Dependency DAG は変更しない。

## 背景

ADR-0018 は過負荷対応をヒエラルキー化したが、「単一 Sector が抱えられるエンティティ数の上限を
どう引き上げるか」は未決だった。柱①（TiDi 閾値が EVE より桁違いに高い大規模戦闘 / ADR-0016）には
**単一 Sector の容量**を上げる必要がある。

### 真のスケール障壁は AoI（配信側）だけである

検討の結果、サーバ計算側に O(n²) の近傍探索負荷は**実在しない**ことが分かった。戦闘機能の大半は
「**既知のターゲット**」に対して動くため、そもそも近傍探索を発生させない:

| 機能 | 近傍探索か | コスト |
|---|---|---|
| 武器発射 | No。自分の `locked_targets`（既知リスト）を撃つ | O(自分のロック数) |
| Orbit / Keep at Range | No。特定の 1 ターゲットとの距離を保つ | O(1) |
| signature resolution | No。命中式内で既ロック対象の sig を使うスカラー計算 | O(1) |
| NPC オートロック | Yes（唯一の常時近傍探索） | #NPC 有限 → **O(n)** |
| 将来の AoE / スマートボム | Yes だが発生源が少数（∝ n でない） | #emitter 有限 → **O(n)** |

真に **O(n²)** になるのは **AoI（配信側）**である。各プレイヤーに自分の周囲だけを配信するには、
接続プレイヤー全員が毎 Tick「周囲に誰がいるか」を問う必要がある。全 Ship を毎回走査すれば
プレイヤー密戦闘では O(p·n) ≈ O(n²) になる。帯域も全世界配信ではグローバル n に比例する。

### EVE の知見と、その一歩先

EVE は「グリッド = 興味範囲そのもの」とし、静的・大きなセルでバケツ化する。本 ADR はこの発想を採り、
純・静的セル（自セルのみ可視）ではなく、**自セル + 各軸隣接 26 セル（3×3×3 = 27 セル）**を
可視範囲にする。不連続を観測者の位置ではなく 27 セル外殻へ押し出す。

---

## 決定

### AoI を「固定セル境界 + 3×3×3 隣接可視」で実装する（`dawn-sector`）

各プレイヤーの興味範囲を、自船が属するセルと各軸 ±1 の隣接セル、合計 27 セルとする。
セル座標は固定された空間分割であり、所属判定は絶対座標の床除算で決まる。

```
セル境界     : 空間を固定境界で分割する。境界自体は Tick 間で変化しない。
index lifecycle:
               配信 frame ごとに、権威ある現在位置から CellGrid の bucket を全再構築する。
               incremental update は行わず、index は永続化しない。
所属判定     : 船位置 → セル座標を床除算 O(1) で求め、セルごとに ShipId を保持する。
可視範囲     : プレイヤー自船セルを中心とする 3×3×3（27 セル）の在籍船。
InitialState : 全 Ship ではなく、その 27 セルの船のみを送る。
observer失敗 : 自船を解決できない admission / resume / handoff は明示的に失敗させ、
               空 payload や全 Ship payload へフォールバックしない。
購読更新     : 前 frame の可視集合と現在の可視集合を比較し、Enter/Leave を ShipId 順に送る。
配信順序     : Enter → Leave → filtered DomainEvent → MotionCorrection → PositionSnap。
配信フィルタ : 関与 Ship が観測者の現在可視集合に含まれる DomainEvent のみを送る。
```

### index は incremental ではなく frame ごとに再構築する

`AoiFrame::rebuild` は各 runtime の共通 Runtime Tick 出力を受けた後、`SimulationNode` の権威ある絶対位置から
`CellGrid` を再構築する。この方針を採る理由は次の通りである。

- CellGrid は配信専用の派生状態であり、snapshot や DurableJournal に含めない。
- 移動、spawn、destroy、warp、Sector handoff の更新漏れを runtime ごとに管理する必要がない。
- recovery 後も通常時と同じ `rebuild` を通るため、配信再開前に必ず権威状態と一致する。
- bucket と近傍列挙を ShipId 順に整列するため、挿入履歴に依存せず決定的である。

全再構築は「セル境界が静的」であることと矛盾しない。静的なのは空間の区切り方であり、
その時点の在籍 Ship を表す bucket は frame ごとに作り直す。

### 一つの deep AoI frame module が配信policyを所有する

`dawn-sector::aoi_frame::AoiFrame` が以下を一括所有する。

1. CellGrid の再構築
2. observer ship の解決と 27 セル可視集合の計算
3. player ごとの前回可視集合
4. Enter/Leave 差分
5. Event filtering
6. owner MotionCorrection
7. warp-arrival PositionSnap
8. 上記メッセージの決定的な順序

single-process runtime、clustered runtime、production Sector Node はこの同じ実装を使う。
runtime adapter に残すのは Sector routing、Redirect、session retention、および crate dependency 方向を守る
`AoiSink` adapter のみである。

### 設計上の制約

- **配信レイヤーの関心事であり権威ある状態に触れない**。AoI はdurable commitと
  live applyが成功したframe outputをobserverごとに絞る。`PublicEventTail`の保持や
  catch-upは担当しない。
- **セルbucketは派生・非永続**。復旧後は権威ある位置から再構築してから session をseedする。
- **Sector 内に閉じる**。Sector 越えは引き続き Raft と Redirect/resume が担当する。
- **27セル規則を全runtimeで共有する**。runtime独自のvisible-set policyを禁止する。
- **observer identityを成功payloadで隠さない**。自船を解決できなければ接続・resume・
  post-transit handoffを拒否し、全world InitialStateを送らない。
- **戦闘の射程判定は厳密距離のまま**。AoI候補集合は権威判定を置き換えない。

### 計測で閾値の上昇を実証する

「O(n²) を消した」ではなく「単一 Sector の容量が上がった」を示す。`--aoi-bench` で
プレイヤー数を増やしながら AoI 有無の処理時間・配信量を比較する。

---

## 単一密戦闘では空間索引は効かない

AoI が効くのは空間的に散らばった負荷である。全員が同一 27 セル近傍に集まれば全員が相互可視となり、
配信量は O(n²) に戻る。一点集中は ADR-0018 の局所 TiDi に落とす。
**AoI は TiDi 閾値を押し上げるが、TiDi を不要にはしない。**

---

## 不変条件との関係

| 不変条件 | 関係 |
|---|---|
| INV-002 | CellGrid は派生・非永続。snapshot round-trip / catch-up に含めない |
| INV-MOVE | セル所属は権威ある現在位置から再計算し、位置をイベント化しない方針を維持 |
| INV-005 | index構築とfilterは論理 Tick 出力後の決定的計算。物理時刻不使用 |
| INV-003 | AoI は Sector 内の配信filter。Sector 越えは Raft + Redirect/resume |
| ADR-0018 | 容量向上で TiDi 閾値を上げる。一点集中は依然 TiDi に落ちる |

---

## 代替案

- **incremental CellGrid**: 通常時の更新量は小さくなるが、spawn/destroy/warp/recovery/handoffの全経路で
  bucket更新を正しく同期する必要がある。runtime間のpolicy重複とstale entryの危険を増やすため不採用。
- **純・静的セル（自セルのみ可視）**: 最小実装だが不連続が自分の位置で出るため不採用。
- **真円 R の Bubble を毎 Tick 全走査**: 連続だが O(p·n) を自ら作るため不採用。
- **exact 半径加速グリッドを別途用意**: 常時必要な権威近傍探索がないため不採用。
- **AoI を持たず帯域を増やす**: 大規模戦闘の目標と矛盾するため不採用。
- **空間構造を新クレート化**: `dawn-sector` 内で全runtimeから共有でき、DAGを増やす利得がない。

---

## スコープ外

- セルサイズの具体値・チューニング
- LoD（遠方/非戦闘の更新間引き）
- クライアント側 prediction / reconciliation
- Sector の動的分割

---

## 実装チェックリスト

- [x] `dawn-sector::aoi::CellGrid` に床除算、27セル近傍、ShipId順列挙を実装
- [x] InitialState を 27 セルスコープに限定
- [x] Enter/Leave、Event filtering、MotionCorrection、warp PositionSnapを実装・テスト
- [x] `AoiFrame`へindex lifecycle、observer resolution、visible-set memory、ordered deliveryを統合
- [x] single-process、clustered simulation、production Sector Nodeを同じ`AoiFrame`へ移行
- [x] index policyをframeごとの全再構築と明記し、incremental policyを不採用と記録
- [x] admission/resume/recovery時に権威状態から再構築してseedするテストを追加
- [x] missing observerをfresh/resume/post-transitで明示拒否し、full-world fallbackを削除
- [x] runtime-path equivalenceを共有frame出力で検証
- [x] `--aoi-bench`でAoI有無の処理時間・配信量を比較

---

*提案: 2026-06-15。人間承認済み 2026-06-15。AoI frame lifecycle 統合: 2026-08-01（Issue #225）。missing observer拒否: 2026-08-01（Issue #234）。*
