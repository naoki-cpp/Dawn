---
scope    : Godot クライアント（client/scripts/）の保守性・設計品質レビュー — 未完項目
audience : AI Agent / Human Developer
update   : /architecture-review で状態が変わるたびに更新
related  : docs/architecture/architecture-review/client.md（構造評価）,
           docs/architecture/architecture-review/client-completed.md（完了済みログ）
date     : 2026-08-11
---

# Architecture Review — Dawn Client（未完項目）

C-1〜C-8、C-11、C-13〜C-15は解消済み。実装詳細と完了条件は各GitHub Issueに置き、ここでは判断だけを保持する。

## C-9（保留）: `hud_manager.gd` watch帯

877行で、増分はtyped refsとpanel build/updateという同一責務。直ちに分割しない。
**再評価:** 型定義・panel構築・panel更新が独立して変化するか、回帰やtest境界の不明瞭化が起きた場合。

## C-10（#200・P2）: render scale / warp thresholdのauthority重複

`WORLD_SCALE`と`MIN_WARP_DISTANCE`がRust/Godot間で手動同期されている。
**判断:** 既存のRust/GDExtension境界またはtyped initial stateを単一authorityにする。別のconstants fileは作らない。

## C-16（Fix候補）: `server_message_gd.rs`のwire adapter責務混在

`crates/dawn-client-gdext/src/server_message_gd.rs`は995行で、`ServerMessageDecoder`、
wire→`ClientFact`変換、`ClientState`へのapply、`EventPresentation`からGodot targetへのcallback
dispatchを一つのfileに保持している。各処理は同じGDExtension境界に属するが、wire schema、client
state policy、Godot scene callbackという異なる変更理由で進化する。
**根本原因:** typed state境界を導入した後も、adapterの入口・変換・副作用通知を一つのmoduleへ
積み上げたため。**判断: Fix。** 同じcrate内でdecode、fact conversion、presentation dispatchを
module分割し、Godot公開APIと`ServerMessageOutcome::dispatch`のcommit後順序は維持する。
wire typeや`dawn-client-core`の責務を新crateへ移さない。

## R-2（保留）: `main.gd`追加分割

`main.gd`は1148行だが、live state、interaction、presentationは既に分離済み。残るscene lifecycle /
node generation / network send / HUD assemblyはcomposition glueとして凝集している。
**再評価:** scene-tree構成を自動検証できるようになるか、独立した変更理由が再び混在する場合。

採らない方針:

- `main.gd`の機械的な`.tscn`分割
- raw `InputEvent`のdeep module流入
- static値を別の手動同期constants fileへ移すだけの対応
