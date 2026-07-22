---
scope    : 条件が整うまで着手しないロードマップ項目
audience : AI Agent / Human Developer
update   : トリガーが発火したとき、または保留理由が変わったとき
related  : ./README.md, ./pending.md, ../roadmap.md
---

# Roadmap — Deferred

ここには今すぐ着手しない項目だけを置く。条件が満たされたら、項目を
[pending.md](./pending.md)へ移し、`README.md`の優先順位も更新する。

## 3.5 トリガー待ちバックログ

| ID | タスク | 着手条件 |
|---|---|---|
| 8B-2 | Dynamic Sector Fission | `population_cap`の80%到達（現行80,000隻/Sector） |
| 8B-3 | Simulation LoD | idle反復がTick予算の有意割合になったとき |
| 8B-6 | 構造化SLAイベント | 係数・継続Tick・engage/recoverのイベント化が必要になったとき |
| 8B-8 | 差分TiDi越境 | Fission後 |
| 8C-5 | NPCオートロック連携 | 全走査版と同一結果のテスト後 |
| 8D-defer | Raftログ圧縮・InstallSnapshot等 | 現行の静的ノード構成で問題化したとき |
| 8E-2 | fleet jumpのバッチ提案 | レイテンシが実測で問題化したとき |

## Phase 9 の保留項目

| ID | タスク | 保留理由 |
|---|---|---|
| 9A-5 | 「受動採取ではない」ことのチェック項目化 | 取得経路が増えた時に再点検する。現状はロジックと既存の取得経路で判断可能 |
| 9E-2 | 受動蓄積ゼロの回帰チェックをCIへ昇格 | 自動テストで守るべき性質に成熟するまでチェック項目として運用 |

## 運用ルール

- トリガー未発火の項目は、通常のスプリント計画に混ぜない。
- トリガーが発火したら、現状を再計測してから `pending.md`へ移す。
- 保留理由が変わった場合は、項目の説明だけでなく関連ADRやarchitecture docsも確認する。
