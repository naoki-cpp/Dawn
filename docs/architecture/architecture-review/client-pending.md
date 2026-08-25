---
scope    : Godot クライアント（client/scripts/）の保守性・設計品質レビュー — 未完項目
audience : AI Agent / Human Developer
update   : /architecture-review で状態が変わるたびに更新
related  : docs/architecture/architecture-review/client.md（構造評価）,
           docs/architecture/architecture-review/client-completed.md（完了済みログ）
date     : 2026-08-25
---

# Architecture Review — Dawn Client（未完項目）

C-1〜C-8、C-10〜C-19は解消済み。C-9とC-20はtrigger付きで保留する。
実装詳細と完了条件は各GitHub Issueに置き、ここでは判断だけを保持する。

## C-9（保留）: `hud_manager.gd` watch帯

859行で、増分はtyped refsとpanel build/updateという同一責務。直ちに分割しない。
**根本原因:** Godot `Control`の構築と更新が同じtyped node参照とpanel lifecycleを共有し、
独立した変更境界がまだ現れていないため。**判断: Defer。** pass-through panel wrapperは作らない。
**再評価:** 型定義・panel構築・panel更新が独立して変化するか、回帰やtest境界の不明瞭化が起きた場合。

## ~~C-10（#200）~~ 解消済み

`WorldSpace::render_scale()`と`ClientRules::min_warp_distance()`へ一本化済み。詳細は
[client-completed.md](./client-completed.md)を参照する。

## R-2（保留）: `main.gd`追加分割

`main.gd`は967行だが、live state、interaction、presentationは既に分離済み。残るscene lifecycle /
node generation / network send / HUD assemblyはcomposition glueとして凝集している。
**根本原因:** 残る処理はscene-tree構築とsession lifecycleを共有し、独立した変更境界をまだ持たないため。
**判断: Defer。** pass-through surfaceを増やさず、下記triggerが発火するまで現在のcomposition rootを維持する。
**再評価:** scene-tree構成を自動検証できるようになるか、独立した変更理由が再び混在する場合。

## C-20（保留）: `WorldPresentation`へのvisual lifecycle集積

`world_presentation.gd`は637行で、floating-origin rebase、navigation marker position/LOD、
sky/sun/fog/starfield bake、warp tunnel、player material/tactical overlayを調停する。
`NavigationMarkerRenderer`と`Starfield`のdeep moduleは既に分離され、現在のcoordinator自体は
一つのsession visual boundaryとして機能している。
**根本原因:** graphics feature追加のたびに、独立して変わるsetup/update lifecycleが同じ
`build`/`refresh` ownerへ集まるため。
**判断: Defer。** production部分が700行を超える、二つ以上のvisual familyを同時編集する回帰が続く、
またはtest fixtureがenvironmentとplayer effectの両方を常に構築するようになったら、最初に
sky/sun/fog/starfield bakeを`SpaceEnvironmentPresentation`へ抽出する。抽象的なvisual managerは作らない。

採らない方針:

- `main.gd`の機械的な`.tscn`分割
- raw `InputEvent`のdeep module流入
- static値を別の手動同期constants fileへ移すだけの対応
- 行数だけを減らすための汎用visual effect wrapper
