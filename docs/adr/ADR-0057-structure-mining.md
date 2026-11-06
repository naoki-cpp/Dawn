---
id      : ADR-0057
title   : Structure mining — passive extraction attached to a destructible asset
status  : proposed
date    : 2026-08-26
deciders: [human, ai-agent]
related : FBD-009 (amends — the AFK-mining clause gains a structure exception), ADR-0016 (§6 anti-grind — the pillar FBD-009 serves), ADR-0058 (already made this argument for production time), roadmap 9C (owns placement, defence, reinforcement), docs/design/game-design.md §5 (採掘は実装しない — amended)
---

# ADR-0057 — 構造物採掘

## 背景

### FBD-009 が禁じているのは何か

`docs/architecture/forbidden-changes.md` の原文はこうである。

> **AFK mining** — do not implement content where a player activates a mining
> laser and walks away.

理由として挙がっているのは 2 点。

1. 能動的な判断の瞬間が消える
2. EVE の採掘者は海賊にとっての「無力な標的」であり、採掘者自身のコンテンツに
   なっていない

**どちらも主語はプレイヤーであって、資源フローではない。** 禁じられているのは
「資源が時間で湧くこと」ではなく、**プレイヤーが何もしていない時間が発生すること**
である。同じ節が既に例外を明記している。

> Active-decision resource gathering, or resource sinks that force decisions
> via scarcity, remain open designs. Only unattended, idle-progress mining is
> banned.

`docs/design/game-design.md` §5 の説明も同じ線を引いている。曰く
「つまり**採掘している人**はゲームをしていない」。問題にされているのは人である。

### 資源の湧き口が 1 つしかない

現状 `ScrapMetal` の取得経路は撃破ドロップのみで、レートは 1 隻 = 1 個
（`node/tick.rs` `SCRAP_METAL_PER_SHIP_DESTROYED`）。ADR-0058 の open question
「T1 の希少性」が既に指摘している通り、**撃破 1 隻 = Scrap 1 = 船 1 隻**の 1:1
交換は戦闘がちょうど自己完結してしまい、経済が戦闘の外へ広がらない。

9D で Market を作ったが、流通し得るものは完成品とスクラップだけである。

### ADR-0058 が生産時間について同じ論法を提案している

ADR-0058「FBD-009 との関係」はこう述べ、未承認の設計案として記録されている。

> 時間のかかる生産はプレイヤー設置インフラ側だけになり、それを持つかどうかは
> 能動的な選択（建てる・守る）の結果である。

**「破壊可能な設置資産に紐づく時間進行は受動成長ではない」という方針が提案されて
いる。** 本 ADR はその判定を生産から採取へ広げるだけであり、新しい論法を持ち込む
ものではない。

## 決定

**採取は構造物だけが行う。プレイヤーが乗る採掘は実装しない。設置上限は天体 1 つに
つき 1 基とする。**

### 1. 禁止線を引き直す

| | 現行 FBD-009 | 本 ADR 後 |
|---|---|---|
| 船に乗った採掘（採掘レーザー Module・採掘船） | 禁止 | **禁止のまま** |
| 破壊可能な設置構造物による採取 | 禁止 | **許可** |
| 時間・課金による恒久的な能力成長 | 禁止 | 禁止のまま |

**動かさないのは「無力な標的を作らない」ことである。** 採掘レーザーは Module として
実装せず、採掘船という船種も作らない。FBD-009 の核心はそこにあり、本 ADR はそこへ
触れない。

### 2. なぜ構造物なら成立するのか

禁止理由の 2 点が両方とも消える。

- **プレイヤーのセッション時間が 1 秒も消費されない。** 採掘船に座る人間が存在
  しない。プレイヤーの実プレイは「どこに置くか」「守り切れるか」の判断になる。
- **無力なパイロットが存在しない。** 標的は資産であり、襲撃は艦隊戦になる。これは
  ADR-0016 の柱（player-driven territory・real loss）そのもので、避けるべきものでは
  なく**作りたいもの**である。

放置しているのは**プレイヤーではなく構造物**である。この差は言葉遊びではない。EVE の
採掘が失敗コンテンツだったのは資源が湧いたからではなく、**人間が画面の前で何もして
いなかったから**である。構造物はその人間を配置から消す。

### 3. 攻撃可能でない場所に置けてはならない

本 ADR の論拠はすべて「構造物が実際に破壊され得ること」に乗っている。**安全地帯に
置けた瞬間、これはただの AFK 収入になり、FBD-009 が禁じているものそのものへ戻る。**

`docs/design/game-design.md` は既に同種の懸念を記録している。EVE のステーションを
そのまま輸入すると「何もせず留まれる安全な場所」になりやすく、FBD-009 の反グラインド
方針と相性が悪い、と。

したがって本 ADR は 9C に要件を課す。**採取構造物を設置できるのは、他プレイヤーが
攻撃できる場所に限る。** 場所の具体的条件・防衛機構・襲撃タイマーは 9C の決定空間で
あり、本 ADR は要件だけを定める。

### 4. 採取構造物は天体 1 つにつき 1 基

**設置数の上限は枠数やクールダウンではなく、マップの地形が決める。** 惑星・衛星 1 天体
につき採取構造物は 1 基までとし、それ以上は置けない。

これは原則C「数を揃えれば自動的に勝てる設計を避ける」への回答である。上限を調整値
（1 プレイヤーあたり N 基、など）で与えると、N は際限なく議論の対象になり、しかも
**どこに置くかという判断が消える**。天体 1:1 なら、増やす唯一の方法は**他人が押さえて
いる天体を取ること**になる。収入を伸ばす行為がそのまま戦闘になる。

ADR-0058 が枠数と tick 数の調整に頼る案を「設計ではなくバランスで解く脆い形」として
却下したのと同じ判断である。

**実装の鍵は既にある。** `AnchorId` は `CelestialBodyId` と 1 対 1 で
（`dawn-core/src/navigation.rs`）、船の権威位置が `(anchor, offset)` で表現されている。
「天体に 1 基」は `CelestialBodyId` 上の一意制約そのもので、新しい識別子を発明する
必要がない。

#### 現状のサイト数は 4（足場であって設計値ではない）

`data/galaxy.toml` にある天体は 7 個（`Star` 3・`Planet` 4）で、`CelestialBodyKind` は
`Star` と `Planet` の 2 値しかない。衛星は現時点でドメインに存在しない。素直に読むと
本ルールは全銀河で採取構造物 4 基を意味する。

**この 4 という数字は開発上の最小構成であり、今後マップを拡張していく**
（2026-08-26 決定）。したがってサイト数そのものは本 ADR の制約ではない。

#### サイトが希少すぎると Technetium を再現する

ADR-0058 §1 が引いている 2 例目——Technetium は、まさに**月の採掘**が地域偏在した
ために起きた。保有勢力がボトルネックを握り、相互不可侵を約するカルテル（OTEC）が
生まれ、CCP は 2013 年に価値を分散させて解体した。

**サイトが少数なら、それを押さえた集団は定義上の独占である。** ADR-0058 §1 が課した
「支払える者なら誰でも入手できなければならない」という要件と正面から衝突する。

マップ拡張はこの解決策で正しいが、**「増やす」という意図だけでは要件にならない。**
いくつになれば足りるのかが言えないと、誰も判定できない。そこで数ではなく性質として
課す。

> **サイト密度要件**: 採取サイトの総数は、**単一勢力が全サイトを保持し切れない**
> 水準でなければならない。実装時点および以後のマップ拡張のたびに、この性質が
> 保たれているかを見る。

満たす手段はマップ側に限らない。`CelestialBodyKind` へ `Moon` を足す（EVE の
moon mining に対応する）、星系を増やす、のいずれでもよい。**採取産物を T2 の
ボトルネックにしない**という逃げ道も残っており、そちらを採れば独占が起きても
T2 の入手経路は塞がらない。

**いずれも採らずに少数サイトのまま実装した場合に起きることは、EVE が実測済みである。**

### 5. 資源経済の入り口は戦闘のまま

ADR-0058 §3 により構造物自体は T1、すなわち `ScrapMetal` から作る。そして
`ScrapMetal` の唯一の湧き口は撃破である。

```
撃破 ──> ScrapMetal ──> 採取構造物 ──> (時間) ──> 資源
                             ^                      |
                             +──── 襲撃 / 防衛 ─────+
```

**最初の構造物は必ず戦闘で買う。** 資源が資源だけで増える閉路は存在せず、North Star
（戦闘が主コンテンツ）と整合する。ブートストラップも壊れない — 全損したプレイヤーは
ADR-0058 §2 の NPC ステーションから T1 で復帰できる。

### 6. 何を掘るかは本 ADR では決めない

資源の種類、レート、ADR-0058 のレシピ（中間財を置くか）との接続は別 ADR。本 ADR が
決めるのは**禁止線の位置だけ**である。

## FBD-009 改訂文（承認後に適用する）

`docs/architecture/forbidden-changes.md` の **AFK mining** 節を次へ差し替える。

> **AFK mining** — do not implement content where a player activates a mining
> laser and walks away.
>
> Reason: it removes the moment of deliberate player decision-making. In EVE,
> miners function as "helpless targets" for pirates; the miner themself isn't
> really playing. **The ban is about the player, not the resource flow.**
>
> Therefore no mining-laser module and no mining hull may exist: a player must
> never be the thing sitting still.
>
> Extraction by a **destructible, player-placed structure** is permitted
> (ADR-0057). No player session is consumed, and the target of a raid is an
> asset rather than a helpless pilot. This permission is conditional on the
> structure being placeable only where other players can attack it; a
> safe-space extractor is the banned thing under another name.

## 検討して落とした案

**FBD-009 をそのまま維持する。** 資源の湧き口が撃破ドロップ 1 つに固定されたままに
なる。ADR-0058 の open question「T1 の希少性」が解けず、Market に流通するものも
増えない。禁止条文の文言は「プレイヤーが放置すること」を指しており、構造物採取まで
禁じる読みは条文が述べている理由から出てこない。

**船に乗った採掘を解禁する。** FBD-009 の核心（無力な標的を作らない）へ正面から反
する。構造物案の価値は、まさにそれを避けながら資源フローだけを得る点にある。

**NPC ステーションが資源を売る。** 湧き口が安全になり、territory の価値が消える。
ADR-0058 §2 が NPC ステーションを「恒久的な床」と定めたのは復帰保証のためであって、
経済の主要供給源にするためではない。

**採取をアクティブ操作にする（スキャニング等）。** `docs/design/game-design.md` §5 が
既に代替案として挙げているもの。これは却下ではなく**別の話**である。本 ADR は構造物
採取を入れるが、能動的な探索コンテンツを排除しない。両立する。

## 実装チェックリスト

- [ ] 本 ADR が human 承認を得る（FBD-009 は forbidden change であり、
      「propose an ADR revision and get human approval first」が前提条件）。
- [ ] `docs/architecture/forbidden-changes.md` の AFK mining 節を上記改訂文へ
      差し替える。
- [ ] `docs/design/game-design.md` §5「採掘は実装しない（非交渉的）」を改訂し、
      非交渉的だったのが**船に乗った放置採掘**であって採取全般ではなかった旨を
      明記する。§5「その他スコープ外」の `× 放置採掘・製造` も同様。
- [ ] `docs/architecture/architecture.md` の
      `Resource <- idle/passive mining is banned by FBD-009; contested-only` を
      更新する。
- [x] ADR-0016 と ADR-0058 の `related` に本 ADR を追記する。
- [ ] 採掘レーザーが Module として、採掘船が ShipType として存在しないことを
      維持する。
- [ ] 採取構造物が攻撃可能な場所にしか設置できないことをテストが示す（9C 後）。
- [ ] 1 天体に 2 基目を設置する要求が拒否されることをテストが示す
      （INV-006: コマンドは拒否され得る）。
- [ ] 実装時点のサイト数が §4 の**サイト密度要件**を満たす（単一勢力が全サイトを
      保持し切れない水準）。マップ拡張のたびに再確認する。

## Open questions

- **~~数の線形スケール~~** — §4 で決着（天体 1:1）。上限はマップ地形が与える。
- **~~サイト数~~** — 現状の惑星 4 個は最小構成で、今後マップを拡張する
  （2026-08-26 決定）。残る宿題は数ではなく §4 の**サイト密度要件**を満たし続ける
  ことで、拡張のたびに単一勢力が全保持できない水準かを見る。`Moon` を
  `CelestialBodyKind` へ足すかは、その手段として別途判断する。
- **オフライン中の襲撃。**（**保留** — 2026-08-26 時点で意図的に後回し。）守れない
  時間帯に落とされるだけなら、構造物は時差の弱い側を一方的に罰する装置になる。EVE の
  reinforcement timer 相当が要るか。9C の防衛機構と併せて別途決める。
- **稼働に人が要るか。** 完全放置か、燃料補給のような定期的な能動操作を挟むか。
  後者は FBD-009 の精神に近い一方、「ルーティン作業」になれば
  `docs/design/game-design.md` §5 が別途嫌っているものになる。
- **何を掘るか。** `ScrapMetal` を掘るのか新資源を足すのか。ADR-0058 のレシピと
  中間財の議論に直結する。
