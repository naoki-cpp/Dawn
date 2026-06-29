# EVE Online / EVE Frontier リファレンス

dawn プロジェクトの設計・バランス調整の参照元として、EVE Online（既存作）と
EVE Frontier（CCP の新作）の公開情報を収集・整理したもの。

> このファイルは「外部ゲームの事実の記録」であり仕様ではない。dawn の挙動を変える根拠に
> 使う場合は ADR を起票すること（AI_DEVELOPMENT_GUIDE.md「Project North Star」/ docs/architecture/event-schema-evolution.md）。数式・定数は原典に当たって検算する。
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

**dawn の現行式**（docs/architecture/tick-model.md Step 6 Combat System）:
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
  実装は挙動変更のため要 ADR（docs/architecture/event-schema-evolution.md）。
- **🚫 Wrecking shot（確定300%）は採用しない。** 低確率の大ダメージは結果の分散を運任せにし、
  AI_DEVELOPMENT_GUIDE.md / game-design.md の「プレイヤーの意図的判断を増やすか」という設計の問いに対して
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

### Time Dilation (TiDi) — dawn は**境界つき局所的最終手段として採用**（ADR-0018）
- 大規模戦闘でサーバが過負荷になると**ゲーム内時間を最大 5% まで引き延ばす**（tick 0.1Hz まで低下）。
  負荷が捌けると 30% 程度まで回復。EVE は各ノード = 1 Python コアのため**早期かつ広域**に発動する。
- **dawn の立場（ADR-0018 で改訂）**: 当初は「意図的に不採用」だったが、単一密戦闘は分割不能で
  その場合の手段が入場制限のみになると「クライマックスから締め出す」＝ EVE より悪い体験になりうる（§11.1）。
  改めて、過負荷は **分割 → LoD → 局所 TiDi → 入場制限** の順で対処する。TiDi は
  **(a) 局所 (b) 観測可能 (c) 非破壊 (d) 自動回復 (e) 後置** の 5 条件つき最終手段として許可。
  差別化は「TiDi が無い」ではなく「**閾値が EVE より桁違いに高く、出ても局所・短時間・自動回復**」。

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
| Time Dilation | ◐ 局所的最終手段で採用（ADR-0018） | INV-TiDi: 分割→LoD→局所 TiDi→入場制限。閾値を桁違いに高く |
| Stargate ジャンプ(燃料0) | ✅ JumpGate / Transit (ADR-0009) | 燃料コスト無し（Frontier の固定ゲート相当） |
| 船ジャンプドライブ(燃料比例) | ❌ 未導入 | Frontier 固有。採用は要 ADR |
| 恒星熱による移動制限 | ❌ 未導入 | 「意図的判断を増やす制約」の候補 |
| Smart Turret(友敵識別の自動防衛) | ❌ 未導入 | Bot System が近いが設置型ではない |
| Tribe / Syndicate | ❌ スコープ外 | 社会システムは未承認 |
| Approach（半自動操船） | ✅ ADR-0015 | EVE の "Approach/Orbit/Keep at Range" の Approach 相当 |

---

## 6. コミュニティの声 — テーマ別 良/悪（フォーラム観測データ・傾向）

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
| 40% | progression_skills | 589 | スキル制＋課金育成への不満 | 観測はスコープ判断と整合（**FBD-009 と整合・実証ではない/11.5**） |
| 39% | economy_market | 2539 | AFK 採掘/ratting＝"退屈なグラインド"批判 | 観測はスコープ判断と整合（**FBD-009 と整合・実証ではない/11.5**） |
| 38% | performance_tidi | 312 | TiDi/lag は一様に負の体験 | **TiDi 閾値を桁違いに高く・局所/短時間に（ADR-0018）** |
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

> **総括**: dawn の既存方針（**TiDi を局所的最終手段に限定（ADR-0018）/ 採掘・スキル育成・課金成長を入れない /
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

> いずれも挙動変更につき、着手は ADR 起票 → 人間承認（AI_DEVELOPMENT_GUIDE.md / docs/architecture/event-schema-evolution.md）。本ファイルは
> 「外部ゲームの事実と示唆の記録」であって仕様ではない。

---

## 8. 分散アーキテクチャの実例 — EVE のノード/グリッド/スケール（dawn の本丸）

ADR-0016 で柱① **「TiDi の無い大規模リアルタイム戦闘」** を筆頭に据えた。これは dawn の
分散基盤そのものが売りになるという主張であり、**EVE がどこで詰まり、何で凌いでいるか**を
正確に知ることが最重要になる。ここが「EVE を超える」の技術的な主戦場。

### 8.1 EVE のスケール戦略（7 つの常套手段 + 1 つの驚き）

1. **Do Nothing** — 多くの負荷スパイクは放っておけば収まる。
2. **Run It Hot** — ノードは常時 100% CPU で回す（コスト最適）。
3. **Sharding by Solar System** — 世界をソーラーシステム単位で分割。1 ノードに複数システム。
4. **Live Node Migration** — 過負荷時、**小さな戦闘を別マシンへ移し**、大戦闘に資源を空ける
   （移動中プレイヤーは一時切断、移動後は快適に）。
5. **Supernodes / Reinforced Node** — 予告された大戦闘用に、**該当システムを専用ノードへ手動割当**
   （他プロセスと競合させず全リソースを与える）。Reserve node を確保して TiDi を回避/緩和。
6. **Throttle Expensive Operations** — セッション変更（システム遷移・艦変更・fleet 参加）は
   **最大 10 秒に 1 回**に制限。数百のスキルを再計算する高コスト処理だから。
7. **Brain-in-a-Box** — セッション変更のたびにスキル/艦性能を再計算せず、専用ノードが
   事前計算して **1 回の更新にまとめて送る**。fleet 移動時の負荷を激減。
8. **（驚きの一手）Time Dilation** — 過負荷時に**ゲーム内時間を引き延ばす**。lag/desync の
   代わりに全員一律で遅くし、整合性と公平性を保つ。← dawn は **局所・最終手段として採用**（ADR-0018）。
   EVE との差は「全 Sector 一律・早期発動」ではなく「当該 Sector 局所・高閾値・自動回復・観測可能」。

### 8.2 EVE の構造的限界（dawn が突くべき弱点）

- **1 ノード = 1 CPU コアの単一スレッド（Stackless Python）**。約 5000+ システムを ~170 ノードに
  マップし、低負荷システムは相乗り・過密システム（Jita）は単独・市場も単独。
  **個々のシステムは 1 コアの天井に当たり続ける**——20 年解けていない根本制約。
- **Static Cluster Premapper**: 起動時に各システムの負荷フィンガープリントを推定して割当。
  **コンステレーション（隣接システム群）は同一ノードに置きたがり**、最適ノードより 20% 重い
  程度なら近接性を優先する（局所性 > 完全な負荷分散）。
- **Node Death**: あるノードが落ちると、そのノード上の**複数システムが巻き添え**で停止し、
  プレイヤーが切断される。単一スレッド/単一プロセスの脆さがそのまま障害単位になる。
- **Interest 更新間隔**: 物理 + interest graph の更新に間隔があり、高速艦が「滑らかに近づく」
  のではなく**突然出現**する。Bracket（クライアントのアイコン）は大規模戦で**サーバ側負荷**を生む。
  グリッドは概ね **250km 以内**のオブジェクトを同一グリッドに保つ。

### 8.3 dawn への含意（Phase 8 / 柱① の設計指針）

| EVE の手法/限界 | dawn の立場・打ち手 |
|---|---|
| 1 ノード=1 コア単一スレッド（解けない天井） | **Rust + Sector 単位 ECS は 1 ノードで複数コアを使える**。EVE が 20 年逃れられない制約を構造的に回避——**最大の優位**。 |
| Time Dilation（過負荷を時間で吸収） | **局所的最終手段で採用（ADR-0018）**。通常は一定、分割不能な密戦闘超過時のみ当該 Sector を局所 dilate（観測可能・自動回復）。 |
| Reinforced Node を**手動・事前**割当 | dawn の主張は **動的・自動の Sector 分割 + 入場制限**。EVE が人手で凌ぐところを自動化するのが研究の核（§ tick-model §8）。 |
| Node Death でシステムごと巻き添え切断 | **Raft フェイルオーバー + Event 再生（INV-002）**で復旧可能にする。「ノード死＝世界の喪失」を「ノード死＝再選出して再生」に変える。**売りになる差別化**。 |
| Premapper の**局所性優先**（近接システムを同居） | dawn の Sector→Node 割当も**近接 Sector を同居**させると Transit（Raft）コストが下がる。動的分割の指針。 |
| Throttle / Brain-in-a-Box（セッション変更が高コスト） | dawn の Transit/Jump も**ハンドオフを軽く**保つ。派生状態（fitting stat 等）は**再計算せずスナップショットで渡す**。dawn は既に Event↔派生状態を分離しており整合。 |
| Interest 更新間隔・Bracket のサーバ負荷 | dawn の次の課題は **Area-of-Interest**。各クライアントは自分の「グリッド」だけ受け取り、更新頻度を可変にする。現状の Sector 単位フィルタ（fb2a484）を **Sector 内のグリッド単位 AoI** に細分するのが大規模戦の帯域レバー。 |

> **総括**: 「EVE を超える」の技術的な核は 3 点に集約される。
> ① **マルチコア/ノード**（Rust）で EVE の単一スレッド天井を超える、
> ② **動的・自動の Sector 分割 + LoD**で過負荷を捌き、捌けない単一密戦闘のみ局所 TiDi に逃がす（ADR-0018・INV-TiDi）、
> ③ **Raft + Event 再生**で Node Death を復旧可能にする。
> いずれも dawn の既存設計（Sector/Node・Raft・イベントソーシング）の延長線上にある。
> Phase 8（Anti-TiDi / スケール基盤）の ADR でこれらを具体化する。

出典:
[EVE Online Architecture（HighScalability）](https://highscalability.com/eve-online-architecture/),
[7 Sensible and 1 Surprising Way EVE Scales](https://highscalability.com/7-sensible-and-1-really-surprising-way-eve-online-scales-to/),
[Tranquility Tech IV（ノード/RAM 構成）](https://www.eveonline.com/news/view/tranquility-tech-iv),
[Brain in a Box（mass test）](https://www.eveonline.com/news/view/final-mass-test-for-brain-in-a-box-on-october-27-dont-miss-it),
[My node was equipped with the following…（reinforced node）](https://www.eveonline.com/news/view/my-node-was-equipped-with-the-following...),
[Grid Sizes & You](https://www.eveonline.com/news/view/grid-sizes-you),
[Building a Balanced Universe（premapper）](https://www.eveonline.com/news/view/building-a-balanced-universe)

### 8.4 座標系の精度と原点（dawn ADR-0028 の一次根拠）

CCP 公式の Map Data ドキュメントが EVE の座標系を明記している。dawn の大規模座標系
（ADR-0028）の直接の比較対象。

- **単位**: `1.0 = 1 メートル`（宇宙・星系の両座標系で統一）。
- **数値型**: **f64（double precision）**。「32-bit 浮動小数点では、恒星間（大）と惑星間（小）の
  スケールを**合成**すると精度が足りない」と明言。
- **星系ローカル原点**: 各星系が独立した座標系を持ち、**原点は恒星（恒星の座標は常に [0,0,0]、
  SDE/ESI に明示値を持たない）**。惑星は恒星からの相対位置で与えられる。
- **グローバル合成**: 惑星の宇宙座標 = 星系位置 + 惑星ローカル位置。宇宙系は左手系・星系は
  右手系のため **X 軸を符号反転**して合成する。
- **クライアント描画**: 「64-bit が使えない場面（3D 描画等）では **Floating Origin** で精度問題を
  緩和できる」と明記。

> **dawn への含意（ADR-0028 / 調査 2026-06-21）**: dawn は EVE と **(a) 1 unit = 1 m、
> (b) 恒星を原点とする星系ローカル系、(c) クライアント側 Floating Origin** で一致する。
> EVE が f64 を要るのは「**1 星系内に恒星〜外縁惑星を真の AU（1.5×10¹¹ m）で置く**」からだが、
> **dawn は星系内距離を圧縮**するためその条件が発生せず、**f32 を維持**できる（≤10⁶〜10⁷ units で
> ulp ≤ 1 m）。当初は決定論（INV-002/Raft）を根拠に **i64 固定小数点**を提案したが、調査で
> **dawn の決定論は「イベントが権威結果を運び、複製・再生は再計算せず適用」する設計で既に解決済み**
> と判明（戦闘 RNG は `thread_rng()`＝非決定論でも、結果が `DamageTaken` 等でイベント化される）。
> 再生で再計算される量は位置積分のみで、スナップショットで窓も有界。よって **i64 は Deferred**、
> 現方針は **f32 + Sector ローカル圧縮**（[ADR-0028](../adr/ADR-0028-large-world-coordinates.md)）。
> なお EVE が f64 で足りるのは「**星系ごと単一権威サーバで再生・ノード間一致が不要**」だからで、
> これは dawn の決定論要件とは別問題。本件の副産物として、ADR-0025 の「1 unit = 1,000 km」表記が
> 誤りで、戦闘データの `1 unit = 1 m` が正しかったことも確認された。

出典:
[Map Data — EVE Developer Documentation（CCP公式）](https://developers.eveonline.com/docs/guides/map-data/),
[EVE Online coordinate system — GameDev.net](https://www.gamedev.net/forums/topic/619254-eve-online-coordinate-system/),
[Double-precision floating-point format — Wikipedia](https://en.wikipedia.org/wiki/Double-precision_floating-point_format)

---

## 9. 一次技術資料（CCP 講演 / devblog / 論文）と読み筋

Fanfest / GDC の CCP 技術講演・devblog・学術論文。**dawn の Phase 8 設計の直接の根拠**になる。
最重要の発見: **CCP 自身が「単一スレッド・モノリシックなノードが限界」と公言し、外部エンジン
（Hadean）で空間分割の分散シミュレーションに賭けた**こと。dawn はその答えを最初から設計に
内蔵している。

### 9.1 CCP の一次技術資料（注釈つき）

- **EVE: Aether Wars（GDC 2019・Hadean "Aether Engine"）** 〔最重要〕
  EVE 資産 + Hadean のクラウド分散エンジンで **10,000 隻の戦闘**を狙った技術デモ。
  CCP の言葉: 「*the core of New Eden is still full-mesh nodes in a super-computing cluster,
  where each node is a **monolithic single-threaded application***」。
  → **CCP 自身が単一スレッド・モノリシックノードを限界と認め、空間分割の分散シミュレーション
  （まさに dawn の Sector 分割）に賭けた**。dawn の命題が当事者によって裏書きされている。
  違いは「EVE は後付けで外注、dawn は最初からそう作る」。

  **結果（その後・2019〜）**:
  - GDC19: **14,274 クライアント接続 / ピーク ~10,412（人間 3,852 + Bot）/ 88,988 隻撃沈 /
    1,470 万発**を 1 インスタンスで処理。Gamescom: 88 か国 4,369 人。Fanfest（Phase III）:
    PlayFab 認証で **30,000 人同時サインオン**のストレステストを通過。
  - **ただし 30Hz の tick rate を一貫して維持できなかった**（ログイン認証も初回 30 分遅延）。
    → **「正しいアーキテクチャでも、大規模で tick を一定に保つのは本当に難しい」**という
    率直な教訓。dawn の INV-TiDi（論理 Tick 一定）は容易ではなく、dawn の中核的挑戦そのもの。
  - **CCP は Aether Wars を製品化せず、Phase III 後に終了**（"research initiative"）。
    **ライブ EVE（Tranquility）には統合されず、TQ は今も単一スレッド・モノリシックノードのまま。**
    Hadean は以後 Minecraft / metaverse 方面へ。
  → 含意は二重: ①「正しい設計は実証済み（PoC は成功）」が、②「EVE は実証しても本番に載せ替え
    られなかった」。**dawn の勝ち筋は "最初から本番がその設計" であること**（後付け移行の
    巨大コストを負わない）。同時に①の tick-rate 課題は dawn が正面から解くべき本丸。
- **CarbonIO & BlueNet（ネットワーク技術 devblog）** 〔最重要〕
  「*Stackless Python can only execute as fast as your fastest CPU core*」— **GIL のせいで
  マルチコアが効かない**。大規模戦は単一コア能力を超える。そこで CarbonIO（GIL 外の
  マルチスレッド通信エンジン）と BlueNet（C++ が Python を完全バイパスしてノード間ルーティング）
  を **C++ で書いて GIL を回避**した。
  → dawn が **Rust（GIL 無し・ネイティブにマルチコア）**を選んでいることは、EVE が後から
  C++ で部分的に逃げた制約を**最初から負わない**ことを意味する。§8.3 ①の一次的裏付け。
- **Stackless Python in EVE（Kristján Valur Jónsson）** — tasklet（軽量マイクロスレッド）で
  大量同時接続を捌くモデル。dawn の Actor/Mailbox（tokio task）と発想は近いが、GIL が無い分
  dawn は真の並列を取れる。
- **Tranquility Tech IV（ハードウェア devblog）** — ~170 ノードで全システムを simulate、
  Jita 単独・The Forge 市場単独、1 マシン 13 ノード・512GB（ノード平均 ~39GB）。
  → 「過密拠点は専用ノード」という運用知見。dawn の動的 Sector 分割の現実的な目安。
- **Brain in a Box（devblog）** — セッション変更時のスキル/艦再計算が高コスト。事前計算して
  1 更新で送る。→ dawn の Transit/Jump ハンドオフは派生状態をスナップショットで渡し、
  再計算を避ける（§8.3）。

### 9.2 学術論文・実戦データ

- **"Monitoring and Analyzing Performance of Networked Virtual Environments: The Case of EVE"
  （IEEE）** — EVE を題材にした NVE 性能測定。分散プラットフォームへの移行を扱う。
- **"Avatar Mobility in Networked Virtual Environments"（arXiv:0807.2328）** —
  **3 か月・約 3 億回の移動 / 70 万アカウント**を分析し、**イベントに対応した人口スパイクを
  予測**できると示す。→ dawn の **動的 Sector 分割は「予測して事前分割」できる**という
  学術的裏付け（EVE の手動 Reinforced Node を自動化する根拠）。
- **Battle of B-R5RB / Asakai（実戦データ）** — 1 システムに最大 **2,670 人同時 / 延べ 7,548
  キャラ**。TiDi 下で 21 時間。→ dawn が「TiDi 無しでこの規模」を目標値にする際の比較基準。

### 9.3 読み筋（総括）

> EVE の 20 年の技術史は、**「単一スレッド・モノリシックなソーラーシステムノード」という
> 原罪を、TiDi / Reinforced Node / CarbonIO / BlueNet / Brain-in-a-Box / Aether Wars と
> 次々に"回避策"で凌いできた歴史**である。CCP 自身が Aether Wars で根本再設計の必要を認めた。
>
> dawn は、その回避策の積み重ねが指し示す終着点 — **GIL の無い言語（Rust）/ 空間分割
> （Sector）/ コンセンサス（Raft）/ イベントソーシング（再生可能な状態）** — を**最初から
> 設計に内蔵**している。「EVE を超える」は奇策ではなく、**EVE が後付けで目指した先を最初から
> 正しく作る**ことに等しい。Phase 8 ADR はこの §8/§9 を根拠に書く。
>
> **ただし誠実な留保**: Aether Wars は「正しい設計でも大規模で **tick rate を一定に保つのは
> 難しい**」ことも示した（30Hz を一貫維持できなかった）。dawn の INV-TiDi（論理 Tick 一定）は
> アーキテクチャを選べば自動で得られるものではなく、**dawn が正面から実証すべき本丸の難所**。
> 「アーキテクチャは正しい」と「一定 tick を実際に守れる」は別問題であり、後者こそが
> dawn の研究価値の中心になる。

出典:
[EVE: Aether Wars（tech demo）](https://www.eveonline.com/news/view/introducing-a-new-tech-demo-eve-aether-wars),
[CarbonIO and BlueNet](https://www.eveonline.com/news/view/carbonio-and-bluenet-next-level-network-technology-1),
[Stackless Python in EVE（Jónsson slides）](https://www.slideshare.net/Arbow/stackless-python-in-eve),
[Devblog: Tranquility Tech IV](https://forums.eveonline.com/t/devblog-tranquility-tech-iv/398191),
[Avatar Mobility in NVEs（arXiv:0807.2328）](https://arxiv.org/pdf/0807.2328),
[Monitoring & Analyzing Performance of NVEs: EVE（IEEE）](https://ieeexplore.ieee.org/document/1364600/),
[Battle of B-R5RB（Wikipedia）](https://en.wikipedia.org/wiki/Battle_of_B-R5RB)

---

## 10. 大規模分散シミュレーション基盤の比較（dawn の Sector 分割の設計参考）

§8/§9 は EVE 単体の話。ここでは **EVE 以外の分散シミュレーション基盤**と**空間分割の手法**を
並べ、dawn の動的 Sector 分割（Phase 8）が**どの戦略を採り / どの罠を避けるべきか**を定める。
最大の教訓は **SpatialOS（Improbable）の顛末** — 「何でも分散」は統合コストで死ぬ。

### 10.1 空間分割の手法カタログ

| 手法 | 概要 | 採用例 | dawn 適性 |
|---|---|---|---|
| 静的ゾーニング | 固定境界でゾーン分割 | EVE（system=node）/ 旧来 MMO | 現 dawn（3ノード固定）。出発点として堅い |
| Grid / Quadtree / Octree | 空間を再帰細分し負荷で分割 | **Aether Engine = octree** | 動的分割の候補。ただし粒度は Sector で十分 |
| Voronoi | 分割ノードを動かして負荷均衡 | 研究 | 過剰。Sector 粒度には重い |
| Q+Rtree 等ハイブリッド | quasi-static な物体を最適化 | 研究 | 当面不要 |

**境界の扱い**が要点: 「**grey area / mirroring**」（境界付近を両サーバが部分所有しデータをミラー）は
ハンドオフを滑らかにするが、**サーバコードを著しく複雑化し同期問題を増やす**（一次資料の指摘）。

> dawn の立場: **grey area を採らない。** INV-003（Sector 境界を越える操作は Raft 経由）を守れば、
> 境界の二重所有による同期バグを**構造的に排除**できる。代償はハンドオフのレイテンシだが、
> dawn は Transit 頻度を下げる設計（docs/architecture/design-violations.md Pattern 5）でこれを吸収する。

### 10.2 既存基盤の比較

| 基盤 | 空間分割 | 局所性 | 顛末・教訓 |
|---|---|---|---|
| **EVE / Tranquility** | 静的（system=node, premapper） | constellation 同居 | 単一スレッド天井 → TiDi。20 年解けず（§8/§9） |
| **Aether Engine（Hadean）** | octree・コア/マシン跨ぎ動的 | あり | PoC は 14k 接続成功も**製品化されず**。一定 tick が課題（§9） |
| **SpatialOS（Improbable）** | 動的・worker 分散 | locality of reference | **Worlds Adrift 閉鎖(2019)。過大なサーバオーバヘッド/ネット障害頻発/統合に既存バックエンド全書き換え/運用スキル希少/Unity にブロックされ開発中タイトル全滅**。"distributed-everything" の代償 |
| **dawn** | Sector（現 静的3ノード → 動的分割は Phase 8） | 近接 Sector 同居（§8.3） | これから。粗粒度 + Raft(transit のみ) で中庸を狙う |

### 10.3 dawn への設計示唆（Sector 分割の指針）

1. **粗粒度を保つ（最重要）。** SpatialOS の死因は「全エンティティを分散管理」した統合コスト。
   dawn は **Sector 単位の粗い分散 + Raft は transit だけ**を維持する。個々のエンティティを
   分散トランザクションに乗せない。これは既に dawn の設計（INV-003 / Raft は境界越えのみ）。
2. **動的分割は octree より "Sector 再割当" で十分。** EVE premapper の「負荷フィンガープリント
   + 局所性」+ arXiv:0807.2328 の「スパイク予測」を組み合わせ、**Sector を予測的に別ノードへ
   migrate / split** する（EVE の手動 Live Node Migration / Reinforced Node の**自動版**）。
   空間を octree で連続再分割するより、**意味境界（Sector）で切る**方が実装も整合も楽。
3. **境界ミラーリングを避ける。** §10.1 のとおり grey area は同期バグの温床。Raft ハンドオフを
   明示的な所有権移転に保つ（INV-003 / FBD-006）。
4. **始めは静的でよい。** 動的分割は Phase 8。早すぎる分散は SpatialOS の轍。まず固定 Sector で
   ゲーム（戦闘の深み）を成立させ、スケールは後段で。

> **総括（3 つの道）**: **SpatialOS は「分散を全部やる」で統合コストに殺され、EVE は「分散を避ける
> （単一スレッド）」で TiDi に縛られた。** dawn の勝ち筋はその中間 — **粗粒度の Sector 分散 +
> イベントソーシング + Raft（transit のみ）**。奇しくもこれは現行 dawn の設計そのものであり、
> §8〜§10 は「この中庸路線が正しい」ことを外部事例から裏付けている。Phase 8 はこの粒度を
> 崩さずに動的化することに集中する。

出典:
[Aether Engine（Minecraft 採用・octree/動的負荷分散）](https://www.pcgamer.com/minecraft-is-using-a-spatial-simulation-engine-to-make-larger-and-more-immersive-experiences/),
[Hadean × Minecraft（Medium）](https://medium.com/@hadeaninc/opening-up-new-possibilities-with-minecraft-45aa6a29e78),
[Worlds Adrift（SpatialOS・閉鎖 / Wikipedia）](https://en.wikipedia.org/wiki/Worlds_Adrift),
[Unity blocks Improbable's SpatialOS（MCV）](https://mcvuk.com/development-news/unity-blocks-improbables-spatial-os-all-live-and-in-development-games-affected/),
[Overcoming the Limits of Scale in Virtual Worlds（Delphi）](https://members.delphidigital.io/reports/overcoming-the-limits-of-scale-in-virtual-worlds),
[A Dynamic Load Balancing for MMO Game Server（Springer）](https://link.springer.com/chapter/10.1007/11872320_29),
[Load balancing for MMOGs（GameDev.net・境界 grey area 議論）](https://www.gamedev.net/forums/topic/433915-load-balancing-for-mmogs/)

---

## 11. 批判的検討 — dawn 設計へのリスクと反論

§6〜§10 は dawn の設計を肯定する材料に偏っていた（確証バイアス）。ここでは**逆向き**に、
dawn 自身の設計が抱える未解決問題・誇張・前提の弱さを列挙する。**「EVE を超えられるか」という
実現可能性は本プロジェクトの関心外（ビジョンは方向性）なので扱わない**。対象は
「dawn の技術設計と意思決定の土台が健全か」に限る。

### 11.1 アンチ TiDi（INV-TiDi）には分割できない本丸ケースが残る 〔重大〕

- **単一の密な戦闘は原理的に分割できない。** 全対全で相互作用する 1 つの大戦闘（B-R5RB 型）を
  ノード跨ぎにすると、毎 Tick ノード間で全状態を同期する必要が生じ、それは Raft/イベント処理の
  遅い経路そのもの。**dawn の動的 Sector 分割は、最も必要な「ザ・大規模戦」でこそ効かない。**
- dawn の答えは入場制限（SpawnRejected）= **「この戦闘に入れない」**。EVE が TiDi を選んだのは
  「目当てのコンテンツから締め出す方が残酷」と判断したから。**入場制限 vs 時間引き延ばしは
  どちらが良いか自明でなく、dawn は後者を一方的に劣ると断じている。** §6 の不満データには
  「参加できないこと」への不満も含まれる。
- ~~**未解決**~~ → **ADR-0018 で対応**。INV-TiDi を反転し、単一密戦闘では入場制限を最後に下げ、
  局所 TiDi（全員残る）を優先する劣化ヒエラルキー（分割→LoD→局所 TiDi→入場制限）を採用した。
  「締め出すより全員が少し遅い方が良い」という本批判の指摘を設計に取り込んだ。
  残る課題（差分 TiDi の越境因果）は ADR-0018 の未解決論点として明記。

### 11.2 イベントソーシングの内部矛盾（不変条件どうしが衝突する）〔重大〕

- **FBD-001（truncate 禁止）+ INV-001（append-only）+ INV-002（ログから完全再生）は、
  長寿命シャードで永続的に両立しない。** ログは無限増加し、再生は非現実的な時間になる
  （EVE の「システムロードに 19 分」が示唆）。
- 通常の解はスナップショット + ログ切り詰めだが、それは **FBD-001 と正面衝突**する。dawn は
  スナップショットを持つが、「切り捨て禁止のまま完全再生を保証する」運用方針が未定義。
- **そもそも EVE はシミュレーションをイベントソースしていない**（現在状態を DB 保持）。
  リアルタイム Tick で毎秒大量イベントを追記し続ける設計は異例で、書き込み増幅が重い。
  → ~~**要 ADR**~~ → **ADR-0017 で対応**: 2層ログ（ホット=圧縮可 / コールド=永久 append-only）を導入し、
  INV-002 を「最新の検証済みスナップショット + 末尾から再生可能」に改訂。FBD-001 は trait 上維持
  （圧縮はセグメント移送で履歴を破壊しない）。

### 11.3 Raft を Transit のホットパスに置く代償 〔中〕

- 全 Sector 越え移動が Raft コミット（リーダー経由・ネットワーク往復）を通る。**頻繁な境界越えで
  リーダーが律速**し、移動ごとに遅延が乗る。
- dawn の緩和策「Transit 頻度を下げる」（docs/architecture/design-violations.md Pattern 5）は、**アーキテクチャがゲーム設計を
  制約している**ことの裏返し。流動的移動や「境界を跨ぐ戦闘」がやりにくく、柱①
  「大規模リアルタイム戦闘」と緊張関係にある。
- 軽減はあるが（Sector を粗く / 近接同居 / バッチコミット）、**「境界をまたぐ戦闘」は本質的に苦手**
  という性質は残る。→ **ADR-0017 §5 で方針決定**: 単一 Raft グループを意図的に維持（マルチ Raft は
  メンテナンス不能として却下）。脱出路（境界ごとマルチグループ Raft）は記述のみ・事前構築しない。
  唯一の単純な備えはバッチ提案だが、実測で fleet-jump 遅延が問題化してから入れる。

### 11.4 マルチコア/Rust の優位は限定的（§8/§9 の表現を割り引く）〔訂正〕

- §8.3①・§9 で「Rust が単一スレッド天井を超える / 最大の優位」と書いたが、これは **GIL の天井に
  限った話**。EVE の真のボトルネックは**アルゴリズム的（密グリッドの全対全 O(n²)）**で、
  言語では消えない。**2,670 隻グリッドの O(n²) は Rust でも同じ。**
- マルチコアが効くのは「独立した多数の小戦闘」であって「1 つの大戦闘」ではない。
  **優位が要るその瞬間（大戦闘）に優位が薄れる。** §8/§9 の競争的トーンは割り引いて読むこと。
- 残る真の優位は「多数の中小戦闘を 1 ノードで並列に捌ける」点であり、これは依然有効だが、
  EVE を象徴する「単一巨大戦闘」を解くものではない。

### 11.5 実証データ（§6）の選択バイアス 〔中〕

- **フォーラム投稿者 ≠ 課金者/中央値プレイヤー。** 声の大きいベテランの不満であり、
  「何が売れ・何が定着するか」ではない。「グラインドは嫌われる」と言うが、**グラインド重の
  ゲームが収益上位を占める**現実と整合しない（生存者/選択バイアス）。
- **進行(progression) ≠ グラインド。** EVE の長期育成は強力な**リテンション**装置
  （サンクコスト・アイデンティティ）。FBD-009 の絶対禁止は健全な進行まで捨て、
  **「グラインドゼロ＝リテンションゼロ」**になりうる。§6 は FBD-009 を裏付けると結論したが、
  それは **既存の制約に合う引用を選んだ**面がある（分析側の確証バイアス）。
- 含意: FBD-009 は「ポリシー判断」として尊重するが、「§6 が実証した」と強弁しないこと。

### まとめ（誠実な総括）

> §8〜§10 の「中庸路線が正しい」という結論には、**11.1（密戦闘は分割不能）と 11.4（O(n²) は
> 言語で消えない）という強い反論が成立する**。より正確な総括は:
> **「dawn の分散設計は *多数の独立した中小戦闘* には効くが、*EVE を象徴する単一巨大戦闘* には
> EVE と同じ壁に当たる。アンチ TiDi の優位は限定的で、密戦闘での体験設計（締め出し or 上限）は
> 未解決。」**
> さらに 11.2（イベントソーシングの不変条件衝突）は **dawn 内部の実装可能性に関わる穴**で、
> Phase 8 より前に ADR で塞ぐ必要がある。§6 の実証は FBD-009 の十分条件ではない（11.5）。
> これらは「方向性としての EVE 超え」を否定しないが、**設計の弱点として明記しておくべき**。
