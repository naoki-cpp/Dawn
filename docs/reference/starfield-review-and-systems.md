---
scope    : Starfield Steam review themes and gameplay systems relevant to Dawn design
audience : Human Developer / AI Agent
update   : Revisit when Starfield receives major gameplay updates or Dawn's economy scope changes
related  : ../process/roadmap/README.md, ../process/roadmap/pending.md, ../design/game-design.md, ../adr/ADR-0057-structure-mining.md, ../adr/ADR-0058-production-jobs.md
---

# Starfield Reviews and Systems

調査日: 2026-11-15

この文書は Starfield の公開情報と Steam user reviews の傾向を、Dawn の設計検討用に記録する。
外部ゲームの事実の記録であり、Dawn の仕様ではない。挙動・ルール・不変条件を変更する場合は、
関連 ADR と forbidden-change を確認し、必要な承認を得ること。

Steam のレビュー全文を統計的に分類した調査ではない。Steam の集計表示と、参考になる好評・不評の
レビュー例を読んだ定性的な整理である。集計値は変動する。

## 1. 要点

Starfield から Dawn が学ぶべきなのは、惑星やアイテムの量ではなく、船・資源・拠点・移動を
プレイヤーの選択につなげる方法である。船の組み立てや活動の自由度は好評の核になっている一方、
移動の分断、反復的な探索地点、煩雑な在庫・物流、相互に影響しないシステムは、広さを深さに
結びつけられなかったという批判につながっている。

## 2. Steamレビューの傾向

### 評価のスナップショット

調査時に参照できた Steam Store 表示では、英語レビューは 86,057 件中 57% が好評で総合評価は
Mixed、直近30日分は 349 件中 62% が好評で同じく Mixed だった。これは英語レビューの表示であり、
全言語の全レビューを単純に代表する数値ではない。

### 評価されている点

- **船を自分のものとして設計すること**: 船体の形だけでなく、武器・防御・推進・貨物などを
  選び、性能の異なる船を作る自由度が、プレイヤーに長期目標と自己表現を与えている。
- **遊び方の選択肢**: クエスト、戦闘、探索、製作、拠点運営などから、自分の関心に沿って
  遊びを選べる点が好評レビューに現れる。
- **手作りの任務・遭遇**: 勢力の任務やランダムな宇宙遭遇は、同じ移動や戦闘でも状況に変化を
  作る。好評例では多様な任務と宇宙遭遇が魅力として挙げられている。

### 繰り返される不満

- **移動が分断される**: 星図やロードを何度も挟み、移動が空間を旅する体験ではなく目的地選択に
  見える、という批判がある。
- **発見の反復**: 同じ構造・配置・戦利品の地点が再登場し、探索の驚きが急速に薄れるという
  指摘がある。
- **在庫整理が遊びを妨げる**: 資源を集めること自体よりも、重量制限、保管場所、製作場所への
  持ち運び、長い取引リストの操作が面倒だというレビューがある。
- **システム間の結びつきが弱い**: 勢力の大きな任務を終えても他勢力や世界への影響が乏しく、
  その後は似た反復ミッションへ戻るという批判がある。
- **進行時間に対して意味ある判断が減る**: 多数の活動があっても、それぞれが同じ報酬や同じ
  手順へ収束すると、作業量は増えても選択の深さにならない。

参考にした Steam レビュー例として、長時間プレイヤーの不評レビューは探索動機、ロードを挟む
移動、勢力任務間の相互作用の弱さを指摘する一方、船ビルダーを最も楽しい要素の一つとして挙げて
いる。また別の好評レビュー例は、船の設計、拠点、任務の自由な順序、宇宙遭遇を長所として挙げ、
同時に在庫・取引 UI を弱点としている。個別レビューはユーザー個人の経験であり、全プレイヤーの
合意とはみなさない。

## 3. 主なゲームシステム

### 船の設計と戦闘

船はリアクター、エンジン、重力ドライブ、武器、シールド、貨物、居住区などの部品で構成され、
部品は複数の船性能に影響する。クラス、重量、接続条件、必要な部品などが制約となり、単なる
見た目の組み替えではなく性能上の選択を作る。船には乗員を割り当て、宇宙戦闘、ドッキング、
敵船への乗り込み・鹵獲も行える。

### 惑星調査と資源取得

惑星調査では生物、無機資源、地形・惑星特性などをスキャンする。資源はカッター等で直接採取
できるものと、惑星上の資源地点に拠点の採取設備を設置して得るものがある。宇宙空間の小惑星から
資源を得る経路もある。

### 拠点・物流・製作

拠点は資源地点を選んで設置し、採取機には電力を供給し、保管設備へ資源を集める。貨物リンクで
拠点間の資源輸送を行い、乗員配置が生産などに影響する。研究プロジェクトは必要素材を消費して
製作レシピを解放し、資源を装備・武器・船・拠点の改善に使う。2026年の公式更新では、複数拠点
から在庫を共有する容器や生産速度表示も追加された。

### 勢力と任務

各勢力は独自の理念と任務を持つ。ミッションボードには通常任務に加え、海賊行為、密輸、窃盗など
の異なる活動がある。ただしレビューでは、勢力の主要任務が世界全体へ与える影響や、勢力同士の
関係が弱いという不満も見られる。

### 星系内移動

2026年の Free Lanes 更新では、クルーズモードで星系内の惑星間や POI へ移動できるようになり、
宇宙空間でのランダム遭遇も引き続きゲームの要素となっている。これは完全なロードレス移動を
意味するものではないが、星系内の移動自体をプレイ可能な空間へ近づける更新である。

## 4. Dawnへの適用

### 取り入れる価値があること

1. **アイテムごとの役割を一貫させる。** 用途、供給元、消費先、保管場所、移送方法、市場での
   扱いを定める。アイテム数を増やすこと自体を進捗としない。
2. **ステーションを機能上の拠点にする。** 提供サービス、利用条件、ローカル在庫、位置による
   選択を明確にする。各ステーションが異なる意味を持つかを、サービス差が実際の行動を変えるかで
   評価する。
3. **ひとつの縦切りを最後までつなぐ。** 資源獲得、船またはステーションへの保管、Station操作、
   加工・建造・取引、そしてその結果が戦闘や移動へ戻るところまで通す。
4. **制約を判断に変える。** 貨物容量、ステーション在庫、価格、移動距離は、それだけでは
   面白さにならない。危険を取るか、何を持ち帰るか、建造と取引のどちらを選ぶかなどの判断を
   生むときにだけ有効である。
5. **コンテンツの密度を優先する。** 少数の手作り・意味のある遭遇や資源地点を、同じ POI の
   大量複製より優先する。移動にも到達感、危険、選択、発見のいずれかを伴わせる。

### そのまま移植しないこと

- Starfield の惑星数や汎用 POI 数を目標にしない。広さを作業量で埋めると、探索が反復へ変わる。
- 資源輸送や在庫管理を、プレイヤーに何度も往復させるだけの手順にしない。
- Starfield のスキルポイント成長を模倣しない。Dawn の成長・戦力原則と FBD-009 を優先する。
- 拠点の受動採取を Dawn のルールとして既成事実化しない。採取の許可範囲は FBD-009 と
  提案中の ADR-0057 に従い、承認・設計判断の後に扱う。
- 全てのステーションへ同じ機能を足すのではなく、機能差が供給・危険・市場利用の判断を変えるか
  確かめる。

## 5. Roadmap 9E-0 への接続

現在のロードマップでは、経済ループの人間プレイテストに先立ち、9E-0で Item / Station の
ゲームプレイを設計・実装する。Starfield 調査から、9E-0では以下を成果物として残すとよい。

- **Item map**: Item / 用途 / 供給元 / 消費先 / 保管・移送 / 市場可否 / そこで生まれる判断。
- **Station service matrix**: Station種別または個別 Station / Dock条件 / サービス / 在庫範囲 /
  利用者 / その場所を訪れる理由。
- **選んだ経済スライス**: 供給から利用・加工・取引までを通して遊べ、既存の Item / Station / Market
  のどこに判断が増えたか説明できる実装。
- **更新済みプレイテスト**: 実装されたループを対象に、操作の理解だけでなく、選択・リスク・他者との
  取引が実際に発生するかを観察する手順。

この文書は 9E-0 の結論を先に決めない。特にプレイヤー設置構造物、受動採取、生産Tier、
FBD-009の扱いは、各 ADR と既存の承認手順に従う。

## 6. Sources

### Steam

- [Starfield Store page and review summary](https://store.steampowered.com/app/1716740/Starfield/) — 英語レビュー集計、ゲームの公式ストア概要。
- [Starfield Steam reviews](https://steamcommunity.com/app/1716740/reviews/) — レビュー例と反復するテーマ。
- [Etherghost's Steam review](https://steamcommunity.com/profiles/76561198002901957/recommended/1716740) — 探索、移動、勢力任務間の結びつきへの批判例。
- [Blue's Steam review](https://steamcommunity.com/id/CP-Blue/recommended) — 船、拠点、任務の自由度、宇宙遭遇を評価し、UIや船ビルダー制約も挙げる例。

### Bethesda official sources

- [Ship components](https://help.bethesda.net/app/answers/detail/a_id/61674/~/elements-of-a-ship---starfield)
- [Ship building tips](https://help.bethesda.net/app/answers/detail/a_id/60959/~/ship-building---tips-%26-tricks---starfield)
- [Outpost getting started](https://help.bethesda.net/app/answers/detail/a_id/61083/~/outposts---getting-started---starfield)
- [Resource gathering](https://help.bethesda.net/app/answers/detail/a_id/61082/~/resource-gathering---tips-%26-tricks---starfield)
- [Research Lab](https://help.bethesda.net/app/answers/detail/a_id/61121)
- [Scanning and surveying](https://help.bethesda.net/app/answers/detail/a_id/60950/~/scanning/surveying---system---starfield)
- [Factions](https://help.bethesda.net/app/answers/detail/a_id/61057/~/factions---starfield)
- [Free Lanes update notes, April 7, 2026](https://bethesda.net/en-US/news/starfield-free-lanes-update-notes)
