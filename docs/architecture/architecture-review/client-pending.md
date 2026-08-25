---
scope    : Godot クライアント（client/scripts/）の保守性・設計品質レビュー — 未完項目
audience : AI Agent / Human Developer
update   : /architecture-review で状態が変わるたびに更新
related  : docs/architecture/architecture-review/client.md（構造評価）,
           docs/architecture/architecture-review/client-completed.md（完了済みログ）
date     : 2026-08-24
---

# Architecture Review — Dawn Client（未完項目）

C-1〜C-8、C-11、C-13〜C-19は解消済み。実装詳細と完了条件は各GitHub Issueに置き、ここでは判断だけを保持する。

## C-9（保留）: `hud_manager.gd` watch帯

859行で、増分はtyped refsとpanel build/updateという同一責務。直ちに分割しない。
**再評価:** 型定義・panel構築・panel更新が独立して変化するか、回帰やtest境界の不明瞭化が起きた場合。

## C-10（#200・P2）: render scale / warp thresholdのauthority重複

`WORLD_SCALE`と`MIN_WARP_DISTANCE`がRust/Godot間で手動同期されている。
**判断:** 既存のRust/GDExtension境界またはtyped initial stateを単一authorityにする。別のconstants fileは作らない。

## R-2（保留）: `main.gd`追加分割

`main.gd`は967行だが、live state、interaction、presentationは既に分離済み。残るscene lifecycle /
node generation / network send / HUD assemblyはcomposition glueとして凝集している。
**再評価:** scene-tree構成を自動検証できるようになるか、独立した変更理由が再び混在する場合。

採らない方針:

- `main.gd`の機械的な`.tscn`分割
- raw `InputEvent`のdeep module流入
- static値を別の手動同期constants fileへ移すだけの対応
