---
scope    : Godot クライアント（client/scripts/）の保守性・設計品質レビュー
audience : AI Agent / Human Developer
update   : クライアント側で大規模リファクタ実施後 / 新スクリプト追加時
related  : docs/architecture-review-server.md（サーバー側）, docs/architecture.md, docs/playtest-guide.md
date     : 2026-06-20（C-1 完了後に更新）
---

# Architecture Review — Dawn Client (Godot)

サーバー側 [architecture-review-server.md](./architecture-review-server.md) のクライアント版。
**分析のみ。コード変更はロードマップに従い段階的に実施すること。**

---

## 現状評価

**総合: B−**（2026-06-20、C-1 完了で C+ から上昇）

| 観点 | 評価 | 理由 |
|---|---|---|
| ファイル分割 | B+ | `main.gd` から `HudManager`/`NavigationMarkerRenderer`/`ShipPicking`/`InputDecoder` の4クラスを抽出（C-1完了）。残るのは入力ルーティング・イベント dispatch・spawning・状態保持のオーケストレーション層 |
| `main.gd` の責務集約 | B− | god object はほぼ解消。マウス入力処理（ダブルクリック判定・HUD連動）のみ意図的に残置（理由は C-1 参照） |
| 重複 | A− | マーカー生成・ピッキング・ワープ着地点計算の同型ロジックは解消済み（C-2） |
| 結合度 | B− | signal 経由の `connection.gd` ↔ `main.gd` 結合は良好。`@onready` のシーンツリー直パス参照と modules dict のキー前提は脆い（C-3/C-4、保留） |
| デッドコード | A | 残骸なし。コメントは ADR 参照付きで現状と一致 |
| テストカバレッジ | B | 新設4クラス + main.gd残存ロジックの一部（モジュールdeactivate判定など）を GdUnit4 で計58ケース実行確認済み。HUD構築呼び出し・マウス入力・イベント dispatch本体は未テスト——シーンツリー/ネットワーク依存のため |
| サーバー側との対比 | — | サーバー側はクレート分割（A−）、クライアントはファイル分割（B+）。テストカバレッジは依然サーバー側（カバレッジ80%要件）が厚い |

サーバー側が長期にわたる分割リファクタ（Phase 2〜9）を経て A− に達したのに対し、
クライアントは 2026-06-20 の C-1 で初めて本格的な責務分離に着手し、4クラスの抽出と
GdUnit4 テスト基盤の整備（`scripts/setup-godot.*` による pin 済み Godot CLI）を完了した。

---

## ファイルサイズ一覧（2026-06-20 時点、C-1 完了後）

| ファイル | 行数 | 判定 |
|---|---|---|
| `client/scripts/main.gd` | 1094 | 🟡 オーケストレーション層（入力ルーティング・イベント dispatch・spawning・状態保持）に縮小。god object ではなくなったが、まだ最大ファイル |
| `client/scripts/hud_manager.gd` | 474 | 🟢 C-1で新設。HUD全パネルの構築・更新を持つ stateless static class |
| `client/scripts/ship_controller.gd` | 272 | 🟢 単一船の視覚表現に専念。結合なし |
| `client/scripts/connection.gd` | 245 | 🟢 WebSocket I/O とシグナル発行のみ。教科書的な境界 |
| `client/scripts/navigation_marker_renderer.gd` | 146 | 🟢 C-1で新設。ゲート/天体マーカー生成 + スペクトル色 |
| `client/scripts/camera_controller.gd` | 124 | 🟢 自己完結したオービットカメラ |
| `client/scripts/tactical_overlay.gd` | 93 | 🟢 射程リング描画のみ |
| `client/scripts/ship_picking.gd` | 90 | 🟢 C-1で新設。船/ゲート/天体ピッキング3関数 |
| `client/scripts/input_decoder.gd` | 85 | 🟢 C-1で新設。キー入力→アクション決定の純粋関数 |

合計 2,623 行のうち `main.gd` が42%を占める（C-1着手前69%から大幅低下）。
新設4ファイル（795行）はいずれも stateless static class で、main.gd の状態や
シーンツリーへの直接依存を持たない。

（`client/test/*.gd`（main_test.gd 108行 + ship_picking_test.gd 109行 +
navigation_marker_renderer_test.gd 112行 + input_decoder_test.gd 108行 +
hud_manager_test.gd 190行、合計627行）は別カウント。新設4クラスの全staticメソッド +
main.gd残存ロジックの回帰テストをGdUnit4でテスト済み〔計58ケース、全件PASS〕。
§「テストカバレッジ」参照）

---

## main.gd 内部構造（行範囲別、C-1 完了後）

| 行範囲 | 内容 | 評価 |
|---|---|---|
| 1–40 | ノード参照（`@onready`）・HUD参照用 Dictionary 変数宣言 | 🟢 |
| 42–135 | 定数・内部状態（船/HP/モジュール/ゲート・天体配列/選択状態） | 🟡 ドメインごとに分類はされているが量が多い |
| 137–163 | `_ready()` / `_process()` | 🟡 `_process()` がカプ再現・近接判定・太陽方向・HUD更新の集約呼び出し点になっている |
| 164–188 | ゲート/天体マーカー生成（呼び出しのみ） | 🟢 本体は `navigation_marker_renderer.gd` |
| 189–243 | 太陽方向シェーダー更新・ゲート近接判定 | 🟢 `NavigationMarkerRenderer.spectral_color()` を利用 |
| 244–313 | `_input()`（キー判定は `InputDecoder` へ委譲・マウス処理は残置） | 🟡 マウスのダブルクリック判定・HUD連動・ピッキング選択は状態を持つため main.gd に残置（理由は C-1 参照） |
| 314–395 | ダブルクリック判定・船/ゲート/天体ピッキング・選択 | 🟢 ピッキング本体は `ship_picking.gd` |
| 396–454 | ロックオン・移動・停止コマンド送信 | 🟢 |
| 455–788 | サーバーイベント dispatch（jump/system change/AoI/fitting/destroyed等） | 🟢 個々のハンドラは narrow |
| 789–908 | ワープ着地点計算（ゲート/天体） | 🟢 共通コア `_compute_warp_snap_pos_core` に抽出済み（C-2）。GdUnit4 テスト済み |
| 909–970 | HUD更新ディスパッチ（`HudManager` 呼び出し） | 🟢 値の整形（速度文字列・距離文字列等）のみ main.gd 側、Control構築・描画は `HudManager` |
| 971–1003 | クライアント側カプ再現シミュレーション | 🟢 サーバーの `CapacitorSystem` を正しく模倣 |
| 1004–1094 | 状態クリア・空間環境構築・プレイヤーマテリアル | 🟢 |

---

## 問題一覧

### 解消済み（要約）

| ID | 内容 | 解消内容 |
|---|---|---|
| C-1 | `main.gd` god object（13以上の異種責務） | 4クラスに分割抽出（下表）。main.gd 1661→1084行（-35%）。挙動変更なし |
| C-2 | マーカー生成/ピッキング/ワープ着地点計算の同型ロジック2重実装 | 各組の「文字通り同一」な部分のみ named helper に抽出（後にC-1で各クラスへ移動）。挙動変更なし |

C-1 の抽出先（実施順）:

| 抽出先 | 内容 | 規模 |
|---|---|---|
| `ship_picking.gd`（`ShipPicking`） | 船/ゲート/天体ピッキング3関数 | 90行 |
| `navigation_marker_renderer.gd`（`NavigationMarkerRenderer`） | ゲート/天体マーカー生成 + `spectral_color` | 146行 |
| `input_decoder.gd`（`InputDecoder`） | キー入力→アクション決定（F1–F8/S/J/A/W/Tab のみ。マウス処理は意図的に除外※） | 85行 |
| `hud_manager.gd`（`HudManager`） | HUD全パネル（status/ship status/target/module bar/duel overlay）の構築・更新 | 474行 |

いずれも stateless static class（GdUnit4テスト付き）。`HudManager` のみ build_* が
Control サブツリーを構築して参照 Dictionary を返すビルダー形式（main.gd がノード
所有権を持ち続ける）で、他3クラスは「呼び出し側がデータを渡し、計算/構築だけする」形。

※ マウス入力（ダブルクリック判定・HUD連動・ピッキング選択）は状態・依存が絡み
「入力→決定」の純粋形に切り出せないため main.gd に残置。詳細は採らない方針を参照。

**運用上の注意**: `class_name` を新規追加した直後は Godot がプロジェクトを
スキャンするまでグローバル識別子として認識されない（CLI テストが
`Identifier "X" not declared` で失敗する）。`class_name` 抽出のたびに発生する。
```bash
"$GODOT_BIN" --headless --editor --quit-after 3 --path client
```

### テストカバレッジ（C-1 完了時点 + 以降の回帰テスト追加）

| テストファイル | 対象 | ケース数 |
|---|---|---|
| `main_test.gd` | main.gd 残存純粋関数 + モジュールdeactivate判定の回帰テスト | 7 |
| `ship_picking_test.gd` | `ShipPicking` | 8 |
| `navigation_marker_renderer_test.gd` | `NavigationMarkerRenderer` | 8 |
| `input_decoder_test.gd` | `InputDecoder` | 15 |
| `hud_manager_test.gd` | `HudManager` | 20 |
| **合計** | | **58**（全件PASS、orphan node 0） |

テスト導入で見つかった不具合・定着した手順（詳細: `AI_DEVELOPMENT_GUIDE.md` §8）:
- `Node3D` をシーンツリーに追加せず `global_position` を読むと `(0,0,0)` 固定になる
- `class_name` 新規追加直後はキャッシュ未更新で全件失敗する（上記コマンドで解消）
- `add_child()` しない Control ノードは `auto_free()` で明示的に解放する（orphan node 検出）

`main.gd` に残るマウス入力処理・イベント dispatch・spawning は、ネットワーク接続や
シーンインスタンス化が絡むためテストよりも視覚的な確認が主な検証手段になる領域。

### Medium（保留）

#### C-3: シーンツリー直パス参照の脆さ

`@onready` で `$Connection` / `$World/Ships` / `$World/Gates` / `$World/Bodies` /
`$HUD` 等を直接パス参照しており、`main.tscn` のノード構成を変更すると
silent にではなく実行時エラーで気づく形になる（null チェックなし）。
現状ノード構成は安定しているため実害は小さいが、シーン構造変更時は要注意。

#### C-4: PlayerFitting dict のスキーマ非検証

`_player_modules` 配列の各要素は `"is_active"` / `"module_id"` / `"slot"` /
`"cap_cost_per_cycle"` / `"stat_delta"` 等の特定キーを前提に読まれるが、
`connection.gd` 側でのスキーマ検証はない。サーバー側 JSON 形式
（`serialization.rs` の `build_player_fitting_json()`）とキー名が食い違うと
silent に値が欠落する（GDScript の `Dictionary.get()` はデフォルト値で握り潰す）。

---

## 改善ロードマップ

C-1/C-2 は解消済み（上記「問題一覧」参照）。残るは C-3/C-4 の保留のみ——
いずれも実害が小さく、トリガー（ノード構成変更 / サーバーJSON形式変更のADR）が
発生したときに対応すれば十分。

`main.gd` の god object 問題は解消したため、クライアント側の次の課題は構造リファクタ
ではなく機能側（戦闘の深み、ADR-0016 §5）か、C-3/C-4 のトリガー待ちが妥当。

### 採らない方針

- main.gd を複数の `.tscn` 化されたコンポーネント（個別シーン+スクリプト）に分割することは、
  シーン参照切れのリスクが高い。pin 済み Godot CLI で構文・実行エラーは検出できるようになったが、
  シーンツリー構成の妥当性（ノードパスの解決・レイアウト）はヘッドレス実行だけでは確認しきれない。
  GDScript ファイル内の `class_name` 抽出（同一シーンに留める）の方が安全で、C-1 では
  実際にこの方式で4クラスを抽出し、GdUnit4 で検証できた。
- マウス入力処理（ダブルクリック判定・HUD連動）の抽出は、C-1 で見送った理由が今も
  有効なため、当面行わない（再評価のトリガー: C-3 のシーンツリー依存が解消されたとき）。

---

## 触らない箇所（安定・枯れている）

- `connection.gd` — WebSocket I/O とシグナル発行のみ。ドメインロジックなし。教科書的な境界
- `ship_controller.gd` — 単一船の視覚表現に専念。他システムへの結合なし
- `camera_controller.gd` — 自己完結したオービットカメラ。依存はターゲットノード参照のみ
- `tactical_overlay.gd` — 射程リング描画のみ。受け取った値を描くだけで状態を持たない
- `ship_picking.gd` / `navigation_marker_renderer.gd` / `input_decoder.gd` /
  `hud_manager.gd` — C-1 で新設した stateless static class 群。いずれも GdUnit4
  テスト付きで挙動が固定されている。他クラスへの依存はメソッド引数経由のみ
  （Camera3D・候補データ・Callable・refs Dictionary）で、main.tscn のノード構成
  変更の影響を受けない
- `main.gd` のイベント dispatch 層（455–788行） — 個々のハンドラは narrow で ADR 参照付き
