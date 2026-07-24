---
id      : ADR-0044
title   : サーバー権威座標を絶対 f64 に統一する方針
status  : accepted
date    : 2026-07-23
deciders: [human, ai-agent]
related : ADR-0028 (大規模座標の方式比較), ADR-0029 (現行アンカー方式),
          ADR-0019 (AoI 空間インデックス), ADR-0042 (wire protocol)
---
# ADR-0044 — サーバー権威座標を絶対 f64 に統一する方針

## 背景

ADR-0029 は、サーバー上の船を「アンカー + ローカル f32 オフセット」、
クライアントを浮動原点 + Godot `Vector3` とする方式をAcceptedにした。
この方式は、近傍の戦闘演算をf32で処理しつつ真スケールの天体配置を扱うための
実装方針として成立している。

しかし現在の実装では、同じ「位置」が複数の意味で流れている。

- `PositionComp` はアンカー相対のf32オフセット
- `AnchorTable` は天体の絶対f64座標
- `InitialState` / `AoiEnter` / `PositionSnap` / `MotionCorrection` は絶対f64
- 一部の `EventWire` と互換用のクライアントイベント処理はf32の`Vector3`
- 変更前のクライアントナビゲーションと近接判定は、絶対f64を`Vector3`へ狭めた後に行われていた

その結果、変更前はゲートやステーションの表示位置と判定位置がずれる、アンカーをまたぐ
距離判定で精度を失う、通常移動とワープで異なる座標経路を通る、といった問題を
同じ種類の「座標バグ」として繰り返し発生させる構造になっている。
`docs/reference/carbon-engine-comparison.md` の再評価トリガーも満たした。

## 決定

真スケール対応の長期方針として、サーバーの権威的な船・天体・ゲート・ステーション
の位置を、Sectorフレームの絶対f64座標に統一する。クライアントの描画だけは、浮動原点
からの近傍オフセットへ変換してGodot `Vector3`（f32）で保持する。

目標とする値の意味は次のとおり。

```text
AbsolutePosition  : Sectorフレームの絶対位置。f64。サーバー権威・wireの基準。
LocalOffset       : 絶対位置 - 浮動原点。クライアント描画用。Godot Vector3。
Velocity          : 1 tick あたりの変位。現段階では f32 を維持する。
```

移行完了までのルールは以下とする。

1. 新しいサーバーコードは、絶対座標とアンカー相対オフセットを同じ型で表さない。
2. 絶対距離、AoIセル判定、ゲート・ステーションの範囲判定、ワープ曲線はf64で行う。
3. 速度と近傍の1 tick移動はf32を維持し、位置積分時に必要ならf64へ昇格する。
4. クライアントはサーバー座標を`Vector3`として保存しない。f64コンポーネントのまま
   `WorldSpace`で原点との差分を計算し、最後にだけGodotの`Vector3`へ変換する。
5. `AnchorTable`、`AnchorComp`、`AnchorRebased`は移行期間の互換層とし、新しいゲーム
   ロジックがこれらを直接読むことは許可しない。
6. wireの位置フィールドは「絶対」か「クライアント向けローカル」かを型と名前で
   明示し、同じ`PosWire`を意味の異なる座標に使い回さない。

最終的な型変更は、`dawn-core::Position`の意味・型、イベント、スナップショット、
wireスキーマを一括した移行として扱う。ADRの承認前に、既存の`Position`を部分的に
f64へ変更したり、アンカー方式と絶対方式を新機能ごとに使い分けたりしない。

## 採用しなかった選択肢

- **アンカー相対f32を長期方針として維持する:** 現行ADR-0029の実績は尊重するが、
  絶対値と相対値の境界を守るための変換箇所が増え、今回のような判定・表示の不一致を
  根本的には減らせない。
- **速度・描画を含めて全てf64にする:** Godot標準の`Vector3`と既存のwire帯域に対して
  必要以上の変更となる。位置の絶対精度が問題の中心であり、速度までf64にする根拠は
  現時点ではない。
- **i64固定小数点へ移行する:** 決定論上の利点はあるが、現在の物理・wire・表示の単位
  と合わず、f64の境界問題を解消するための変更としては過大である。

## 実装チェックリスト

- [x] 人間が本ADRを承認し、`status`を`accepted`へ変更する
- [x] `dawn-core::AbsolutePosition` を定義し、静的な天体・ゲート・ステーション定義の絶対座標に適用する
- [ ] サーバーの位置・距離・AoI・ナビゲーション判定を絶対f64経路へ移行する（AnchorTable / combat / AoI CellGrid / TransitOp / ship_absolute / WarpComp / entity_absolute_f64 は移行済み。PositionComp と低レベル補間用配列が残る）
- [x] `PositionComp`と`AnchorComp`の移行方針を決定し、互換読み取りを隔離する（PositionCompはアンカー相対f32オフセットとして維持し、AbsolutePositionへの変換はAnchorTable / ship_absoluteに限定。旧snapshot / transit payloadは専用legacy decoderで読み取り、absolute_positionは`None`へ変換）
- [ ] 位置を含むDomainEvent、snapshot、wire schemaを同じ移行で更新する（snapshot と ShipSpawned / SectorTransitCompleted / JumpGateUsed の移行済み。残るイベント境界を整理する）
- [x] f64 wire位置をクライアントで`Vector3`へ変換する前に差分計算するテストを追加する

- [x] AU桁のゲート・ステーションで表示位置と近接判定が一致するテストを追加する
- [x] 旧アンカー経路を削除する前にreplay・transit・warpの互換性を検証する（既存のreplay・transit・warp回帰テストで確認）
- [x] `docs/architecture/entity-model.md`と`docs/architecture/wire-protocol.md`を実装に同期する

実装時は `dawn-client-core::WorldSpace` が絶対f64座標・浮動原点・軸変換を所有し、
`dawn-client-gdext::WorldSpace` は最終的なGodot `Vector3`/`PackedFloat64Array`変換だけを担当する。
GDScript側はNode3Dの配置・原点リベース時のシーンツリー更新を担当する。

## 影響と保留事項

これは既存イベントのフィールド型を変える可能性があるため、実装時にはイベント
スキーマとsnapshotの移行戦略を先に決める。リリース前のため旧ログをそのまま維持
するか、移行ツールを用意するかは実装PRで確定する。
