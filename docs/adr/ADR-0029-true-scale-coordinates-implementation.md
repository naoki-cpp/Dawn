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
  `pending_auto_jumps` と同型）。`warp_step` の到着時に push → 3 serve ループが `drain_completed_warps()` → 
  `deliver_aoi_frame` が到着のたびに所有者／可視オブザーバへ権威的 `PositionSnap`（`ship_absolute` f64）。
  旧 `AnchorRebased`→`PositionSnap` 分岐を撤去し単一機構に統一（Gate／同一アンカー内ワープもカバー）。client は
  `_player_warp_snap_pos` 事前計算・速度到着検知・`_compute_warp_snap_pos*` を撤去し `PositionSnap` 一本に。

**テストガード**：決定論（warp→`AnchorRebased`→snapshot/restore で絶対位置一致・`snapshot_io.rs`）、真 AU 精度
（異アンカー2船の f64 間合い sub-mm・`anchor.rs`）、ゲート真 AU 範囲判定（`navigation.rs`）、Gate ワープが
`drain_completed_warps` に出る（`navigation.rs`）、`WorldSpace` 相互逆変換（`world_space_test.gd`）、AoI の真 AU セル
境界（`aoi.rs`）。

### 再活性化前の残課題

圧縮では顕在化しないが、スケール値を上げた瞬間に効くもの。真スケール再活性化と束ねて潰す。

- ⬜ **ゲート座標の再オーサリング**：配置値（現状 600,000 units 固定で `UNITS_PER_AU` 非連動）を真 AU 向けに
  `UNITS_PER_AU` 連動でセクター縁へ置き直す（精度ではなく座標値の設計）。
- ⬜ **ゲート到着の f64 化（R1 の積み残し・真 AU 限定）**：到着**権威**は解決済み（client は `PositionSnap` で正しく着地）。
  残るは server 側の到着**座標精度** — `process_warp` のゲート分岐が `g.position`（f32）・`dest_anchor=None` のままなので、
  真 AU では到着点が ~16 km 粗く到着後も恒星アンカー。再活性化時に **Body と対称化**（f64 到着点＋最寄りアンカーへリベース）。
- ⬜ **視覚定数の再調整**：`VISUAL_SPEED_CAP`／`SUN_EFFECTIVE_DISTANCE`／`BODY_MARKER_CLAMP_DISTANCE` は `WORLD_SCALE`
  と暗黙連動。`WARP_SPEED` と共に再活性化時にまとめて再調整。
- ⬜ **圧縮スケールでのプレイテスト**：ワープ到着権威化を入れた後の実機確認（ワープ→到着の見え方・カメラ・複数クライアント）。
- 📝 **記録のみ（許容）**：combat が `anchor_abs: HashMap` を受ける（dawn-ecs へアンカー概念がやや漏出・許容範囲）／
  InitialState のワイヤで body・gate 位置が f32（船は f64・クライアントはマーカー描画ゆえ実害小）／`ship_absolute_pos`
  が `ship_absolute` の薄いラッパ。いずれも再活性化時に整理可。

**真スケール再活性化の手順**（上記残課題を消化後）：`galaxy.rs` の `UNITS_PER_AU` を `1.495978707e11` に、
`WARP_SPEED` を再調整し、データ flip ＋ プレイテストで詰める。

### 通し review #2（2026-06-22・R1–R3 修正後）

R1（ゲート f64 源）・R3（アンカー欠落 assert）・R2（AoI f64 化）を入れた後の再レビュー。

**総評：圧縮ベースの土台は一貫して健全になった。** サーバ側のクロスアンカー境界（spawn／AoI／combat／距離／
ゲート range）はすべて f64 で合成され、決定論テスト・精度テスト・`debug_assert` でガードされている。R2 の作業中に
`dawn-sector-node` の AoI 漏れ（生オフセット）という**既存バグも一掃**できた。圧縮スケールでの自己整合性・マージ
可能性は高い。残る負債は明確に「真 AU 起動でのみ効く」ものに収斂した。

> この review 時点の所見はその後ほぼ解消した（ワープ到着権威化 ✅、AoI f64 ✅）。残った所見は上の
> 「再活性化前の残課題」に集約済み：ゲート到着の f64 化（真 AU 限定）・視覚定数・プレイテスト、および 📝 の軽微な
> 整理（InitialState ワイヤの body/gate f32、assert 2系統、`ship_absolute_pos` の薄いラッパ）。

**マージ判断（更新）**：圧縮スケールの土台（#1–#4・#6・R1–R4）は一貫して緑で、main へ載せる価値がある状態。
残課題は真 AU 起動と束ねるべきものに収斂した。残る検証は圧縮スケールでのプレイテストのみ。

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
- **CLAUDE.md §1 スコープ**：スケール前提があれば（**自律編集しない**。CLAUDE.md / ADR の改訂は人間承認・
  CLAUDE.md §1/§7）。

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
