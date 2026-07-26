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

ADR-0029 は、サーバー上の船を「アンカー + ローカルオフセット」、
クライアントを浮動原点 + Godot `Vector3` とする方式をAcceptedにした。
この方式は、近傍の戦闘演算をf32で処理しつつ真スケールの天体配置を扱うための
実装方針として成立している。

しかし現在の実装では、同じ「位置」が複数の意味で流れている。

- `PositionComp` はアンカー相対のf64オフセット
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
Velocity          : 1 tick あたりの変位。f64。サーバー物理・wire・クライアント予測の基準。
```

移行完了までのルールは以下とする。

1. 新しいサーバーコードは、絶対座標とアンカー相対オフセットを同じ型で表さない。
2. 絶対距離、AoIセル判定、ゲート・ステーションの範囲判定、ワープ曲線はf64で行う。
3. 速度、近傍の1 tick移動、操船距離、AOIセル境界はf64で計算する。
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
- **描画まで全てf64にする:** Godot標準の`Vector3`を置き換える必要があり、状態と
  シミュレーションをf64にする今回の目的を超える。f64からGodot `Vector3`への変換は
  浮動原点との差分を取った最後の境界に限定する。
- **i64固定小数点へ移行する:** 決定論上の利点はあるが、現在の物理・wire・表示の単位
  と合わず、f64の境界問題を解消するための変更としては過大である。

## 実装チェックリスト

- [x] 人間が本ADRを承認し、`status`を`accepted`へ変更する
- [x] `dawn-core::AbsolutePosition` を定義し、静的な天体・ゲート・ステーション定義の絶対座標に適用する
- [x] サーバーの位置・距離・AoI・ナビゲーション判定をf64経路へ移行する（AnchorTable / combat / AoI CellGrid / TransitOp / ship_absolute / WarpComp / entity_absolute_f64 / PositionComp / orbit / keep-at-range を含む）
- [x] `PositionComp`と`AnchorComp`の移行方針を決定し、互換読み取りを隔離する（PositionCompはアンカー相対f64オフセットとして維持し、AbsolutePositionへの変換はAnchorTable / ship_absoluteに限定。直前mainのsnapshot / transit payloadは専用previous decoderで読み取り、既存の`absolute_position`を保持したままf64へ拡張）
- [x] 位置・速度を含むDomainEvent、snapshot、wire schemaを同じ移行で更新する（`AbsPosWire` / `PosWire` / `VelWire` と移動プロファイル値をf64化し、生成schemaを更新）
- [x] f64 wire位置をクライアントで`Vector3`へ変換する前に差分計算するテストを追加する

- [x] AU桁のゲート・ステーションで表示位置と近接判定が一致するテストを追加する
- [x] 旧アンカー経路を削除する前にreplay・transit・warpの互換性を検証する（既存のreplay・transit・warp回帰テストで確認）
- [x] `docs/architecture/entity-model.md`と`docs/architecture/wire-protocol.md`を実装に同期する

実装時は `dawn-client-core::WorldSpace` が絶対f64座標・浮動原点・軸変換を所有し、
`dawn-client-gdext::WorldSpace` は最終的なGodot `Vector3`/`PackedFloat64Array`変換だけを担当する。
GDScript側はNode3Dの配置・原点リベース時のシーンツリー更新を担当する。

## 影響と保留事項

永続化されたpostcardデータは自己記述型ではないため、型変更だけで旧データを
現行型へ直接deserializeしてはならない。実装では次の互換境界を採用した。

- `FileEventStore` は新規ログに `DAWNEVT2` ヘッダーを書き、旧8バイト
  `base_index` ヘッダーのログも認識する。旧ログの `VelocityChanged`、
  `AnchorRebased`、`SectorTransitCompleted` は固定された直前世代のf32型からf64へ
  widenして読み込む。新規追記またはCompaction時には現行型として保存される。
- `StateSnapshot` は新規ファイルに `DAWNSNP2` マーカーを付ける。マーカーのない
  直前mainのsnapshotは、`absolute_position`を含む固定されたf32
  `Position` / `Velocity` / `SectorBounds` 型で先に読み込み、f64へ変換する。
  Transit payloadも同じ直前世代のShipSnapshotとf32 `entry_pos`を専用型で読む。
  さらに古いpre-ADR-0044形式は、この移行の互換対象に含めない。
- `TransitOp` は新規payloadに `DAWNTRN2` マーカーを付け、マーカーなしの直前main
  payloadだけをprevious decoderで読み込む。
- 直前世代のイベントログ、snapshot、Transit payloadを実バイト列で読み込む回帰テストを維持する。
