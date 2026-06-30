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
現在のフェーズ : Phase 8A / 8C / 8B 一区切り（2026-06-15）— スケール基盤の中核が稼働
フェーズの状態 : 全 workspace グリーン。
                - 8A（durability）: 2 層ログ + snapshot 圧縮 + CheckpointScheduler（ADR-0017）
                - 8C（AoI）: 静的セルグリッド 3×3×3 + Enter/Leave 差分配信 + イベントフィルタ（ADR-0019）
                - 8B（一区切り）: 局所 TiDi コア（決定論的・非破壊・自動回復）+ 入場バックストップ
                  （生カウント・--pop-cap）。柱①の主要レバーが単一 Sector 内で出揃った（ADR-0018）。
                残り 8B（Fission / LoD=ADR-0020 deferred / 越境 TiDi / SLA イベント化）は
                独立 ADR か 8D 連動で後続（§10「Phase 8B 一区切り」ボックス参照）。
                次の前進先: 8D（物理ノード分散）か 戦闘の深み（ADR-0016 §5: Tackle 〜 Logistics）。
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
  RepairSystem（Step 6.5）、RepairApplied、Godot 緑フラッシュまで実装済み。
  次は本体の Logistics（遠隔修理）
- ✅ Godot クライアント構造リファクタ + テスト基盤（2026-06-21）— `main.gd` の god object を
  `HudManager`/`NavigationMarkerRenderer`/`ShipPicking`/`InputDecoder` の4クラスへ分割
  （1661→1094行）。`scripts/setup-godot.*` で pin 済み Godot CLI を取得し GdUnit4 を導入、
  計58ケースを実行確認（詳細: `docs/architecture/architecture-review-client.md`）。モジュール
  ON/OFF→CAP!誤表示のバグ修正も含む。Rust側は343テスト全パス

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

**Phase 8A（durability）/ 8C（AoI・8C-5 任意除き）/ 8B（一区切り）完了。**
8B は局所 TiDi コア（8B-4/5/7）+ 入場バックストップ（8B-1）で柱①の主要レバーが揃ったため一区切りとした
（詳細は §10 の「Phase 8B 一区切り」ボックス参照）。残り（8B-2 Fission / 8B-3 LoD=ADR-0020 deferred /
8B-8 越境 TiDi / 8B-6 SLA イベント化）は独立 ADR か 8D 連動で後続。

**次の自然な前進先（いずれか）**:
- **8D 分散インフラ（物理ノード分散）** — dawn-replication / ネットワーク RaftTransport / dawn-sector-node
  （ワイヤ = postcard 再利用、`dawn-proto`/protobuf は不採用）。第1次は静的 3 ノード + LAN 平文の最小スライス（§10 の 8D 表参照）。
  8B-2（Fission）はこれと本質的に対なので、8D 着手時にまとめて設計するのが自然。
- **戦闘の深み（ADR-0016 §5）** — Warp（✅）→ Tackle（✅）→ Signature Resolution（✅ ADR-0012 の
  命中率式で実質完了）→ Orbit/Keep at Range（✅ ADR-0031）→ Local Repair（✅ ADR-0033）
  → **Logistics（遠隔修理・次）**。
  柱②④（グラインドゼロの深い戦闘 / 実損ある危険な宇宙）を厚くする方向。
  - ✅ **Warp（intra-Sector 短距離 Fold = ワープ・ADR-0022）** — 「逃がさない」の前提となる高速離脱。
    align/warping 2 フェーズ・Tick Step 2.6・W キー配線済み。これで Tackle（次）が意味を持つ。
  - ✅ **Propulsion Physics — 慣性モデル（ADR-0023）** — EVE 式指数接近による align time・
    mass + inertia_modifier パラメータ化・Afterburner 対応の StatDelta 拡張（speed_multiplier /
    mass_add）。MovementSystem を exponential approach モデルに置換済み。
    おまけ: J キー（JumpCommand）が gate 射程外のとき auto-warp-then-jump を自動実行
    （WarpComp::auto_jump フラグ + pending_auto_jumps キュー）。
  - ✅ **Tackle（Fold Disruptor・ADR-0024 実装済み）** — TackledComp / process_tackle（Step 4.5）/
    TackleApplied・TackleReleased イベント / can_propose_warp・can_propose_jump 拒否 /
    スナップショット永続化 / data/modules.toml Fold Disruptor I（id=12）配線済み。
  - ✅ **Orbit / Keep at Range（ADR-0031 実装済み・2026-06-23）** — OrbitComp/KeepAtRangeComp・
    process_orbit（Step 2.55）/ process_keep_at_range（Step 2.56）。Orbit は固定 UP 軸の
    接線リードでターゲット円周を周回（ADR-0012 のトランスバーサル速度ペナルティが回避手段に
    なる）。Keep at Range は純粋な離脱（周回なし）。半径/距離省略時は武器射程をデフォルトに
    採用。Approach/Orbit/KeepAtRange は相互排他、Warp 中は拒否。O/K キー配線済み。
  - ✅ **Local Repair（ADR-0033 実装済み・2026-06-24）** — Active Shield Booster / Armor Repairer、
    RepairSystem（Step 6.5）/ RepairApplied / client flash_repair を追加。遠隔修理 Logistics の
    共通土台として、自己修理だけを先に通した。

Phase 8 全体のタスク内訳は §10 を参照。残る戦闘の深みは **Logistics（遠隔修理）** が次。

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

## 10. Phase 8 — スケール基盤 / 持続性（ADR-0017 / ADR-0018）

> 本フェーズは 2026-06-14 の設計変更を反映して詳細化した（旧版は「方向性のみ」だった）。
> 対応 ADR は **方針確定済み・コード実装は本フェーズで行う**。
> 関連: ADR-0017（スナップショット圧縮・2層ログ）, ADR-0018（局所 TiDi）, docs/architecture/tick-model.md §8,
> docs/design/game-design.md §8, docs/reference/eve-reference.md §8–§11。

**完了基準（ADR-0018 で更新。旧「5,000 ships 上限で常に SLA」は撤回）:**

- 通常負荷では論理 Tick が一定で SLA（≤32ms）を満たす。
- 空間的に分離可能な負荷は動的分割で**劣化ゼロ**に捌ける。
- 分割不能な単一密戦闘がノード容量を超えたら、当該 Sector **局所**の TiDi で graceful に劣化し、
  dilation 係数を SLA メトリクスに記録、負荷減で 1.0 に**自動回復**する（イベントの並べ替え・欠落なし）。
- 入場制限は**最終バックストップ**としてのみ発動する。
- **創世記 replay なし**で failover / 再起動できる（最新スナップショット + 末尾 replay）。

### 8A. イベントログの持続性（ADR-0017）— 最優先（正しさ / 運用性）

| # | タスク | クレート | 状態 |
|---|---|---|---|
| 1 | **スナップショット検証テスト**: ① round-trip（snapshot→restore→snapshot バイト一致）② snapshot + 末尾 Tick == live（cap/hull 含む） | dawn-simulation | ✅ take_snapshot 正準ソート + 2テスト |
| 2 | ホットログのセグメント化（base_index ヘッダ）+ `compact()` 機構 | dawn-event-store | ✅ FileEventStore.compact + 4テスト |
| 3 | コールドアーカイブ書き出し（append-only）+ 原子的 swap（write-new-then-swap） | dawn-event-store | ✅ compact() 内で実装（header に base を埋め rename 一発で原子的） |
| 4 | failover / 再起動が創世記 replay を要求しないテスト（ADR-0014 連携） | dawn-simulation | ✅ 圧縮後 reopen + restore テスト |
| 5 | snapshot.rs のドキュメントコメントを改訂後 INV-002 に更新 | dawn-simulation | ✅（228f244） |
| 6 | event-catalog.md / architecture.md に2層ログを反映 | docs | ✅ §5-C 復旧モデル + §2 永続化モデル追記 |
| 7 | 圧縮の自動トリガ（ノードのスナップショット周期 → `compact()` 呼び出しのオーケストレーション） | dawn-simulation | ✅ `checkpoint()` + `CheckpointScheduler`（checkpoint.rs）+ Phase 3 デモ配線 + 3テスト |

### 8B. 負荷制御 / Anti-TiDi（ADR-0018 + 既存方針）

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | Sector Population Cap（**最終バックストップに格下げ**） | game-design.md §8 | ✅ 生 `ship_count()` ベース: `at_population_cap()` = `ship_count() >= population_cap`。TiDi 予算と同じ単位（生カウント）。`--pop-cap N` で Sector 毎に可変、両 serve ループで新規入場を拒否・3 テスト。当初の「アクティブ船除外（実効人口）」案は撤回 — INV-MOVE により等速船はイベントを出さず「無イベント = idle」が不成立（放置船を安くするのは数の除外でなく LoD=8B-3 の忠実度低下で表現する） |
| 2 | Dynamic Sector Fission（分離可能負荷の第1手） | tick-model.md §8 | ⬜ |
| 3 | Simulation LoD（忠実度の階層化・更新間引き） | game-design.md §8 層1 / ADR-0020 | ⏸️ **deferred**（ADR-0020）。設計は完了（近似ゼロの 2 段階・交差閉包）だが、着手前のコストモデルで計算メリットが未実証と判明。サーバ計算は O(n²) でなく小定数の O(n)（ADR-0019）で、LoD が削るのは c·(n−k) のみ。再開は go/no-go スパイク（idle 反復が Tick 予算の有意割合か）次第 |
| 4 | 局所 TiDi: dilation = 実時間ペーシングのみ・論理 Tick の処理内容は不変（テスト） | INV-005 と無関係 | ✅ `dilation.rs::DilationController`（判定は論理コスト=ship_count、物理時刻不使用・決定的）。単一 `--serve` ループに実配線（sleep のみ伸ばす） |
| 5 | dilation が当該 Sector 局所であること（隣接へ伝播しない）のテスト | INV-TiDi (a) | ✅ コントローラは状態共有なし・per-Sector（`dilation_in_one_sector_does_not_affect_another`）。クラスタ（多 Sector lockstep）への per-Sector ペーシングは独立ループ化（8B-2 連動）が必要・未 |
| 6 | SLA イベント / メトリクス（dilation 係数・継続時間の記録） | INV-TiDi (b) 観測可能 | 🔶 `active_ticks`（継続 Tick）+ 係数・engage/recover ログ。構造化 SLA イベント化は未 |
| 7 | 負荷減での自動回復（係数 → 1.0）のテスト | INV-TiDi (d) | ✅ `auto_recovers_to_real_time_when_load_drops` |
| 8 | 差分 TiDi の越境因果ルールを実装 ADR で詰める | ADR-0018 未解決論点 | ⬜ |

> **Phase 8B 一区切り（2026-06-15）**
>
> **達成**: 過負荷対応ヒエラルキー（ADR-0018）の中核が機能する状態になった。
> - **局所 TiDi コア（8B-4/5/7）** ✅ — 決定論的に発動（論理コスト基準・非破壊・自動回復）。単一密戦闘の安全網。
> - **入場バックストップ（8B-1）** ✅ — 生カウントの最終手段。
> - **容量レバー（8C / AoI）** ✅ — 真の O(n²)（配信側）を解消し TiDi 閾値を押し上げ。
>
> これで**柱①（TiDi 閾値が桁違いに高い大規模リアルタイム戦闘 / ADR-0016）の主要レバーは単一 Sector 内で出揃った**。
> 単一密戦闘＝クライマックスは「AoI で容量↑ → それでも超えたら局所 TiDi で全員が少し遅い → 極端時のみ入場制限」で一貫して捌ける。
>
> **意図的に open のまま残す項目（柱①をブロックしない）**:
> - **8B-3 Simulation LoD** ⏸️ deferred（ADR-0020）— 計算メリット未実証。再開は go/no-go スパイク次第。
> - **8B-2 Dynamic Sector Fission** ⬜ — 要 ADR。密戦闘には効かず**空間分離可能な負荷**（複数戦線・広域経済）向け。
>   物理ノード分散（**8D**）と本質的に対であり、8D 着手時にまとめて設計するのが自然。クラスタ per-Sector ペーシング（8B-5 残り）の前提でもある。
> - **8B-8 差分 TiDi 越境** ⬜ — 別 ADR・8B-2 に依存。多 Sector の差分 dilation が前提。
> - **8B-6 構造化 SLA イベント** 🔶 — 係数・継続 Tick・engage/recover ログは実装済み。イベント化は小さな磨き込みで後回し可。
>
> **結論**: 密戦闘（柱①）の主要レバーが揃ったので Phase 8B を一区切りとする。残り（Fission / 越境 TiDi / SLA イベント化）は
> それぞれ独立 ADR・または 8D（分散インフラ）と連動して着手する。次の自然な前進先は **8D（物理ノード分散）** か
> **戦闘の深み（ADR-0016 §5: Tackle → Signature → Orbit/Keep at Range → Logistics）**。

### 8C. AoI 静的セルグリッド（ADR-0019）— TiDi 閾値を上げる本体

> 8C が効くほど 8B-4〜7（TiDi 発動）が稀になる。両者は連動する。
> ADR-0019 で確定: 真に O(n²) なのは **AoI（配信側・O(p·n)）** のみ。サーバ計算側は戦闘が
> 既知ターゲットに作用するため近傍探索負荷が実在せず、専用の exact 半径加速グリッドは**撤回**。
> 解は **静的セルグリッド + 3×3×3 隣接可視**（EVE 流バケツ化 + 不連続を 1.5 セル先へ。3D ゆえ 27 セル）。
> 単一密戦闘は空間分割では救えず TiDi（ADR-0018）に落ちる。

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | **新規 ADR 起票**（AoI 静的セルグリッド 3×3×3） | ADR-0019・人間承認済み 2026-06-15 | ✅ |
| 2 | `dawn-simulation` に静的セルグリッド（床除算 + セル→ShipId バケツ、近傍列挙・ShipId 順） | 派生・非永続（スナップショットに含めない）。`aoi.rs` + 6 テスト。3D ゆえ近傍は 3×3×3=27 セル | ✅ |
| 3 | `ws_server` `InitialState` を 3×3×3 スコープ化 + セル跨ぎで外周殻のみ Enter/Leave（churn 有界） | 帯域レバー（fb2a484 の発展） | ✅ 接続時スコープ化 + `aoi_delta` で毎 Tick Enter/Leave 配信（`AoiEnter`/`AoiLeave` 新メッセージ・両 serve ループ + client main.gd）|
| 4 | `DomainEvent` 配信フィルタ（関与 Ship が観測者の 27 セル近傍のときのみ送る） | 配信側の関心事・権威状態に触れない | ✅ `event_visible_to`（主船+副次船）で per-session フィルタ・両 serve ループ + 4 aoi テスト |
| 5 | （副次）NPC オートロック / 将来 AoE の半径内探索を同じ静的セルの 27 セル候補 + 厳密距離に載せ替え | 全走査版と同一結果テスト | ⬜ |
| 6 | p を増やしつつ AoI 有無の 1 Tick 時間・配信量を比較し閾値上昇を記録 | 容量↑の実証 | ✅ `--aoi-bench`（バイナリ内・benches 基盤未整備のため慣習に合わせた）。n=1k→20k で naive scan 770µs→315ms に対し AoI query ~16ms・speedup 3→19.5x・配信量 ~45x 削減 |

### 8D. 分散インフラ（物理ノード）

> **第1次 8D マイルストンは意図的に最小化する**（8D レビュー 2026-06-15 の結論）。
> 「巨大基盤の一括建設」ではなく「実機で検証できる薄いスライス」を先に通す:
> **静的 3 ノード config + postcard ワイヤ + ネットワーク RaftTransport + ログ配布ゴシップ（ADR-0021）+ LAN 平文
> → Pi 実機で Raft/Gossip を検証**。下記の defer 項目はトリガー付きで後続。

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | ~~dawn-proto（protobuf）~~ → **不採用**。ワイヤ = postcard+serde 再利用 + 最小の版付きフレーミング（長さ前置・種別タグ・版ハンドシェイク）を transport 層に置く | AI_DEVELOPMENT_GUIDE.md「Crate Boundaries」参照。理由: Rust↔Rust・多言語不要・スキーマ進化は event-schema-evolution.md で規律化済み。protobuf は型の二重定義のみ生む | ✅ 方針確定（不採用） |
| 2 | dawn-replication（追記ログのゴシップ配布 + アンチエントロピー + スナップショット転送） | 新規クレート・ADR-0021/0027（単一所有のため競合解決 CRDT/LWW は不要）。8D-2a: `ReplicationBus` を dawn-replication の `InMemoryReplicationBus` へ移動し、`dawn-actor` は純粋なクライアント転送境界（`ClientConnection`）に縮小済み。送信側: `OutboundLogPublisher` が append-log cursor と `LogBatch` suffix 構築を保持し、production Node は `publish_new_events` を呼ぶだけに縮小済み。8D-2b: `AntiEntropy`（gap 検出・重複/overlap 判定・`iter_from` suffix 応答）実装済み。8D-2c: `TcpReplicationTransport`（4-byte length prefix + postcard / LAN plaintext）実装済み。8D-2d: `SnapshotTransfer`（`Serialize+DeserializeOwned` ジェネリック・u32 LE length prefix / 256 MiB cap）実装済み（2 テスト）。消費側: `ReplicaSet`（peer セクターごとに gap 検出・冪等・順序保持で複製ログを保持。ライブ world 適用と failover は別機能）実装済み（M-5・6 テスト） | ✅ |
| 3 | ネットワーク `RaftTransport` 実装（`InProcessTransport` の差し替え。静的 config のピア表） | trait は既存（transport.rs）。TLS 可能な選択（TCP+rustls / QUIC）にし後付けを塞がない。`TcpRaftTransport`（4-byte LE + postcard / LAN plaintext / per-peer 自動再接続 / accept ループ）実装済み（dawn-consensus/src/tcp_transport.rs・8D-3） | ✅ |
| 4 | dawn-sector-node（本番実行バイナリ・上記 transport + ゴシップの配線・静的 config 起動） | 新規クレート。`TcpRaftTransport` + `TcpReplicationTransport` を TOML 静的 config で配線。3 プロセスで 3 セクタクラスタ（ws/:787{8,9,80} raft/:790{0,1,2} repl/:791{0,1,2}）。プレイヤー Jump 時は `Redirect` JSON でクライアントを宛先 WS へ誘導し、`player_id` / `ship_id` 付き Hello で同じ player ship を resume（2026-06-29） | ✅ |
| 5 | （任意・推奨）Raspberry Pi クラスタ実機検証 | 下記 ★ 参照 | ✅ 2026-07-01・3項目とも PASS（[8d5-hardware-notes.md](./8d5-hardware-notes.md) 実行ログ参照） |
| 6 | `dawn-sector-node` への永続化配線（FileEventStore + checkpoint + 起動時リカバリ） | Phase 3 で `FileEventStore`/`checkpoint()`/`CheckpointScheduler`/`restore_from` は実装・テスト済みだったが、8D-4 で新設した本番バイナリには配線されておらず、本番は `InMemoryEventStore`（再起動で全消失）のまま稼働していたことが判明。`NodeConfig` に `event_log_path`/`snapshot_path`/`cold_path`/`checkpoint_interval_ticks` を追加し、起動時にスナップショットの有無で新規/復元を分岐、tickループに `CheckpointScheduler::maybe_checkpoint` を配線。実機起動→kill→再起動で tick・log_index が継続することを確認済み | ✅ 2026-07-01 |

**defer（トリガー付き・第1次マイルストン外。2026-07-01 時点で4項目とも未発火、着手不要）:**

| 項目 | トリガー（いつ着手するか） | 現状 |
|---|---|---|
| Raft ログ圧縮 + **InstallSnapshot RPC** | Raft ログ（transit 専用で小・成長は遅い）の無限成長が問題化、または圧縮導入で base_index 前を捨て遅延 follower が AppendEntries で追えなくなったら（ADR-0017 圧縮と対の completeness 項目） | 未発火。`dawn-consensus/src/lib.rs` のスコープ注記どおり未実装のまま。8D-5 実機検証（数百隻規模・短時間）でもログ成長は問題化せず |
| メンバーシップ変更（Raft ConfChange） | ノード入替・スケール・**8B-2 Fission（動的トポロジ）** が要るとき | 未発火。8B-2 Fission は roadmap 上も `⬜`・未着手のまま（要 ADR） |
| 動的ノード発見 | 弾力クラスタにするとき（固定 3 ノードは静的 config で足りる） | 未発火。8D-4/8D-5 とも 3 ノード静的 config のまま運用・検証済み |
| TLS / 認証 | インターネット公開時（LAN の Pi 検証は平文で可）。transport を TLS 可能にしておけば後付け可 | 未発火。8D-5 の実機検証も意図的に LAN 平文のまま実施（[8d5-hardware-notes.md](./8d5-hardware-notes.md) Out of scope 参照）。インターネット公開の計画はまだない |

★ 実機検証（任意・推奨）: ネットワークトランスポート実装後、Raspberry Pi クラスタ
（Pi 4/5 推奨。Zero 2 W は aarch64 ビルド可だが 512MB RAM が制約のため数百隻規模に縮小）で
3 ノードを物理的に分離して動作確認する。目的: 実ネットワーク遅延・分断条件下での Raft / Gossip
挙動を実機で検証する（dawn の競争優位＝分散基盤の本番妥当性を確かめる / ADR-0016）。
検証項目・合否基準・自動検証スクリプトは [8d5-hardware-notes.md](./8d5-hardware-notes.md) 参照。

### 8E. Transit consensus（ADR-0017 §5 で方針決定済み）

| # | タスク | 備考 | 状態 |
|---|---|---|---|
| 1 | 単一 Raft グループを維持（実装変更なし） | マルチ Raft はメンテ不能として却下 | ✅ 方針確定 |
| 2 | バッチ提案（fleet jump = N 隻を 1 エントリに束ねる） | fleet-jump レイテンシが実測で問題化したら着手 | ⬜ 保留 |

### Phase 9 以降（方向性のみ）

```
Phase 9 : Resource + Economy Context（dawn-economy / FBD-008 撤廃により ADR で解禁）
Phase 10: Client 本格化（GDExtension 導入）
           godot-rust で Client-Side Prediction を Rust 実装
           dawn-core 型を Godot へ直接公開
           完了基準: レイテンシを隠した滑らかな操作感
```

### クライアント技術スタック（決定済み）

```
エンジン      : Godot 4
ゲームロジック: GDScript（AI が主に書く）
高性能処理    : godot-rust / GDExtension（Phase 10 以降）
サーバー通信  : WebSocket + JSON（Phase 4〜6 で継続使用）
               → gRPC への移行は Phase 9 以降で再検討（ADR-0007）
型共有        : Phase 4〜9: チャンネル / proto 変換
               → Phase 10: GDExtension で dawn-core を直接 import

→ 技術選択の根拠は ADR-0004 を参照
```

### リポジトリ構成（Phase 4 で追加）

```
dawn/                        ← 既存 Cargo Workspace（サーバー）
client/                      ← Godot 4 プロジェクト（Phase 4 で追加）
  project.godot
  scenes/
    main.tscn
    ship.tscn
  scripts/
    connection.gd            ← ClientConnection の GDScript ラッパー
    ship_controller.gd       ← Ship 表示・移動
    skybox.gd
  assets/
    models/                  ← Ship 3D モデル（glTF）
    shaders/                 ← 宇宙エフェクト
  gdextension/               ← Phase 10 以降
    Cargo.toml               ← godot-rust
    src/
      lib.rs                 ← dawn-core を import
```

### フェーズ横断の設計原則

```
ClientConnection trait を正しく定義することがネットワーク差し替えの鍵
各 Server Context は独立した Crate として追加する
上位 Context は下位 Context に依存しない（Spatial ← Navigation ← Combat …）
Anti-TiDi 優先の方針（INV-TiDi 改訂・ADR-0018: TiDi は局所的最終手段）は全フェーズで維持する
Event Sourcing の原則（INV-001〜006）は全フェーズで維持する
```

---

## 11. 廃止・変更された計画の記録

2026-06-14（Phase 8 前提の3設計判断・ADR-0016/0017/0018）と 2026-06-04
（Phase 4〜11 開発戦略の2段階変更）の詳細は **docs/process/roadmap-history.md** の
「廃止・変更された計画の記録」を参照。
