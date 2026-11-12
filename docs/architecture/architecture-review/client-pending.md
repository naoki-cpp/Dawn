---
scope    : Godot クライアント（client/scripts/）の保守性・設計品質レビュー — 未完項目
audience : AI Agent / Human Developer
update   : /architecture-review で状態が変わるたびに更新
related  : docs/architecture/architecture-review/client.md（構造評価）,
           docs/architecture/architecture-review/client-completed.md（完了済みログ）
date     : 2026-11-12（doc-sync / architecture review: #397のradial menu policyを反映）
---

# Architecture Review — Dawn Client（未完項目）

C-1〜C-8、C-10〜C-19、C-21〜C-22は解消済み。C-9とC-20はtrigger付きで保留。
実装詳細と完了条件は各GitHub Issueに置き、ここでは判断だけを保持する。

## C-9（保留）: `hud_manager.gd` watch帯

859行で、増分はtyped refsとpanel build/updateという同一責務。直ちに分割しない。
**根本原因:** Godot `Control`の構築と更新が同じtyped node参照とpanel lifecycleを共有し、
独立した変更境界がまだ現れていないため。**判断: Defer。** pass-through panel wrapperは作らない。
**再評価:** 型定義・panel構築・panel更新が独立して変化するか、回帰やtest境界の不明瞭化が起きた場合。

## ~~C-10（#200）~~ 解消済み

`WorldSpace::render_scale()`と`ClientRules::min_warp_distance()`へ一本化済み。詳細は
[client-completed.md](./client-completed.md)を参照する。

## R-2（保留）: `main.gd` scene compositionの追加分割

`main.gd`は1276行。live state、interaction、presentationは既に分離済みで、radial action policyも
`ClientInteraction`へ移行済み。残るscene lifecycle / node generation / network send / HUD assemblyは
composition glueとして扱う。
**根本原因:** 残る処理はscene-tree構築とsession lifecycleを共有し、独立した変更境界をまだ持たないため。
**判断: Defer。** pass-through surfaceを増やさず、下記triggerが発火するまで現在のcomposition rootを維持する。
**再評価:** scene-tree構成を自動検証できるようになるか、新しい独立した変更理由がcomposition rootへ再流入する場合。

## C-20（保留）: `WorldPresentation`へのvisual lifecycle集積

`world_presentation.gd`は595行で、floating-origin rebase、navigation marker position/LOD、
sky/sun/fog/starfield bake、warp tunnel、player material/tactical overlayを調停する。
恒星光源モデルとprominence geometryは`StellarSource` / `SolarProminences`へ分離済み。
`NavigationMarkerRenderer`と`Starfield`のdeep moduleは既に分離され、現在のcoordinator自体は
一つのsession visual boundaryとして機能している。
**根本原因:** graphics feature追加のたびに、独立して変わるsetup/update lifecycleが同じ
`build`/`refresh` ownerへ集まるため。
**判断: Defer。** production部分が700行を超える、二つ以上のvisual familyを同時編集する回帰が続く、
またはtest fixtureがenvironmentとplayer effectの両方を常に構築するようになったら、最初に
sky/sun/fog/starfield bakeを`SpaceEnvironmentPresentation`へ抽出する。抽象的なvisual managerは作らない。

2026-10-31再観測: 637行で据え置き。`_setup_starfield`/`_update_starfield`がcatalogue合成、
shader/culling設定、camera追従を知る点も監視する。独立したstar-render変更が複数回callerと
`Starfield`へ波及するなら、まず既存Starfieldへ責務を寄せる。単なる行移動とforwardingは採らない。

2026-11-03整合確認: 695行。700行の閾値未満であり、行数だけで分割しない。

2026-11-05: 純粋な天体投影計算は`dawn-client-core::CelestialProjection`へ集約し、
Godotのcoordinatorは588行。状態を複製せず、observer選択はShipMotionに残す。
天体投影の重複計算は解消したが、environment全体の追加分割は引き続きDefer。

2026-11-07 doc-sync: 580行。ラベルを`ObjectCallouts`へ移し、planet labelのLOD追従を削除。
ShipMotionのrole/mode/visibilityもRustへ集約済み。C-20の分割triggerは未発火。

2026-11-12: 595行。#395/#396で光源・prominenceのleaf moduleは分離され、#398で銀河背景のmodel/render dataが
`GalacticModel` / `Starfield`へ整理された。700行trigger未満で、残るcoordinatorは一つのsession visual lifecycle。
environment familyの分割は引き続きDefer。

## C-21 解消済み: test専用のsystem-name Dictionary投影

Godot公開投影を削除。検証記録は[client-completed.md](./client-completed.md)を参照。

採らない方針:

- `main.gd`の機械的な`.tscn`分割
- raw `InputEvent`のdeep module流入
- static値を別の手動同期constants fileへ移すだけの対応
- 行数だけを減らすための汎用visual effect wrapper
