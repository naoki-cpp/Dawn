---
id      : ADR-0045
title   : クライアント移動状態と表示積分の統合
status  : accepted
date    : 2026-07-23
deciders: [human, ai-agent]
related : ADR-0008 (VelocityChanged), ADR-0023 (movement physics),
          ADR-0029 (floating origin), ADR-0043 (client-side prediction),
          ADR-0044 (absolute f64 coordinate authority)
---
# ADR-0045 — クライアント移動状態と表示積分の統合

## 背景

ADR-0043 は`dawn-client-core::MotionPredictor`を、ローカル予測とリモートの
dead-reckoningで共有する深いモジュールとして導入した。しかし実装上は、同じ船の
表示位置を複数の経路が更新している。

- `MotionPredictor`が予測位置と補正残差を保持する
- `ShipController`が通常時に予測位置を適用する
- `ShipController`がワープ時に別の速度上限積分を行う
- `main.gd`がdock、jump、warp arrivalで`global_position`を書き換える
- `WorldPresentation`が浮動原点移動時に船ノードを直接移動する

速度イベント、MotionCorrection、PositionSnap、フレーム時間の境界も、同じ船の
状態を別々の時計で進める原因になっている。これが移動中のがたつき、ワープ後の
方向固定、dock/undock後の残速表示などを局所的な修正で繰り返す温床になっている。

## 決定

クライアントの船ごとに、Rust側の1つの`MotionTrack`（既存の`MotionPredictor`を
拡張または置換）だけが移動状態と表示位置を決定する。少なくとも次の状態を明示する。

```text
NormalPrediction  : 自船。入力を受け、MovementProfileで予測する。
DeadReckoning     : 他船。最後の権威速度とtickから進める。
WarpPresentation  : ワープ中。権威到着までの表示経路を1つだけ持つ。
Docked            : 入港中。位置・速度・入力を固定する。
```

各状態の遷移は、通常の速度更新・権威補正・warp arrival・dock/undockなどの明示的な
入力としてRust側に集約する。GDScriptは入力を渡し、結果の`render_position`を
Node3Dへ適用する薄いアダプターに限定する。

必須契約は以下とする。

1. 1フレームの通常経路で、船のNode3D位置を書き込む箇所は1つだけにする。
2. ワープ表示用の速度上限積分をGDScript側に別実装しない。ワープ状態の表示方針を
   `MotionTrack`が所有する。
3. `VelocityChanged`の`tick`、`MotionCorrection`の`tick`をトラックへ渡し、受信順では
   なく論理tickで適用する。遅れた速度イベントでフレーム積分を巻き戻さない。
4. `PositionSnap`、dock、undock、jumpは連続補間ではなく、明示的なdiscontinuity
   としてトラックをリセットする。
5. 浮動原点の変更はトラックと描画ノードへ同じ座標変換として通知する。原点管理側が
   船の位置を独自に補間・補正しない。
6. `MotionTrack`の入力・出力はサーバー絶対座標と描画ローカル座標を混同しない。
   ADR-0044の絶対f64値は、原点差分を計算した後にのみGodot `Vector3`へ変換する。

### 実装反映（2026-07-26）

`dawn-client-core::ShipMotion` が、1隻分の `MotionPredictor` と `WorldSpace` を
command-in / frame-out の境界で束ねる。公開する更新経路は `MotionCommand`、描画側が
読む結果は `MotionFrame` とし、`main.gd` や `ShipController` が tick、補正方式、座標
変換の順序を組み立てない。

- `MotionCommand` は server-space の入力、権威サンプル、速度イベント、discontinuity、
  rebaseを受け取る。stale tick、非有限値、無効なwarp値は境界で拒否する。
- Godotの `VISUAL_SPEED_CAP` のようなrender-spaceの速度上限は、GDExtensionが
  `WorldSpace` の単位変換を通してserver-spaceへ変換してから `MotionCommand` に渡す。
  そのためコマンドの `server_speed_cap` は常にサーバー単位である。
- `MotionFrame` は権威位置、予測位置、render位置、server/render速度、状態、tickを
  明示的に分ける。warp中の表示位置を権威位置として扱わない。
- `dawn-client-gdext::ShipMotion` は `PackedFloat64Array` の絶対座標をRustへ渡し、
  `Vector3` への縮小はrender frameの最終境界だけで行う。
- `WorldPresentation` は全船へ同じabsolute originを通知する。rebase後の船ノードを
  `shift` 加算する処理は持たず、各 `ShipMotion` のrender frameを適用する。

`WorldSpace` GDExtension wrapperは座標変換の互換境界として残すが、旧
`MotionPredictor` GDExtension wrapperは削除する。coreの`MotionPredictor`は
`ShipMotion`内部の予測器として残し、productionの `ShipController` は `ShipMotion`だけを
使用する。サーバー物理の
`MovementProfile`、速度、位置、移動距離はサーバーとクライアントコアで`f64`を共有する。
Godot `Vector3` は描画アダプターの最終境界にだけ残る。

## 採用しなかった選択肢

- **現在の複数経路を個別に補正する:** 一時的なバグ修正はできるが、予測器・ワープ積分・
  直接snapの優先順位が暗黙のまま残り、次の不具合を生む。
- **毎フレームの絶対位置をサーバーから送る:** 表示は単純になるが、帯域とイベント
  ソーシングの責務を壊し、クライアント予測の利点も失う。
- **Godotをdouble precisionビルドにする:** 表示精度の問題の一部しか解決せず、状態の
  二重化とtick同期の問題を解決しない。

## 実装チェックリスト

- [x] 人間が本ADRを承認し、`status`を`accepted`へ変更する
- [x] `MotionTrack`の状態と遷移をRustの型または明示的な状態APIで表現する
- [x] 通常移動・リモート移動・ワープ・dockを同じRustトラックでテストする
- [x] `VelocityChanged.tick`をクライアントのトラックへ渡す。速度だけのイベントは
  予測時計を巻き戻さず、authority tickのwatermarkを更新し、遅延イベントを拒否する
- [x] GDScriptのワープ専用積分と直接的な競合位置書き込みを削除する。ワープ表示は
  `MotionTrack`、discontinuityの適用は`ShipController`に集約した
- [x] Node3D位置の単一writerを構造で保証する。通常フレーム、snap、dock/undock、
  floating-origin rebaseはすべて`ShipController`の適用入口を通る
- [x] 浮動原点rebase、遅延補正、warp arrival、dock/undockの回帰テストを追加する
- [x] GDExtension APIを更新し、旧`MotionPredictor` shimを削除してGodot側は
  `ShipMotion`を使う薄い変換アダプターに戻す
- [x] ADR-0043の実装チェックリストと`docs/architecture/tick-model.md`を同期する

## 期待する結果

f32/f64の選択を表示経路ごとに判断するのではなく、権威状態・論理tick・表示状態の
所有者を固定できる。これにより、位置精度の問題と同期タイミングの問題を別々のテスト
として再現でき、ワープやdockだけ特別扱いする表示コードを増やさずに済む。
