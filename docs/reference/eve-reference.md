# EVE Online / EVE Frontier リファレンス

dawn プロジェクトの設計・バランス調整の参照元として、EVE Online（既存作）と
EVE Frontier（CCP の新作）の公開情報を収集・整理したもの。

> このファイルは「外部ゲームの事実の記録」であり仕様ではない。dawn の挙動を変える根拠に
> 使う場合は ADR を起票すること（CLAUDE.md §1/§7）。数式・定数は原典に当たって検算する。
> 最終収集日: 2026-06-13。

---

## 1. 命中判定（Turret Hit Chance）— dawn の Combat System の原型

EVE の砲塔命中率は **2 つの独立成分の積**で、それぞれ `0.5^(term²)` の形をとる。

```
ChanceToHit = 0.5 ^ ( (Angular × 40000 / (Tracking × Signature))²
                     + (max(0, Distance − Optimal) / Falloff)² )
```

| 記号 | 意味 | dawn 対応 |
|---|---|---|
| Angular | 角速度 [rad/s]（直線追尾なら 0） | `angular`（相対速度の接線成分 / 距離） |
| Tracking | 砲塔の追尾速度 [rad/s] | `tracking_speed_add`（StatDelta） |
| Signature | 標的のシグネチャ半径 [m]（大きいほど当たる） | 船の sig（NPC/PLAYER stats） |
| 40000 | Signature Resolution（砲のターゲットサイズ基準） | dawn は簡略化して未導入 |
| Optimal | 最適射程 [m] | `weapon_range_add` |
| Falloff | 減衰射程 [m]。Optimal+Falloff で命中率 50% | `falloff_range_add` |

**dawn の現行式**（CLAUDE.md §6 / Combat System）:
`0.5^((angular/(tracking×sig))² + (max(0,d−opt)/falloff)²)` ——
EVE 式から Signature Resolution 定数（40000）を省いた簡略版。構造は完全一致。

### ダメージ抽選（命中後）
```
RandomDamageModifier = x + 0.49        (x ≥ 0.01)
                     = 3.0             (x < 0.01)  ← Wrecking shot
```
- 通常命中: ベースの 50%〜149%
- 0.49 のオフセットにより期待値 ≈ 1.0（x が一様 [0,1)）
- x < 0.01（1%）で確定 300%（"Wrecks"）
- ログ表現: "Grazes"(50–62.5%) … "Hits" … "Wrecks"(300%)
- dawn は現状フラットダメージ。
- **✅ 通常命中の乱数（50–149%, 期待値≈1.0）は採用方針。** `x + 0.49`（x∈[0,1)）の係数で、
  毎発に小さな分散を与える。期待値が 1.0 なのでバランスを崩さず、命中の手触りが増す。
  実装は挙動変更のため要 ADR（CLAUDE.md §7）。
- **🚫 Wrecking shot（確定300%）は採用しない。** 低確率の大ダメージは結果の分散を運任せにし、
  CLAUDE.md / game-design.md の「プレイヤーの意図的判断を増やすか」という設計の問いに対して
  No（プレイヤーが制御できない揺らぎ）。よって x<0.01 の 300% 分岐は入れず、係数は
  `clamp` ではなく 50–149% の範囲に収める。

出典: [Turret mechanics – EVE University](https://wiki.eveuniversity.org/Turret_mechanics),
[Turret damage – Backstage Lore](http://wiki.eve-inspiracy.com/index.php?title=Turret_damage)

---

## 2. キャパシタ（Capacitor）— dawn の Capacitor System の原型

EVE のキャパシタ回復は**非線形**。現在の充電量に依存し、**25% 地点で回復速度が最大**。

```
ピーク回復 dC/dt = 2.5 × Cmax / RechargeTime   （C = Cmax/4 のとき）
平均回復         = Cmax / RechargeTime
→ ピークは平均の 2.5 倍。0% 近傍と 100% 近傍では回復が遅い（釣鐘状）。
```

- スキル: Capacitor Management +5%/Lv（容量）, Capacitor Systems Operation −5%/Lv（回復時間）
- 「キャパシタ安定（cap-stable）」= 消費 ≤ ある充電率での回復、となる平衡点が存在する状態

**dawn との差**: dawn は**線形回復**（`recharge_per_tick` を毎 Tick 加算）+ サイクル制消費
（`cap_cost_per_cycle`、不足で強制 OFF）。EVE の非線形回復は未導入。意図的な簡略化で、
「使い続けると枯れる/枯れない」の平衡だけを `data/modules.toml` のコメントで管理している
（例: Small Railgun 60/10t は recharge 100/10t に対し +40/cycle で持続可能）。

出典: [Capacitor – EVE University](https://wiki.eveuniversity.org/Capacitor),
[Capacitor recharge talk](https://wiki.eveuniversity.org/Talk:Capacitor_recharge_rate)

---

## 3. Single Shard アーキテクチャ — dawn の研究テーマの原型

- **単一シャード**: 全プレイヤーが 1 つの宇宙に存在。クラスタ全体で約 4 万同接、1 日 2.5 億トランザクション。
- **3 層構成**:
  - Proxy Blades — クライアント接続の受け口・ルーティング
  - SOL Blades — 計算の本体。90〜100 枚 ×2 ノード。**ノード = 1 CPU コアに固定された重い1プロセス**
  - Database Cluster — SQL Server + SSD（高 IOPS で永続化）
- **ソーラーシステム → ノードへのマッピング**で負荷分散（シャード分割ではない）。
  Jita のような過密システムは専用ブレードを割り当て。
- **Stackless Python**: tasklet（軽量マイクロスレッド）で大量同時接続を捌く。
  ただし1ノードは単一スレッド = 1コアしか使えない（→ 過密時の限界）。

### Time Dilation (TiDi) — dawn が**意図的に採用しない**もの
- 大規模戦闘でサーバが過負荷になると**ゲーム内時間を最大 5% まで引き延ばす**（tick 0.1Hz まで低下）。
  負荷が捌けると 30% 程度まで回復。
- **dawn の立場**: INV-TiDi / CLAUDE.md §2 で明確に否定。Tick の論理速度は一定に保ち、
  過負荷は **Sector 入場制限（SpawnRejected）+ 動的分割**で事前対処する。TiDi は体験を損なう設計。

出典: [EVE Online Architecture – HighScalability](https://highscalability.com/eve-online-architecture/),
[Introducing TiDi](https://www.eveonline.com/news/view/introducing-time-dilation-tidi),
[Time dilation – EVE University](https://wiki.eveuniversity.org/Time_dilation),
[Stackless Python in EVE (Jónsson)](https://www.slideshare.net/Arbow/stackless-python-in-eve)

---

## 4. EVE Frontier（CCP 新作）— dawn の Phase 7.5+ ナビ/インフラの参照

ブロックチェーン基盤のサバイバル MMO。プレイヤー製スマートコントラクトで世界を拡張する。

### 4.1 ナビゲーション & 燃料（dawn の JumpGate / Transit に近い）
- **2 種の移動**:
  - **Stargate / Smart Gate ジャンプ = 燃料 0**（固定インフラ経由）← dawn の JumpGate に対応
  - **船のジャンプドライブ = 距離に比例して燃料消費**（自由移動）← dawn には未導入の概念
- **ジャンプ可能範囲**は現在の燃料量で決まり、星系の周囲に到達可能圏が光って表示される。
- 規模: **24,000+ 星系 / 236,000+ 天体**。WASM 製 3D 星図で heat-aware ルーティング。

### 4.2 正準定数・数式（EF-Map AI Facts より。要検算）
```
燃料体積       : 0.28 m³ / unit
燃料定数       : 1e-7 (10⁻⁷) / kg
移動距離       : distance = (fuel_qty × fuel_quality) / (1e-7 × ship_mass)
ジャンプ範囲   : range = (ΔT × C_eff × M_hull) / (3 × M_current)
                 ΔT = 150 − ship_temperature,  C_eff = 比熱 ×(1+適応ボーナス)
外部温度       : H(D) = 100 × (2/π) × arctan(K × 2π × √(L/L_sun) / D)
                 L_sun = 3.828e26 W,  K = 100,  温度上限 150
ジャンプ禁止   : 外部温度 ≥ 90（恒星に近すぎると不可）
```
燃料品質: D1/D2 = 10%/15%（基本）, SOF-40/EU-40 = 40%, SOF-80 = 80%, EU-90 = 90%。

> **dawn への示唆**: 「恒星熱で移動が制限される」「燃料＝移動の通貨」という制約系は、
> dawn の Sector 入場制限（アンチ TiDi）とは別軸の "意図的判断を増やす制約"。
> 採用するなら ADR で。現 dawn の Jump は燃料コスト無しなので、Frontier の固定ゲート側に近い。

### 4.3 Smart Assemblies / Smart Turret（プレイヤー製インフラ）
- **Smart Assemblies**: ストレージ・取引所・防衛構造などをプレイヤーが設置・プログラム可能
  （Solidity 等でスマートコントラクト化、dApp 連携）。
- **Smart Gate**: プレイヤー設置のスターゲート。アクセスルール設定可（public / tribe-only / toll 課金）。
- **Smart Turret**: 範囲内の**自トライブ以外**を自動攻撃。Smart Assembly に接続が必要
  （1 Assembly に最大 3 基）。"友軍/敵を識別する防衛" の最小単位。

### 4.4 社会構造
- **Tribe**（小集団）/ **Syndicate**（大連合）で影響力を拡大。

### 4.5 基盤
- ブロックチェーン: **Sui (Layer-1, Mysten Labs)**。当初 Ethereum 文脈で発表、Solidity で記述。
- エンジン **Carbon Development Platform (CDP)** をオープンソース化予定。a16z 主導で $40M 調達(2023)。

出典:
[Gameplay Features – EVE Frontier Support](https://support.evefrontier.com/hc/en-us/categories/17356348312220-Gameplay-Features),
[Smart Turret](https://support.evefrontier.com/hc/en-us/articles/20197323010076-Smart-Turret),
[Configure Smart Gate](https://docs.evefrontier.com/SmartGate/configure-smart-gate),
[EF-Map AI Facts](https://ef-map.com/ai-facts),
[What is EVE Frontier – Decrypt](https://decrypt.co/resources/what-is-eve-frontier-ccp-space-survival-game-ethereum),
[EVE Frontier on Sui – PlayToEarn](https://playtoearn.com/news/ccp-games-set-to-launch-eve-frontier-on-sui-blockchain)

---

## 5. dawn の現行実装との対応表

| EVE / Frontier 概念 | dawn の現状 | 差分・メモ |
|---|---|---|
| 砲塔命中式 | ✅ 実装（Combat System） | Signature Resolution(40000) 定数を省略 |
| 通常命中乱数(50–149%, 期待値≈1.0) | ◐ 採用方針 | 期待値1.0で手触り向上。実装は要 ADR |
| Wrecking shot(確定300%) | 🚫 不採用 | 運任せの分散は意図的判断を増やさない |
| キャパシタ非線形回復 | ❌ 線形 recharge | サイクル制消費+強制OFFのみ採用 |
| Single Shard / Sol ノード | ◐ 研究テーマ | 3ノード固定・Sector→Node マッピング |
| Time Dilation | 🚫 意図的に不採用 | INV-TiDi: 入場制限で対処 |
| Stargate ジャンプ(燃料0) | ✅ JumpGate / Transit (ADR-0009) | 燃料コスト無し（Frontier の固定ゲート相当） |
| 船ジャンプドライブ(燃料比例) | ❌ 未導入 | Frontier 固有。採用は要 ADR |
| 恒星熱による移動制限 | ❌ 未導入 | 「意図的判断を増やす制約」の候補 |
| Smart Turret(友敵識別の自動防衛) | ❌ 未導入 | Bot System が近いが設置型ではない |
| Tribe / Syndicate | ❌ スコープ外 | 社会システムは未承認 |
| Approach（半自動操船） | ✅ ADR-0015 | EVE の "Approach/Orbit/Keep at Range" の Approach 相当 |

---

## 6. コミュニティの声 — テーマ別 良/悪（実証データ）

EVE 公式フォーラム / EVE Frontier 公式 docs / EVE University Wiki の公開議論
**18,149 文書**（forums 18,094 / wiki 17 / frontier 38）を収集し、テーマ分類 + 語彙
ベース感情分析にかけた結果。収集・解析は dawn 外の独立ツール `eve-research`
（`../eve-research`、コミット済み）で実施。

> **指標の読み方**: サンプルが大きいと net(肯定−否定の総和)は全テーマ正に膨らみ無意味になる。
> 規模に頑健な対立度の指標は **否定率 = 否定 / (肯定+否定)**（高いほど論争的）。
> 感情分析は語彙ベースの近似で皮肉・専門用語を取りこぼす。**数値は傾向の目安**であって
> 設計判断の根拠そのものではない。Reddit は API 登録要件のため未収集。

### 6.1 否定率ランキング（論争度の高い順）

| 否定率 | テーマ | 文書数 | 読み取れる傾向 | dawn への含意 |
|--:|---|--:|---|---|
| **46%** | sovereignty | 701 | "グラインド化した支配"への最も明確な不満 | スコープ外で正解。グラインド設計を避ける |
| 41% | frontier_survival | 258 | ⚠️ 大半は EVE 側の誤検出（§6.3 参照） | データ薄。判断保留 |
| 40% | progression_skills | 589 | スキル制＋課金育成への不満 | スコープ外で正解（**FBD-009 裏付け**） |
| 39% | economy_market | 2539 | AFK 採掘/ratting＝"退屈なグラインド"批判 | スコープ外で正解（**FBD-009 裏付け**） |
| 38% | performance_tidi | 312 | TiDi/lag は一様に負の体験 | **INV-TiDi（不採用）を補強** |
| 37% | ui_ux_npe | 859 | 退屈な序盤が新規を AFK へ追いやる | 操船 UX を "判断のある体験"に |
| 37% | capacitor | 607 | cap 戦＝緊張ある駆け引きとして機能 | サイクル消費＋強制 OFF は妥当 |
| 36% | tank_ehp | 1724 | 小さな数値調整で艦の人気が変わる | データ駆動(TOML)バランスは妥当 |
| 36% | web3_blockchain | 97 | ⚠️ 大半は "wallet"=ISK 財布の誤検出 | データ薄。判断保留 |
| 35% | pvp_conflict | 2088 | リスクは賛否割れるが、代替の grind は更に不評 | 非対称リスク設計は方向として正しい |
| 35% | movement_nav | 1484 | 移動コスト/離脱は能動的判断にすると好評 | 移動を "判断のある時間"に |
| **34%** | combat_application | 1621 | **最も好かれる＝EVE の核**。命中の駆け引き | 命中式(tracking/sig/falloff)を踏襲して正解 |

### 6.2 横断して見えた最重要パターン: 「退屈なグラインドは大罪」

否定率と関係なく、**複数テーマを貫く最強の不満は "boring grind"** であり、しかも
プレイヤー自身が「退屈な設計が AFK/bot を生む」という因果を語っている。これは dawn の
設計の核心の問い（「その機能はプレイヤーが意図的判断を下す機会を増やすか？」）と完全に一致する。

- 「*Boring grind remains boring grind and now requires actively being bored witless by the
  boring grind.*」（sovereignty, net −6 = 全体最強の否定）
- 「*If regular anomaly sites weren't a boring grind, people wouldn't afk/bot run them.*」
  ← **退屈さが AFK/bot の原因**だと明言。
- 「*mining is also boring, but you got to at least put your lasers on a rock before going
  afk*」「*ratting isn't exactly interactive fun… a boring grind*」(economy)
- 「*Lost world… switched to a grind game… 'wait a minute, this is boring'… numbers
  plummeted.*」(pvp)

→ **dawn への含意**: FBD-009（採掘放置・スキル育成の拒否）と game-design.md の中心原則は、
20 年分のコミュニティの不満と強く整合する。"放置で進む"要素を入れないという判断は正しい。

### 6.3 テーマ別の具体的な学び（採用候補）

- **capacitor（cap 戦の非対称性）**: 「*Having been on the receiving end of a good capacitor
  attack it's a miserable place to be — but from an attacker's point of view it's a slightly
  long game: it can take a few cycles to cap out the target, and if they're using a weapon
  that doesn't need capacitor then you've still got to handle the incoming.*」
  → cap 枯渇は**数サイクルかけて効く可逆的・非対称な駆け引き**。"cap 非依存武器が cap 戦の
  カウンターになる"という層構造が深みを生む。dawn の cap-as-tactical-resource を支持。
- **combat_application（適用の賭け）**: 「*i love beam lasers, they are great vs active tank
  because of the alpha (if for a miracle of god you can make them apply)*」
  → 「高アルファだが当てにくい」という**適用のトレードオフ**こそが楽しさ。命中式に
  tracking/sig/falloff を残す根拠（combat は最も愛される＝最も低い否定率34%）。
- **movement_nav（能動的離脱）**: 「*converting nullification and warp core stabilizers to
  active modules with a substantial cooldown will add depth*」「*Look at that awful align
  time.*」→ **受動的な安全をクールダウン付きの能動的判断に変えると深みが増す**。移動コスト
  （align/速度）は立派な戦術軸。dawn の Approach/Transit を "判断のある時間"にする方向を支持。
- **tank_ehp（反復バランス）**: 「*Then they gave it +50 hull, people still hated it, finally
  they tweaked it with extra cpu and grid and it became a ship some people wanted to fly.*」
  → 小さな数値差をプレイヤーは敏感に感じ取る。dawn の **TOML データ駆動バランス**（リビルド
  不要の反復調整）は正しいアプローチ。
- **pvp_conflict（非対称リスク）**: 「*grant miners who are NOT afk a good chance for survival,
  rewarding their active gameplay (gankers can ship-scan to decide whether to commit).*」
  → **能動プレイを報い、攻撃側に情報＋判断を与える非対称設計**は好まれる。dawn の Bot/捕食
  設計の参考。

### 6.4 データ品質の注意（過大評価の訂正）

- **frontier_survival(41%) と web3_blockchain(36%) の否定は信頼できない。** EVE 側
  フォーラムの**キーワード誤検出**が大半: "fuel"=ロールプレイ/"nightmare fuel"、
  "wallet"=ISK 財布。EVE Frontier の community データは公式ヘルプ 38 件しか取れておらず
  （全て肯定的な公式文）、**燃料/恒星熱サバイバルの賛否を語る一次データは実質ゼロ**。
- 前回の小サンプル(546件)で「燃料制約は好評(+6)」と読んだのは**過大評価**だった。正しくは
  「Frontier の survival 設計の良し悪しは、現状の収集データでは判定できない」。採用検討時は
  Reddit r/evefrontier などの community ソースを別途集める必要がある。

> **総括**: dawn の既存方針（**TiDi 不採用 / 採掘・スキル育成・課金成長を入れない /
> 戦闘は適用と cap の駆け引きを核にする / 移動とバランスは能動的判断とデータ駆動で扱う**）は、
> 18k 文書のコミュニティ不満傾向と強く整合する。最大の教訓は「退屈なグラインドを作らない」。
> 一方 Frontier 固有要素（燃料/恒星熱/Web3）の賛否は**データ不足で判定保留**。
> 数値・引用の原典は `eve-research/reports/design-good-bad.md` を参照。

---

## 7. さらに当たるべき一次資料

- EVE University Wiki（最も信頼できる挙動の出典）: https://wiki.eveuniversity.org/
- EVE Frontier 公式 Docs/Support: https://docs.evefrontier.com/ , https://support.evefrontier.com/
- EF-Map（星図・ルート・定数）: https://ef-map.com/
- CCP 公式 dev blog / news: https://www.eveonline.com/news
- Stackless Python in EVE（アーキテクチャ講演スライド）

### 7.1 操船 AI: Orbit / Keep at Range / Approach の力学（dawn Approach の発展）

EVE の3つの自動操船コマンドは「速度ベクトルをどう作るか」が本質的に異なり、それが
**被弾しやすさ（被 tracking）に直結**する。dawn は現在 Approach のみ実装（ADR-0015）。

| コマンド | 速度の作り方 | transversal/angular | 被弾 | 用途 |
|---|---|---|---|---|
| **Approach** | 対象へ直進、radial 最大 | transversal **最小** | **当てられやすい** | 距離を詰める。射程内に入りたいだけの時 |
| **Keep at Range** | 指定距離を保つ直線上で前後 | radial/transversal/angular **すべて最小** | 当てられやすい | optimal を維持したい時 |
| **Orbit** | 対象を中心に円運動 | transversal/angular **最大**、radial 最小 | **当てられにくい** | 射程を保ちつつ被弾を減らす（防御の主役） |
| 手動/spiral-in | 画面端寄りを連打して螺旋接近 | transversal を保ちつつ接近 | 当てられにくい | 最高の立ち回り（PvP で多用） |

> **重要な含意**: dawn の **Approach は「楽だが最も無防備」**な選択肢（transversal 0 = 相手の
> 命中式 §1 で `angular≈0` となり当てられ放題）。combat で最も愛されるのは「適用の駆け引き」
> （§6.3）なので、**Orbit / Keep at Range を足すと、操船そのものが戦術判断になる**。
> 特に Orbit は「射程は保つが当たりにくい」を半自動で実現し、ADR-0015 の自然な拡張になる。
> dawn の命中式は既に angular を入力に持つため、Orbit を入れれば**追加の戦闘ロジックなしで
> 「回り込むと当たりにくい」が創発する**。採用するなら ApproachTarget と同様に
> OrbitCommand/KeepAtRangeCommand を足す ADR を起票（挙動変更・要承認）。

出典: [Manual piloting](https://wiki.eveuniversity.org/Manual_piloting),
[Advanced piloting techniques](https://wiki.eveuniversity.org/Advanced_piloting_techniques)

### 7.2 命中式の精緻化: Signature Resolution × Signature Radius（dawn Combat の発展）

EVE の実効 tracking は、**砲の Signature Resolution と標的の Signature Radius の比**で
スケールする。dawn の現行式（§1）はこの相互作用を省いた簡略版。

```
実効 tracking = tracking × (target_signature_radius / gun_signature_resolution)
  gun_signature_resolution: Small≈40m / Medium≈125m / Large≈400m（砲の「狙えるサイズ」）
  → 標的が砲の解像度より大きい = 当てやすい / 小さい = 当てにくい
  → Target Painter で標的 sig を +30% すると tracking +30% と等価
  → angular == tracking かつ gun_res == target_sig でも命中率は 50%（確定ではない）
```

> **重要な含意**: これを入れると **「大型砲は小型高速艦に当たらない / 小型砲は大型艦に当てやすい」
> が数値から創発**する。dawn は現在 1 つの sig しか持たないが、**武器に
> `signature_resolution`、艦に `signature_radius` を持たせる**と、武器クラスの差別化
> （Small/Medium/Heavy Railgun の使い分け）が `data/modules.toml` の数値だけで表現でき、
> §6.3 の「適用の駆け引き」を強化できる。命中式は
> `0.5^((angular/(tracking×(target_sig/gun_res)))² + …)` に拡張する形。
> これは挙動変更なので ADR 必須（§7・命中式の改訂）。Target Painter 系モジュール（sig 拡大）
> の追加余地も開く。

出典: [Turret mechanics](https://wiki.eveuniversity.org/Turret_mechanics),
[Tracking Speed and Signature Resolution（Total NewbS Guide）](https://evenewplayer.wordpress.com/total-newbs-guide-to-eve-online/military-tutorial/tracking-speed/),
[The Altruist: The Truth About Signature Resolution](http://www.evealtruist.com/2011/12/truth-about-signature-resolution.html)

### 7.3 Frontier の権限モデル: Smart Assembly（将来のインフラ系 ADR の参考）

Frontier のプレイヤー製インフラは **Smart Assembly（Sui 上のプログラム可能オブジェクト）**で、
アクセス制御を**静的 ACL ではなく述語関数**として表現する。

- **canJump(characterId, sourceGateId, destGateId) → bool**: ゲート通過可否を返す関数。
  Tribe/Syndicate 所属チェック、public/tribe-only/toll 課金などをこの関数内で判定。
- **Smart Turret**: 範囲内の**自トライブ以外**を自動攻撃。Assembly に接続必須・最大3基。
- **Smart Storage**: 中身は所有者のもの。明示的な権限付与/移譲がない限り他者はアクセス不可。

> **重要な含意（Web3 を捨てて良い点だけ取る）**: 価値ある核は**ブロックチェーンではなく
> 「アクセス制御 = (actor, resource) を取る純粋な述語関数」という設計パターン**。
> dawn は非 Web3（§6 で Web3 摩擦は反面教師と確認済み）だが、将来インフラ（プレイヤー設置の
> ゲート/防衛/ストレージ）を入れるなら、**Tick パイプライン内で決定論的に評価される
> `can_use(actor, structure) -> bool` の述語**として実装すれば、スマートコントラクトも gas も
> wallet も無しに同じ表現力が得られる。これは INV（決定論・論理 Tick）とも整合する。
> Smart Turret の「友敵識別の自動防衛」は dawn の Bot System の設置型版として ADR 候補。

出典: [Smart Assemblies](https://docs.evefrontier.com/SmartAssemblies),
[Configure Smart Gate](https://docs.evefrontier.com/SmartGate/configure-smart-gate),
[Smart Turret](https://support.evefrontier.com/hc/en-us/articles/20197323010076-Smart-Turret),
[Smart Storage](https://support.evefrontier.com/hc/en-us/articles/20197380495772-Smart-Storage)

### 7.4 深掘り候補の分析（戦闘の幅・リスク設計・資源ループ）

#### 7.4.1 Tackle（Warp Disruptor / Scrambler）= 「逃がさない」非対称ツール【最優先】

EVE で**戦闘が成立する根本理由**。これが無いと誰も捕まらず全員 warp で逃げて戦いが起きない。

- **Disruptor（point）**: 長射程・warp のみ阻害。
- **Scrambler（scram）**: 短射程・阻害強度が高く、**MWD/MJD（高速移動・緊急ジャンプ）も無効化**。
- 機構: アクティブな tackle の **warp disruption strength 合計 > 標的の warp core strength** で拘束成立。

> **dawn への含意（リスク設計の核・§6 の pvp 知見と直結）**: dawn の離脱は warp ではなく
> **Jump/Transit（ADR-0009/0014）**。その類推は「**標的の Jump/Transit 開始を一定条件下で
> 拒否するモジュール/効果**」。現状 dawn の船は自由に Jump 退避できるため、捕食/被食が
> "睨み合い"にならない。Tackle 相当（拘束中は Transit コマンドを **Validation 段階で拒否**）を
> 入れると、コミットの必要な実戦闘が生まれる。**Command Validation フロー（§4 の[2]）に
> 自然に乗る**（新イベント不要、TransitCommand 拒否理由を1つ増やすだけ）。強度 vs core strength
> の合算は cap/lock と相性が良い。→ リスク設計 ADR の最有力候補。

出典: [Tackling](https://wiki.eveuniversity.org/Tackling),
[Warp Scrambling and Warp Disruption（公式）](https://support.eveonline.com/hc/en-us/articles/115004925705-Warp-Scrambling-and-Warp-Disruption)

#### 7.4.2 Logistics（遠隔修理）= 「rep を割るか」の集団戦の駆け引き

味方を遠隔修理する支援ロール。集団戦の中心力学を作る。

- **Shield rep は alpha（即時回復量）が大きい / Armor rep は滑らかだが cap 重い**。
  Armor 修理艦は cap 余裕があり neut/nos に強い、という非対称トレードオフ。
- 「**break the rep**」: 修理量を超えるバースト火力で押し切れるか否かが勝敗を分ける。
- 複数 logi の重ねがけには**逓減（diminishing returns）**があり無限スタックを防ぐ。
- カウンター: logi の cap を neut する / alpha で一気に割る / logi を拘束する。

> **dawn への含意（戦闘の幅）**: dawn は既に 3層HP・cap・lock を持つので、**RemoteRepair
> モジュール種を足せば支援ロールと「rep を割る判断」が生まれる**。逓減カーブ込みで設計すれば
> バランスも取れる。ただしこれは**協力前提の多人数機構**で、優先度は Tackle より下
> （まず捕まえられないと集団戦が起きない）。→ Combat 拡張 ADR の候補。

出典: [Logistics](https://wiki.eveuniversity.org/Logistics),
[Remote Repair Balance Changes（Legion）](https://nosygamer.blogspot.com/2025/05/eve-online-legion-patch-notes-remote.html)

#### 7.4.3 Frontier の Crude/Rift 採取ループ = 「資源の希少性が判断を生む」設計

Frontier の燃料経済。**Lens で Rift から Crude を採取 → Catalyst で Fuel に精製 → 船/拠点が
Fuel を消費**、という循環。

- 採取の前提が能動的: NPC を倒して素材→売却→Lens 購入→Extractor 建造→**使える Rift を探索**
  →採取、と複数の能動ステップを要する（EVE の「レーザー当てて放置」とは異なる導線）。
- **Fuel は消費財**: 活動が激しいほど速く減る。「プレイヤーベースが活発なほど Fuel 消費が増え
  希少化」→ Tribe/Syndicate の勢力争いの基盤になる（ネットワーク効果）。

> **dawn への含意（FBD-009 の精緻な読み）**: 価値ある核は **「消費される希少資源が、探索・
> 防衛・経済の判断と対立を生む」**部分であって、**採取の動作そのものではない**。dawn は
> FBD-009 で「採掘放置」を禁じるが、それと矛盾せず取り込めるのは "資源を**消費シンク**にして
> 希少性で判断を強制する" 側。逆に「Rift にレーザーを当てて待つ」採取動作は dawn が拒否した
> AFK 採掘と同じ罠になりうる。**良い点（希少性→判断/対立）だけ取り、悪い点（受動採取）は
> 採らない**のが筋。判定には community データが必要（§6.4 の留保どおり、現状は公式文のみ）。

出典: [Crude Lenses, Premium Fuel and network effects（whitepaper）](https://whitepaper.evefrontier.com/economy/crude-lenses-premium-fuel-and-their-network-effects),
[Rift Mining Guide（community）](https://www.youtube.com/watch?v=JIzuoMOjQFM),
[Comprehensive Alpha Guide（EVE Frontier Wiki）](https://evefrontier.wiki/Guide/Player-Guides/Comprehensive-Alpha-New-Player-Guide)

#### 優先順位（dawn にとっての価値 × 既存システムとの近さ）

1. **Tackle（7.4.1）** — リスク設計の核。新イベント不要で Command Validation に乗る。最優先。
2. **Signature Resolution（7.2）** — 命中式の精緻化。TOML 数値だけで武器差別化が創発。
3. **Orbit/Keep at Range（7.1）** — 操船が戦術になる。ADR-0015 の自然な拡張。
4. **Logistics（7.4.2）** — 集団戦の深み。多人数前提なので Tackle の後。
5. **資源シンク（7.4.3 の良い点のみ）** — 希少性で判断を生む。受動採取は採らない。要 community データ。

> いずれも挙動変更につき、着手は ADR 起票 → 人間承認（CLAUDE.md §1/§7）。本ファイルは
> 「外部ゲームの事実と示唆の記録」であって仕様ではない。
