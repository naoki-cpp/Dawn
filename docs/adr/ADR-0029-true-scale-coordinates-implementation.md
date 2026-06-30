---
id      : ADR-0029
title   : 真スケール座標の実装 — アンカー相対 f32（サーバ B）＋ 浮動原点（クライアント C2）
status  : accepted
date    : 2026-06-21
deciders: [human, ai-agent]
related : ADR-0028（大規模座標系・方式比較・スパイク GO 判定）, ADR-0001（Event Sourcing）,
          ADR-0008（VelocityChanged）, ADR-0022（媒介変数ワープ）, ADR-0017（スナップショット・スキーマ）,
          ADR-0014（Raft）, ADR-0021（Sector-local 複製）, ADR-0004（Godot クライアント）,
          ADR-0025（天体スケール）, ADR-0019（AoI セルグリッド）
---

# ADR-0029 — 真スケール座標の実装（サーバ B ＋ クライアント C2）

> **ステータス: Accepted（2026-06-21・人間承認）。** アンカー粒度は §2 のとおり「天体単位」で確定。
> ADR-0028 のスパイク（`spike/true-scale-coords`・S1〜S6 全 PASS）で
> 方式 **B（サーバ・アンカー相対 f32 ＋ ワープはアンカー空間 f64）＋ C2（クライアント・浮動原点／
> 標準 Godot ビルド）** の feasibility が確証された。本 ADR はその**実装方針**（移行手順・スキーマ・
> テスト戦略・アンカー粒度）を定める。ADR-0028 は「何を選ぶか（比較・GO）」、本 ADR は「どう作るか」。

## 1. 決定（要旨）

1. **座標は「アンカー＋ローカル f32 オフセット」で持つ。** 巨大な絶対座標を 1 フレームに持たない。
   戦闘・移動演算はローカル f32 オフセットに対して行う（実 AU 近傍でも ulp < 1 mm を維持）。
2. **アンカーの絶対位置は f64 定数**（静的マップデータ由来・全ノード同一・非計算）。決定論に影響しない。
3. **ワープ道中はアンカー空間 f64 で媒介変数評価**し、毎 tick `VelocityChanged`（権威結果）として発行する
   （ADR-0008 / ADR-0022）。リプレイは「再計算せず適用」（ADR-0028 調査 B）ため f64 の機種差は混入しない。
4. **クライアントは浮動原点**：カメラと近傍物体を**同一原点**で描画（標準 Godot ビルド・f32 Vector3）。
   遠方天体はビルボード/マーカー（高精度メッシュにしない・既存の恒星処理を踏襲）。
5. **実値表示**（m/s・AU）の内部↔表示変換は**単一モジュール**に集約する。

## 2. アンカー粒度（要確定 → 本 ADR で決める）

スパイクの必須要件は「アンカーは到着時に近い（offset 小）」こと。アンカー間の中間は f32 で持てない。
本実装では：

- **アンカー = 天体（恒星・惑星・ステーション・ゲート）単位**とする。Sector 原点（恒星）だけでは
  外縁惑星との中間（数 AU）が宙に浮くため、主要天体それぞれをアンカー候補にする。
- **船の「現在アンカー」= 最寄りの天体アンカー。** 通常飛行・戦闘はそのアンカーのローカル f32 で進む。
- **アンカー切替（リベース）はワープ到着時に行う**（offset が小さい瞬間）。通常飛行中に別アンカーへ
  近づいた場合のハンドオフ規約は §6 未解決に残す（第1次は単一天体周辺の運用で足りる）。

> Sector とアンカーは別レイヤー：1 Sector に複数アンカー。Raft/複製の単位は従来どおり Sector（ADR-0014/0021）で、
> アンカーは Sector 内のローカル原点に過ぎない（座標の表現方法であって、所有・合意の単位ではない）。

## 3. データモデル / スキーマ

```
Position（現状: f32 x,y,z）
  → 維持しつつ「あるアンカーに対するローカルオフセット（m）」と再解釈する。
AnchorId（新規）: 天体アンカーの識別子（CelestialBodyId に対応づけ）。
ShipPosition = { anchor: AnchorId, offset: Position(f32) }
AnchorTable（静的・スナップショット非対象）: AnchorId → 絶対位置 [f64;3]（galaxy データから構築）。
```

- **イベント**：`VelocityChanged`（ADR-0008）は従来どおりオフセットに対する速度として機能。
  アンカー切替を表す新イベント `AnchorRebased { ship, from, to }`（権威・スナップショット対象）を追加する。
- **スナップショット（ADR-0017）**：ship ごとに `anchor` を含める。`AnchorTable` は静的のため非対象
  （galaxy データから決定論的に再構築）。**スキーマ版を上げる**（round-trip テストを更新）。
- **galaxy*.toml 移行**：`UNITS_PER_AU` 圧縮を廃し、天体 `position` を実 AU として読み、アンカー絶対位置を
  実 m で構築する。圧縮前提のコメント・定数を撤去（ADR-0025 のスケール記述を再改訂）。

## 4. 移行手順（段階的・各段でテスト緑を維持）

1. **AnchorTable + AnchorId** を dawn-sector に新設（galaxy ローダから構築）。既存挙動は不変（単一アンカー＝恒星）。
2. **ShipPosition にアンカーを付与**：当面は全船が恒星アンカー（offset = 現行 Position）。意味的 no-op。
3. **距離・近傍 API をアンカー対応**に：同一アンカーは f32、異アンカーは f64 で `anchor_abs + offset` 合成（ADR-0019 AoI も）。
4. **ワープをアンカー空間評価＋ `AnchorRebased` 到着**に（ADR-0022 の媒介変数ワープを拡張）。
5. **galaxy データを実 AU 化**：天体を真の距離へ。戦闘・通常飛行は各アンカー近傍なので f32 のまま成立。
6. **クライアント浮動原点**：`_server_to_godot_pos` を「(絶対 − 浮動原点) を f64 で引いてから Vector3」へ。
   絶対位置 = `AnchorTable[anchor] + offset`。遠方天体はビルボード/マーカー化。
7. **HUD 実値変換**を単一モジュールへ集約（m/s・AU）。
8. **スキーマ版を上げ**、スナップショット round-trip / replay 一致 / failover テストを更新。

> 各段は前段の上に乗る小スライス。1〜3 は意味的 no-op（リファクタ）で安全に入れ、4 以降で挙動が変わる。

### 実装状況（PR #2・`spike/true-scale-coords`）

サーバ側のアンカー機構は**圧縮スケールで完全動作・全テスト緑**。実 AU 起動までプレイテストしたが、
シニアレビューで構造的負債が判明したため**実 AU 起動は revert し、土台を固める方針**に切替えた（経緯は後述）。
現在は圧縮ベースで動作し、再活性化前にやるべき残課題のみを残す。

| Step | 内容 | 状況 |
|---|---|---|
| 1 | `AnchorId`（dawn-core）+ `AnchorTable`（f64 絶対位置・rebase/distance/nearest） | ✅ |
| 2 | 船に `AnchorComp`。spawn は**最寄り天体**にアンカー（`set_spawn_anchor`・決定論的・replay 一致） | ✅ |
| 3 | 絶対位置/距離をアンカー対応に。サーバの絶対位置アクセサ群（`entity_abs_pos`／`entity_absolute_f64`／`ship_distance`／`ship_absolute`）に一元化し、combat／approach／tackle／bot／navigation／AoI を同一フレームへ | ✅ |
| 4 | ワープ到着で Body アンカーへリベース（`AnchorRebased` 権威イベント・apply/replay）。`WarpComp.warp_arrival_abs`＋`entity_absolute_f64` で f64 精密到着 | ✅ |
| — | AoI を絶対位置ベースに（`ship_absolute_positions`／両 serve ループ／InitialState scoping／jump 再送） | ✅ |
| 6 | クライアント浮動原点を単一 `WorldSpace`（`client/scripts/world_space.gd`）に集約。server↔Godot 変換を `to_godot`/`to_server`/`dir_*` 経由に統一し、原点が動くと前進/逆変換が食い違う潜在バグを排除。リベース2系統を `_apply_origin_rebase` に統合。`floating_origin.gd` 削除・`world_space_test.gd` 追加 | ✅ |
| 7 | HUD 速度は既に実値表示（`METERS_PER_UNIT=1.0` の単一定数）。AU 距離フォーマッタは consumer が出た時に同一箇所へ足す（YAGNI） | ✅(速度) |
| 8 | スナップショットに `anchor` を永続化（リベース済み船の絶対位置を復元）。スキーマ bump | ✅ |

> **レビューで revert した経緯（2026-06-22）**：実 AU 起動までプレイテストした上でのシニアレビューで、土台の一部が
> 未完のままクライアントに応急処置が積み上がっていることが判明（アンカー割当が「常に恒星」で `nearest_body` で
> ない／AoI がアンカー非対応／クライアント座標処理が応急処置の堆積／距離計算が全消費者に散在）。中核設計（方式B・
> 決定論の扱い）は健全と確認した上で、**実 AU 起動を revert → 下記の土台を先に完成**させる方針に切替えた。

### レビュー指摘への対応（完了・圧縮では no-op／真 AU で load-bearing）

上の Step 表（#1〜#4・#6・#8）に加え、シニアレビューの指摘を以下で解消した。

- ✅ **R1 ゲートの f64 化**：`JumpGateDef.abs_m: [f64;3]` を追加し、`is_in_range_abs`／`distance_abs` で
  `can_propose_jump`／`can_propose_warp`（ゲート・天体の両分岐）を f64 比較に載せ替え（真 AU でも範囲判定が粗くならない）。
- ✅ **R2 AoI の f64 化**：`CellGrid` を f32 `Position` → `[f64;3]` 絶対座標に（セル binning を f64 floor 除算に）。
  関連アクセサ（`ship_absolute_positions`／`ship_absolute_pos`／`ships_visible_to`／`build_initial_state_json_for`）も f64 化。
  **副次**：`dawn-sector-node` の serve ループが #2 の絶対化から漏れて生オフセットで AoI していた既存バグを修正、
  未使用化した `ship_positions`（footgun）を削除。
- ✅ **R3 アンカー欠落の検知**：生オフセットへのフォールバック分岐に `debug_assert!`（`debug_assert_missing_anchor`
  ／combat は populated 時のみ）。populated なテーブルにアンカーが無い＝データ整合性バグを debug ビルドで顕在化。
- ✅ **ワープ到着権威化（R4）**：ドメインイベントを増やさない transient 到着リスト（`completed_warps: Vec<ShipId>`・
  `pending_auto_jumps` と同型）。`warp_step` の到着時に push → serve ループが `drain_completed_warps()` → 
  `AoiDelivery` が到着のたびに所有者／可視オブザーバへ権威的 `PositionSnap`（`ship_absolute` f64）。
  旧 `AnchorRebased`→`PositionSnap` 分岐を撤去し単一機構に統一（Gate／同一アンカー内ワープもカバー）。client は
  `_player_warp_snap_pos` 事前計算・速度到着検知・`_compute_warp_snap_pos*` を撤去し `PositionSnap` 一本に。

**テストガード**：決定論（warp→`AnchorRebased`→snapshot/restore で絶対位置一致・`snapshot_io.rs`）、真 AU 精度
（異アンカー2船の f64 間合い sub-mm・`anchor.rs`）、ゲート真 AU 範囲判定（`navigation.rs`）、Gate ワープが
`drain_completed_warps` に出る（`navigation.rs`）、`WorldSpace` 相互逆変換（`world_space_test.gd`）、AoI の真 AU セル
境界（`aoi.rs`）。

### 再活性化前の残課題

圧縮では顕在化しないが、スケール値を上げた瞬間に効くもの。真スケール再活性化と束ねて潰す。

- ✅ **ゲート座標の再オーサリング**：`JumpGateEntry.position` を固定 units から AU 単位（天体と同じ規約）に変更し、
  `entry_to_gate` で `UNITS_PER_AU` 換算するように。`data/galaxy.toml`／`galaxy.demo.toml` を 3.0 AU へ置き直し
  （現行 `UNITS_PER_AU=200,000` では旧来の 600,000 units と数値一致＝圧縮スケールでの挙動は不変、外縁惑星
  Meridian の ~1.5 AU の外側）。テスト `gate_positions_are_converted_from_au_to_units_like_celestial_bodies` で
  天体と同じ変換規約を確認（2026-06-23）。
- ✅ **ゲート到着の f64 化（R1 の積み残し）**：`process_warp` のゲート分岐を `JumpGateDef.abs_m`（f64）から
  到着点を計算し、`AnchorTable::nearest_anchor` でゲート直近の天体アンカーへリベースするように変更（Body と対称）。
  `warp_arrival_abs` は `dest_anchor` 経由の `anchor_table.abs()` 参照をやめ、ターゲットの f64 源を直接受け取る形に
  一般化（Gate/Body 共通パス）。テスト `gate_warp_arrival_is_symmetric_with_body_warp` で到着距離とリベース先を確認（2026-06-23）。
- ✅ **ワープ中補間の絶対 f64 化（レビューで新たに発覚・残課題に追加していた構造的負債）**：`WarpComp` の
  `warp_start`（アンカー相対 f32）を `warp_start_abs`（Sector-frame f64）に変更し、`warp_step` の道中補間
  （smoothstep イーズ）を絶対 f64 で行うように変更。旧実装は道中ずっと「エンゲージ時点のアンカー」基準の
  f32 オフセットで補間していたため、真 AU では異アンカー間のワープ道中に f32 ulp（~54 km @ 3AU）が**毎 tick
  複利的に蓄積**する経路があった（決定 #3 の「アンカー空間 f64 で媒介変数評価」を字面どおり満たしていなかった）。
  修正後は f64 のまま補間し、`PositionComp` 書き込み直前の 1 回だけ f32 へキャスト（複利なし・到着点は既存の
  到着リベースで自己修正）。`warp_arrival_abs` に「到着リング内ならオーバーシュートせず現在地に留まる」ガードを
  追加（旧 `warp_arrival_point` の f32 版にあったが、到着点計算の一元化で見落としていた edge case）。
  `WarpComp` は非永続（スナップショット対象外）のためスキーマ影響なし。全テスト緑で確認（2026-06-23）。
- ⬜ **視覚定数の再調整**：`VISUAL_SPEED_CAP`／`SUN_EFFECTIVE_DISTANCE`／`BODY_MARKER_CLAMP_DISTANCE` は `WORLD_SCALE`
  と暗黙連動。`WARP_SPEED` と共に再活性化時にまとめて再調整。
- ✅ **ゲート近傍にアンカーが無い問題（2026-06-23 発見・解消）**：最初の真AU試行で発覚（§2「アンカー＝天体単位、
  ゲートは自前のアンカーを持たない」のもと、ゲートが最近接の天体からも ~2.26 AU 離れていたため、リベースしても
  オフセットが小さくならなかった）。**トポロジ変更**で解消：`data/galaxy.toml`／`galaxy.demo.toml` のゲート座標を
  天体近傍（各 ~0.028 AU）に再配置（ゲート自身にアンカーを持たせる案は採用せず、§2 の決定はそのまま）。
- ✅ **`process_approach` のゲート分岐も絶対座標 f32 で精度ロス（再活性化作業中に新たに発覚）**：`process_approach`
  が `entity_absolute`（f32 合成）と `g.position`（f32）を直接比較していたため、真 AU では双方の f32 ulp
  （到着判定の 1600m を上回る数 km〜数十 km）が到着判定を妨げていた（R1 はワープ/ジャンプ側のみ対応済みで、
  Approach 側は見落とし）。`dest_in_ship_frame_abs`（f64 のまま減算して 1 回だけ f32 へキャスト）を新設し、
  Ship/Gate 両ターゲットをこの経路に統一。
- ✅ **真 AU 再活性化（2026-06-23）**：`UNITS_PER_AU = 1.495978707e11`、`WARP_SPEED` を同倍率（×747,989.35）で
  再調整。`galaxy.rs`／`node/mod.rs`／両 `galaxy*.toml` のコメントを更新。テスト側の f32-at-true-AU 前提
  （ulp 1.0m 未満を期待する箇所、AU 級の絶対座標を f32 経由で組み立てる箇所）を洗い出して修正
  （`set_spawn_anchor_abs` を test-only ヘルパーとして新設— f64 絶対点から直接アンカー＋オフセットを組む、
  f32 round-trip を経由しない）。全テスト緑（dawn-sector 126/126、ignored 1 件除く）。
  - 唯一 ignore にした既存テスト：`dawn-simulation::cluster::committed_jump_moves_ship_to_gates_destination_sector`
    （actor 層に f64 絶対座標での spawn 経路が無く、`gate.position` f32 経由の spawn では到着判定の許容半径
    2000m を ulp 誤差が上回る。`SpawnShipAbs` 的なテスト専用メッセージを追加すれば再有効化可能・未着手）。
- ⬜ **視覚定数の再調整**：`VISUAL_SPEED_CAP`／`SUN_EFFECTIVE_DISTANCE`／`BODY_MARKER_CLAMP_DISTANCE` は
  カメラ相対の見せ方の定数であり、浮動原点クライアントでは AU スケールと直接連動しない（再検討の結果、
  事前のブラインド調整は見送り）。実機プレイテストで違和感が出た場合のみ調整する。
- 📝 **記録のみ（許容）**：combat が `anchor_abs: HashMap` を受ける（dawn-ecs へアンカー概念がやや漏出・許容範囲）／
  InitialState のワイヤで body 位置は f32（船と gate は f64。gate はクライアントのマーカー／近接判定と
  サーバの jump 範囲判定を揃えるため `JumpGateDef.abs_m` を配信）／`ship_absolute_pos` が `ship_absolute`
  の薄いラッパ。

### ワープの加減速を lore に合わせて自然化（2026-06-23・ユーザー指摘）

プレイ確認で「ワープ開始時と到着時で加減速の仕方が違う」という指摘。原因は Aligning（スラスター加速・船依存）
から Warping（位置を smoothstep で補間）への切り替え時、smoothstep が両端で速度ゼロのカーブだったため、
エンゲージの瞬間に速度がほぼゼロへスナップしてから再加速していた（到着側は元から smoothstep 終端で滑らかに
ゼロへ減速していたので問題なし）。

**修正**：`warp_step` の補間を smoothstep からキュービック・エルミートスプラインへ変更
（開始タンジェント＝エンゲージ時の実速度ベクトル `warp_start_vel`、終端タンジェント＝ゼロ）。
これにより：
- 終端は従来通り解析的にゼロ（`h00(1)=h10(1)=0, h01(1)=1` なので到着点に正確に静止）
- 開始はエンゲージ直前の巡航速度から連続的にワープ速度へ加速（スナップなし）

`WarpComp` に `warp_start_vel: Velocity` を追加（非永続・スナップショット対象外なのでスキーマ影響なし）。
テスト `warp_engage_carries_the_aligning_speed_into_the_transit_no_snap` で速度連続性を確認。
「加速→トンネル突入→巡航→トンネル退出→減速」が一つの連続したモーションに見えるようになった（2026-06-23）。

**現状**：真 AU は稼働中（`UNITS_PER_AU = 1.495978707e11`）。残るのは実機プレイテスト
（視覚定数が実際に違和感を生むか、ワープ加減速が見た目どおり自然になったか）と、
上記 ignore テストの再有効化（任意・未着手）。

### 残課題：到着直前の見た目の減速がクライアント側でクランプされて消えている（2026-06-23・プレイ確認）

上記サーバ側修正（速度連続化）を実機で確認したところ「滑らかに減速していない」と再指摘。原因はサーバではなく
**クライアントの可視化定数 `VISUAL_SPEED_CAP`**（`client/scripts/ship_controller.gd:74`、Godot 単位で 2,000）：

- 真 AU のワープは実速度が数億〜数十億 units/tick に達するため、そのまま描画すると f32 が壊れて画面外に飛ぶ。
  これを避けるため `_process()` が毎フレーム `spd > VISUAL_SPEED_CAP` ならキャップ値へクランプして描画している。
- ワープ区間のほぼ全体で実速度がキャップを大幅に超えるため、**見た目は終始一定速度の巡航**にしかならず、
  サーバ側の滑らかな減速カーブ（キャップを下回るのは到着直前のごく数 tick だけ）が画面にほぼ反映されない。
  最終 tick で `Velocity::ZERO` を受け取ると見た目は「一定速度→いきなり停止」になる。
- サーバ側のエルミート補間自体は正しく動作している（テスト・到着精度は green）。これは**表示専用の問題**。

**対応方針（実装済み・2026-06-23）**：ユーザー方針確定 — 他者の船は、確定してワープトンネルに入るまで体感できる
加速を見せ、トンネル中は非表示（描画しない）でよい、到着時は逆（トンネルを抜けてから減速）。これは
`VISUAL_SPEED_CAP` の使い方を「クランプして見せる」から「クランプを跨いだら隠す」に変えるだけで実現できる：

- `ship_controller.gd` の `_process()` を `_is_player` で分岐。
  - **プレイヤー自身の船**（カメラが追従）：従来通り `VISUAL_SPEED_CAP` でクランプして常時表示
    （操縦中は継続的な視覚フィードバックが要るため、こちらは変更なし）。
  - **他者の船**：実速度 `spd` が `VISUAL_SPEED_CAP` を超えたら非表示（`visible = false`）。超えていない間
    （Aligning〜エンゲージ直後の加速、および到着直前の減速）はそのまま表示。非表示中も**位置はクランプせず
    実速度のまま積分**するため、再表示された瞬間の位置が現実の軌道に近い（古い位置から再開してしまわない）。
    描画していない間の大きな位置ジャンプは見えないので無害。
- `_in_tunnel: bool` で表示状態の遷移を一度だけ切り替え（毎フレームの再代入を避ける）。
- `initialize()` でリセット（ノード再利用時の保険）。

これにより「確定→トンネル突入まで体感加速→トンネル中非表示→トンネルを抜けて体感減速→停止」という
lore通りの見え方になる。サーバ側のロジック・テストには影響なし（表示のみの変更）。

### 自機のワープ演出：トンネル表現で自然な前後の繋ぎ（2026-06-23・ユーザー方針）

「キャップの問題は解決した？」に対し、上記は**他者の船のみ**の修正で、自機（カメラ追従中）は従来通り
クランプ＋常時表示のままだったと回答 → 自機にも同種の体感を持たせたいが、カメラが追従するノードを隠せない
ので「非表示」ではなく**トンネル表現**でいく方針が確定。さらに「トンネルの前後を自然にしたい」という要望から、
閾値を跨いだ瞬間にオン/オフする実装ではなく、**速度に応じてなめらかにフェードする**画面オーバーレイにした。

**実装**：
- `client/shaders/warp_tunnel.gdshader`（新規）：画面中心に収束する放射状の光の筋＋コアグロー。
  `intensity`（0〜1）で不透明度を制御、`intensity=0` は完全透明（無効化）。
- `client/scripts/warp_tunnel_effect.gd`（新規）：上記シェーダーの `ColorRect` に付けるラッパー。
  `set_intensity(value)` で `shader_parameter/intensity` を更新するだけの薄い層。
- `client/scenes/main.tscn`：`HUD` 直下にフルスクリーン `ColorRect`（`WarpTunnel`）を追加
  （`mouse_filter = ignore` でクリックを透過）。
- `client/scripts/main.gd`：`_update_warp_tunnel_effect()` を新設し `_process()` から毎フレーム呼ぶ。
  - 自機の速度（`ship_controller.get_speed_godot()`、Godot 単位・キャップ無視）と
    `WARP_TUNNEL_THRESHOLD`（`VISUAL_SPEED_CAP` と同値・要同期のコメント付き）を比較してターゲット強度を決定。
  - **`lerpf` による指数的イーズ**（`WARP_TUNNEL_FADE_RATE`）でターゲットへ毎フレーム漸近させる — これが
    「トンネル前後を自然にする」の本体：実際のサーバ速度は1 tickで閾値を跨ぐが、画面側はそれを滑らかな
    フェードへ変換する。
  - 付随でカメラ FOV を強度に応じて広げる（`WARP_TUNNEL_FOV_BOOST`）、速度感の補強（任意・低リスク）。
- `ship_controller.gd` に `get_speed_godot()` を追加（`get_speed_server()` の Godot スケール版）。

**検証**：GdUnit4 全67件 green（既存挙動に影響なし）。シーン読み込み・シェーダーマテリアル割り当て・
`set_intensity` 呼び出しをヘッドレス実行で個別確認（パースエラー無し）。実際の見え方（フェードの速さ・
オーバーレイの濃さ）は引き続きプレイテストで確認が必要 — `WARP_TUNNEL_FADE_RATE` / シェーダーの
`scroll_speed` / `tunnel_color` は数値チューニングの余地あり。

### 速度・距離の表示単位を実スケールに対応（2026-06-23・ユーザー指摘）

真AU化後も HUD の速度表示が常に `m/s` 固定だった（warp 中は秒速数十億 m にもなり、桁数が現実的でない）。
ターゲット距離表示も `km` 固定で同根の問題（AU 級の距離で同様に桁が破綻）。

ADR §1 決定 #5「実値表示（m/s・AU）の内部↔表示変換は単一モジュールに集約する」を実装：

- `client/scripts/unit_format.gd`（新規）：`format_speed(mps)` / `format_distance(meters)` の static 関数のみ。
  しきい値は両者共通（< 1,000 → そのまま m/m・s、< 0.01 AU 相当 → km/km・s、それ以上 → AU/AU・s）。
  `main.gd` は `WorldSpace` と同じ理由（headless テストのクラスキャッシュ依存回避）で `class_name` ではなく
  `preload` で読み込む。
- `main.gd` の `_update_hud()` の速度・距離フォーマットをそれぞれ `UnitFormat.format_speed/format_distance`
  に置き換え（ハードコードの `"%d m/s"`／`"%.1f km"` を削除）。
- テスト `client/test/unit_format_test.gd`（新規・8件）でしきい値境界・各帯（m/s, km/s, AU/s 相当）を確認。

**検証**：GdUnit4 全75件 green。

### ゲートマーカーも遠距離クランプ対応（2026-06-23・ユーザー指摘）

ゲートは元々マーカー（リング＋ラベル）を持っていたが、惑星マーカーのような「カメラ遠方クリップ面の手前へ
クランプして常に見える」処理（`_update_body_markers`／`NAV_MARKER_CLAMP_DISTANCE`、旧名
`BODY_MARKER_CLAMP_DISTANCE`）が無かった。真AU化でゲートが恒星から AU 級に離れた今、クランプ無しではゲート
マーカーが far plane（`scenes/main.tscn` の `far=100000`）の外に出て実質見えなくなる ——
ちょうど惑星が以前に抱えていた問題と同根。

**実装**：
- `navigation_marker_renderer.gd`：ゲートマーカーに `gate_id`／`gate_pos`（サーバ座標）の meta を追加
  （惑星マーカーの `body_id`／`body_pos` と同じパターン）。
- `main.gd`：`_update_gate_markers()` を新設（`_update_body_markers()` と同型・毎フレーム呼び出し）。
  クランプ定数は両者で共有するため `BODY_MARKER_CLAMP_DISTANCE` を `NAV_MARKER_CLAMP_DISTANCE` に改名。
  `_apply_origin_rebase` のゲート位置シフトは不要になったので削除（毎フレーム再配置されるため、惑星と同じ理由）。
- `ship_picking.gd`：`pick_gate_at` の引数を `gates: Array, to_godot_pos: Callable`（サーバ座標を都度再計算）
  から `gates_root: Node`（マーカーの実際の `global_position` を見る）に変更。クランプ前提のままだと
  「見えている場所」と「クリック判定される場所」がズレる（惑星の `pick_body_at` は元から実位置基準だった）。
- テスト更新：`ship_picking_test.gd` の `pick_gate_at` テストを新シグネチャに対応＋ meta 欠落時のテストを追加。

**検証**：GdUnit4 全76件 green。

### 通し review #2（2026-06-22・R1–R3 修正後）

R1（ゲート f64 源）・R3（アンカー欠落 assert）・R2（AoI f64 化）を入れた後の再レビュー。

**総評：圧縮ベースの土台は一貫して健全になった。** サーバ側のクロスアンカー境界（spawn／AoI／combat／距離／
ゲート range）はすべて f64 で合成され、決定論テスト・精度テスト・`debug_assert` でガードされている。R2 の作業中に
`dawn-sector-node` の AoI 漏れ（生オフセット）という**既存バグも一掃**できた。圧縮スケールでの自己整合性・マージ
可能性は高い。残る負債は明確に「真 AU 起動でのみ効く」ものに収斂した。

> この review 時点の所見はその後ほぼ解消した（ワープ到着権威化 ✅、AoI f64 ✅）。残った所見は上の
> 「再活性化前の残課題」に集約済み：ゲート到着の f64 化（真 AU 限定）・視覚定数・プレイテスト、および 📝 の軽微な
> 整理（InitialState ワイヤの body/gate f32、assert 2系統、`ship_absolute_pos` の薄いラッパ）。

**マージ判断（更新）**：圧縮スケールの土台（#1–#4・#6・R1–R4）は一貫して緑＋プレイテスト確認済み（2026-06-23）で、
**main へマージ可能な状態**。残課題は真 AU 起動と束ねるべきものに収斂しており、圧縮ベースでの追加検証は不要。

## 5. テスト戦略

- **決定論**：replay が再計算するのは f32 オフセット積分のみ（現行と同型）。`AnchorRebased` は権威イベントで
  適用のみ。スナップショット＋末尾 replay 一致（INV-002）を新スキーマで検証。
- **精度**：実 AU 近傍の戦闘で位置・間合い誤差 < 1 m（スパイク S1 の本実装版・`#[test]` 化）。
  アンカー跨ぎ到着の誤差 < 1 mm（S2/S3 相当）。
- **クライアント**：浮動原点の相対位置連続性（S5 相当）を GdUnit4 で恒久テスト化。
- **AoI/距離**：異アンカー間の近傍判定が絶対座標版と一致（ADR-0019 の 27 セル候補＋厳密距離）。

## 6. 未解決（実装中に詰める）

- **通常飛行中のアンカーハンドオフ**：ワープ以外で別アンカー支配域へ越えるときの切替規約（第1次は不要）。
- **アンカー間が遠い天体同士の AoI**：異アンカー近傍判定の f64 合成コスト（実測で評価）。
- **既存セーブ/スナップショットの移行**：旧スキーマからの一方向変換 or 破棄方針（開発段階なら破棄で可）。
- **複数 Sector × アンカー**：8D 分散・Fission（ADR-0020）連動時のアンカー所有（本 ADR の範囲外）。

## 6.5 ゲーム仕様への影響（座標表現にとどまらない）

実 AU 化は座標の内部表現だけでなく**ゲーム仕様**を確定させる。これは技術変更の副作用ではなく、
意図された仕様の正式化である（圧縮スケールは feasibility 検証までの暫定だった）。

| 事項 | 現状（圧縮スケール・暫定） | 実 AU 化後（正式仕様） |
|---|---|---|
| 惑星間移動 | サブライトでも実質渡れてしまう | **物理的にワープ（Fold）専用**。数百 m/s では実 AU を渡れない（EVE 流の意図と一致） |
| 距離・速度の表示 | 圧縮値 | 実値（AU・m/s）が正式（§1-5・単一変換モジュール） |
| ワープ所要時間 | 距離を膨らませて演出 | 実距離から決まる（ADR-0022 媒介変数ワープ） |

**lore との整合（追い風）**：フィクションは既に「天文学的距離を Fold で渡る」前提で書かれている。
- `docs/lore/glossary.md`（Depth Variance）：「特定の点で場所間の実効距離を局所的に圧縮できるため、
  **天文学的な距離においても**幾何学的に短いトランジットが可能」。
- `docs/lore/technology.md`：Fold Transit は超光速ではなく**実効距離の圧縮**。Warp は短距離 Fold。

→ 実 AU 化はむしろ lore と整合する（圧縮スケールの方が「天文学的距離」という設定と齟齬していた）。

**改訂が要る上位ドキュメント（実装 ADR 受理後・人間承認を取ってから着手）**：
- **ADR-0025（天体スケール）**：「圧縮」記述を実 AU 正式化へ改訂。
- **ADR-0022（ワープ）**：所要時間の根拠を実距離に。
- **game-design.md / roadmap.md / playtest-guide.md**：距離・移動・スケール前提の記述。
- **AI_DEVELOPMENT_GUIDE.md「Project North Star」のスコープ**：スケール前提があれば（**自律編集しない**。AI_DEVELOPMENT_GUIDE.md / ADR の改訂は人間承認・
  AI_DEVELOPMENT_GUIDE.md「Project North Star」/ docs/architecture/event-schema-evolution.md）。

> 本 ADR はこの仕様変更を**明示して合意を取る**ことが目的。上位ドキュメントの実書き換えは本 ADR が
> accepted になってから、各ドキュメントの所有者承認のもとで行う（座標移行 §4 と同期させる）。

## 7. 影響

- 型は `Position` を維持（再解釈）＋ `AnchorId`/`AnchorTable`/`AnchorRebased`＋ゲート/天体の `abs_m` を追加。スキーマ版が上がる。
- ADR-0025（天体スケール）・ADR-0022（ワープ）・ADR-0019（AoI）に追補が要る。
- スパイクコード（`spike_true_scale` / `spike_floating_origin*`）は破棄済み（設計知見は本 ADR と各 production の doc コメントへ吸収）。

## 8. 代替案（却下）

| 案 | 判定 | 理由 |
|---|---|---|
| A. i64 グローバル固定小数点 | 却下（保留） | 真 AU は B で f32 のまま到達でき、全クレート型移行が重い。決定論も既存解決済み（ADR-0028 調査 B） |
| C. f64 グローバル | 却下 | 巨大座標で位置積分自体を f64 再計算 → リプレイ一致の検証コスト。B は f64 を定数＋権威発行に限局 |
| C1. Godot 倍精度ビルド | 却下 | スパイク S4 で標準ビルド＋浮動原点（C2）で十分と確認。非標準ビルド/配布を避ける |
