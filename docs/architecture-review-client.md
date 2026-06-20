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
| テストカバレッジ | B | 新設4クラスは GdUnit4 で計55ケース実行確認済み。`main.gd` 本体（HUD構築呼び出し・マウス入力・イベント dispatch）は未テスト——シーンツリー/ネットワーク依存のため |
| サーバー側との対比 | — | サーバー側はクレート分割（A−）、クライアントはファイル分割（B+）。テストカバレッジは依然サーバー側（カバレッジ80%要件）が厚い |

サーバー側が長期にわたる分割リファクタ（Phase 2〜9）を経て A− に達したのに対し、
クライアントは 2026-06-20 の C-1 で初めて本格的な責務分離に着手し、4クラスの抽出と
GdUnit4 テスト基盤の整備（`scripts/setup-godot.*` による pin 済み Godot CLI）を完了した。

---

## ファイルサイズ一覧（2026-06-20 時点、C-1 完了後）

| ファイル | 行数 | 判定 |
|---|---|---|
| `client/scripts/main.gd` | 1084 | 🟡 オーケストレーション層（入力ルーティング・イベント dispatch・spawning・状態保持）に縮小。god object ではなくなったが、まだ最大ファイル |
| `client/scripts/hud_manager.gd` | 474 | 🟢 C-1で新設。HUD全パネルの構築・更新を持つ stateless static class |
| `client/scripts/ship_controller.gd` | 272 | 🟢 単一船の視覚表現に専念。結合なし |
| `client/scripts/connection.gd` | 245 | 🟢 WebSocket I/O とシグナル発行のみ。教科書的な境界 |
| `client/scripts/navigation_marker_renderer.gd` | 146 | 🟢 C-1で新設。ゲート/天体マーカー生成 + スペクトル色 |
| `client/scripts/camera_controller.gd` | 124 | 🟢 自己完結したオービットカメラ |
| `client/scripts/tactical_overlay.gd` | 93 | 🟢 射程リング描画のみ |
| `client/scripts/ship_picking.gd` | 90 | 🟢 C-1で新設。船/ゲート/天体ピッキング3関数 |
| `client/scripts/input_decoder.gd` | 85 | 🟢 C-1で新設。キー入力→アクション決定の純粋関数 |

合計 2,613 行のうち `main.gd` が41%を占める（C-1着手前69%から大幅低下）。
新設4ファイル（795行）はいずれも stateless static class で、main.gd の状態や
シーンツリーへの直接依存を持たない。

（`client/test/*.gd`（main_test.gd 68行 + ship_picking_test.gd 109行 +
navigation_marker_renderer_test.gd 112行 + input_decoder_test.gd 108行 +
hud_manager_test.gd 190行、合計587行）は別カウント。新設4クラスの全staticメソッドを
GdUnit4でテスト済み〔計55ケース、全件PASS〕。§「テストカバレッジ」参照）

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
| 1004–1084 | 状態クリア・空間環境構築・プレイヤーマテリアル | 🟢 |

---

## 問題一覧

### 解消済み

#### C-1: `main.gd` の責務過多（god object） — 完了（2026-06-20）

13以上の異種責務（入力・HUD構築・メッセージ取り込み・マーカー生成・ピッキング・
ワープ座標計算・カプ再現・環境構築・デュエルUI）が単一クラスに集約していた問題。
サーバー側で Phase 2〜7 を通じて解消した「肥大化した `node/mod.rs`」と同種の問題。

抽出結果（実施順）:

| 抽出先 | 元の対象行（抽出前） | 規模 | 形態 |
|---|---|---|---|
| `ship_picking.gd`（`ShipPicking`） | 船/ゲート/天体ピッキング3関数 | 90行 | stateless static class |
| `navigation_marker_renderer.gd`（`NavigationMarkerRenderer`） | ゲート/天体マーカー生成 + `spectral_color` | 146行 | stateless static class |
| `input_decoder.gd`（`InputDecoder`） | キー入力→アクション決定（F1–F8/S/J/A/W/Tab、**スコープ縮小**：マウス処理は除外） | 85行 | stateless static class |
| `hud_manager.gd`（`HudManager`） | HUD全パネル（status/ship status/target/module bar/duel overlay）の構築・更新 | 474行 | stateless static class（refs Dictionary を main.gd が保持） |

4回の抽出を通して `main.gd` は **1661 → 1084 行（-577行、-35%）**。
全抽出で挙動・外部から見える振る舞いは変更していない。

**`HudManager` の設計**: build_* 系メソッドは Control サブツリーを構築して
参照の Dictionary（例: `{conn_dot, conn_label, name_label, info_label}`）を返す。
main.gd はその Dictionary を自身のメンバ変数（`_status_panel_refs` 等）に保持し、
毎フレームの update_* 呼び出しに渡す。`HudManager` 自身は状態を持たず、
Control ノードの所有権は常に main.gd 側にある——他の3クラスと同じ
「呼び出し側がデータを渡し、クラスは計算/構築だけする」設計を、ノードを返す
ビルダーパターンに拡張したもの。

**マウス入力ハンドラを抽出しなかった理由**（`InputHandler` のスコープ縮小）:
マウスクリック側（`_check_double_click` のタイマー状態・カメラドラッグ判定・
`_module_slot_at` という `HudManager` の関数への依存・ピッキング結果からの選択状態
書き込み）はキーボード側と違って「入力 → 決定」のみに切り出せる純粋な形をしていない。
無理に抽出するとインターフェースが main.gd の状態をそのまま受け渡すだけの薄皮一枚に
なり、実質的な結合は変わらず複雑さだけが増す（Altitude 原則: 場当たり的な分割では
なく本質的な責務境界で分ける）。`HudManager` 抽出が完了して `_module_slot_at` は
`HudManager.module_slot_at()` を main.gd から呼ぶだけの形になったが、ダブルクリック
タイマー状態とカメラドラッグ判定は依然 main.gd 固有の処理であり、再評価の結論は
「現状維持」。

**`class_name` 抽出の手順上の注意**: 新しい `class_name` を追加した直後は、Godot が
プロジェクトを一度スキャンするまでグローバル識別子として認識されない
（`Identifier "X" not declared in the current scope` で CLI テストが失敗する）。
4回の抽出すべてで再現したので、`class_name` を追加するたびに必ず発生するものとして
手順化済み。`scripts/setup-godot.*` で取得した Godot バイナリで以下を一度実行して
キャッシュを再構築すること:
```bash
"$GODOT_BIN" --headless --editor --quit-after 3 --path client
```

#### C-2: 同型ロジックの2重実装（3箇所） — 解消済み（2026-06-20）

- ~~**マーカー生成**: `_spawn_gate_markers()` と `_spawn_body_markers()` は
  「子ノードクリア → 配列ループ → Node3D生成 → メッシュ/Label3D付与」の同一パターン。~~
  → 「子ノードクリア」を `_clear_children(root)` に、「サーバー座標 → Godot 座標変換」を
  `_server_to_godot_pos(p)` に抽出。メッシュ/Label3D の生成自体はゲート（トーラス）と
  天体（恒星/惑星で分岐するスフィア）で構造が異なるため統合せず、文字通り同一だった
  2箇所のみを共通化した（後に `navigation_marker_renderer.gd` へ移動、C-1）。
- ~~**ピッキング**: `_pick_ship_at()` / `_pick_gate_at()` / `_pick_body_at()` は
  「カメラレイ取得 → 対象配列ループ → レイ距離判定 → 最近接を返す」の同一パターン。~~
  → レイと点の距離計算を `_ray_point_distance()` に抽出し、3関数すべてで使用
  （後に `ship_picking.gd` へ移動、C-1）。候補データの取得方法・pick radius の
  計算は関数ごとに異なるため、ループ構造自体は統合せず数式部分のみ共通化した。
- ~~**ワープ着地点計算**: `_compute_warp_snap_pos()`（ゲート向け）と
  `_compute_body_warp_snap_pos()`（天体向け）は同一パターンの重複。~~
  → 方向ベクトル計算＋オフセット適用の共通部分を `_compute_warp_snap_pos_core()` に
  抽出し、係数も `GATE_WARP_ARRIVAL_FACTOR` / `BODY_WARP_ARRIVAL_FACTOR` として
  named constant 化した（main.gd に残置——ワープ計算は HUD 表示にも使うため）。

3件とも挙動・公開シグネチャは変更なし。Callable/lambda ベースの汎用ヘルパー化
（候補データ + 取得関数を渡す形）は当時 Godot エディタでの動作確認ができず見送ったが、
**`scripts/setup-godot.(sh|ps1)` で pin 済み Godot バイナリをローカル取得し GdUnit4 を
実際に CLI 実行できるようになった**ので、この制約は解消済み。実際、`ship_picking.gd`
の `pick_gate_at()` は座標変換用の `Callable`（`main.gd` の named method
`_server_to_godot_pos` を値として渡す）を受け取る形にしている——インラインの
マルチラインラムダではなく既存メソッド参照を渡す形なら構文リスクが小さいことを確認できた。

### テストカバレッジ（2026-06-20、C-1 完了時点）

| テストファイル | 対象 | ケース数 |
|---|---|---|
| `main_test.gd` | main.gd に残る純粋関数（`_server_to_godot_pos` / `_compute_warp_snap_pos_core`） | 4 |
| `ship_picking_test.gd` | `ShipPicking`（レイ距離・船/ゲート/天体ピッキング） | 8 |
| `navigation_marker_renderer_test.gd` | `NavigationMarkerRenderer`（スペクトル色・マーカー生成） | 8 |
| `input_decoder_test.gd` | `InputDecoder.decode_key()`（F1–F8/S/J/A/W/Tab の優先順位・ガード） | 15 |
| `hud_manager_test.gd` | `HudManager`（バー計算・パネル更新分岐・モジュールバー・デュエルオーバーレイ） | 20 |

**合計55ケース**、pin 済み Godot で実行確認済み（全件PASS、orphan node 0）。

テスト実行中に実際の不具合・ハマりどころを複数発見している（「Godot エディタなしでは
検証できない」という旧来の想定が剥がれたことで初めて見つかった類）:
- `main_test.gd` 初回実行時: テストの `Node3D` をシーンツリーに追加し忘れ、
  `global_position` が `(0,0,0)` 固定で読めてしまい1件が偶然PASSしていた。
- `class_name` を新規追加した直後（4クラスすべてで再現）: Godot のグローバル
  クラスキャッシュが未更新で `Identifier "X" not declared` と全件失敗——上記
  「`class_name` 抽出の手順上の注意」を参照。
- `hud_manager_test.gd` 初回実行時: `make_stat_bar()`/`make_mini_bar()` が返す
  Control ノードをどこにも `add_child()` せず放置していたテストが、Godot の
  orphan node 検出（`exit code 101`）に引っかかった。シーンに追加しないノードは
  `auto_free()` で明示的に解放する必要がある。

`main.gd` に残るマウス入力処理・イベント dispatch・spawning はテストよりも視覚的な
確認（クリック判定のズレ・HUDレイアウト崩れなど）が主な検証手段になる領域——
ネットワーク接続・実際のシーンインスタンス化が絡むため、ユニットテストの投資対効果が
他の4クラスより低い。

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

### 完了

| 項目 | 完了日 | 内容 |
|---|---|---|
| C-1 `main.gd` 分割 | 2026-06-20 | `ShipPicking` / `NavigationMarkerRenderer` / `InputDecoder`（キーボードのみ） / `HudManager` の4クラスを抽出。main.gd 1661→1084行（-35%）。マウス入力処理は意図的に残置（理由は上記） |
| C-2 重複ロジックの共通化 | 2026-06-20 | マーカー生成・ピッキング・ワープ着地点計算の同型ロジックを named helper に抽出 |

### 保留

| 項目 | 種別 | 状態・理由 |
|---|---|---|
| C-3 シーンツリー直パス参照 | 品質・保留 | 実害小。ノード構成変更が発生したときに合わせて対応すれば十分 |
| C-4 PlayerFitting スキーマ検証 | 品質・保留 | 現状ドリフトなし。サーバー側 JSON 形式を変更する ADR が出たときに合わせて対応 |

### 次の前進先（このレビューの範囲外）

`main.gd` の god object 問題は解消したため、クライアント側の次の課題は構造リファクタ
ではなく機能側（戦闘の深み、ADR-0016 §5）か、保留中の C-3/C-4 のトリガー待ちが妥当。
強引に main.gd をさらに細分化する（例: マウス入力処理を無理に抽出する）ことは
「採らない方針」として明記する。

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
