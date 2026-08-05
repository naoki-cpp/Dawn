---
id      : ADR-0047
title   : ClientCommand ディスパッチャの明示的 match を維持する
status  : accepted
date    : 2026-07-28
deciders: [human, ai-agent]
related : ADR-0037 (active ship / owned ship), ADR-0041/0042 (wire schema),
          docs/architecture/architecture.md
---

# ADR-0047 - ClientCommand ディスパッチャの明示的 match を維持する

## Context

`SimulationNode::apply_client_command`（`node/commands.rs`）は、ワイヤスキーマ
`ClientCommand` の全20バリアントを明示的に `match` し、コマンドごとに active ship
の解決、docked 判定、ドメイン操作への委譲、サーバ側 followup の生成を行う。

このうち station / inventory 管理系の8コマンドは、private enum
`StationDispatchCommand` へ再エンコードされて `node/command_station.rs` に渡る。
そのため同じ8種類が外側と内側の2箇所に現れる。

`/improve-codebase-architecture` のレビューはこれを「浅い seam」として、
`StationDispatchCommand` を削除し `dispatch_station_command` が `ClientCommand` を
直接受け取る案を挙げた。あわせて、ディスパッチャが20アームのまま縮まらない
真因は22個の `*_owned` メソッドの引数・戻り値がバラバラなことではないか、
という仮説も出た。

検討の結果、**どちらも採用しない**。理由を以下に残す。

## Decision

1. 外側の網羅的な `ClientCommand` match を維持する。
2. `StationDispatchCommand` を維持する。
3. `apply_client_command` が一度解決した active ship を
   `dispatch_station_command` へ引数で渡し、Dock / Undock 内での再解決を削除する。
4. `*_owned` メソッド群の引数・戻り値は統一しない。
5. `StationDispatchOutcome` を `StationDispatchEffect` へ改名する。

3 と 5 のみ実装した。

## 2026-08-05 amendment: family-local policy boundary (issue #264)

R-3 の再評価で、`apply_client_command` の網羅 match 自体ではなく、その各 arm に
flight / module / station / loadout-refresh の検証・適用方針が混在していることを
分割 trigger と判断した。元の decision を破棄せず、次の形へ境界を深める。

1. `ClientCommand` のワイヤ形状と、外側の網羅 match は維持する。
2. 網羅 match は payload を private な閉じた family enum へ分類するだけにする。
3. flight / module / loadout / station の各 module が、その family の検証・状態変更と
   family-local effect を所有する。
4. family effect から既存の `ClientCommandFollowup` への射影は、コマンド入口の
   一箇所だけが所有する。
5. active ship は従来どおり入口で一度だけ解決し、必要な family へ値として渡す。
6. 葉の `*_owned` API、拒否理由、domain event、wire protocol は変更しない。

これにより、新しい flight command の方針変更は `command_flight.rs`、module activation
の変更は `command_module.rs` の中で閉じる。一方、`ClientCommand` に variant が追加された
ときは、従来どおり一箇所の網羅 match がコンパイルエラーになり、どの family が所有するかを
明示的に決めさせる。

### 3 の理由

どのシップにプレイヤーのコマンドが向くか（ADR-0037 の active ship）は、
**コマンドを受け取ったことの性質**であって、station コマンドであることの性質では
ない。したがって解決はコマンド入口が所有すべきで、station ディスパッチャが
`self.ships.active_ship` を読み直すのは責務の重複だった。

現状は同じ値を2度読むだけで挙動に差はないが、片側にだけ条件が足されると
意味がずれる。性能ではなく所在の問題として直す。

### 5 の理由

`StationDispatchOutcome` と `StationOperationOutcome`（`node/station.rs`）は
名前が近いのに意味が違う。前者は「操作後にサーバが行うべき後続処理」、後者は
「ドメイン操作が受理されたか、拒否されたならその理由」である。`Effect` へ
改名して層の違いを名前に出す。

## Rejected alternatives

### `dispatch_station_command` が `ClientCommand` を直接受け取る

外側の8アーム（約60行）は消えるが、`ClientCommand` は20バリアントあるため
`_ => None` のキャッチオールが必要になる。得られるのは主に行数で、失うものは:

- station ディスパッチャの入力範囲を型で制限できなくなる。非 station コマンドを
  渡してもコンパイルが通り、静かに「担当外」として無視される。
- キャッチオールという新しい弱点が入る。

なお、レビュー時点では「`StationDispatchCommand` は station コマンドが閉じた8個の
集合であることを保証している」と評価したが、これは**過大評価**だった。private
enum が保証するのは「変換済みの値について内側 match が網羅的」であることだけで、
将来 `ClientCommand::RepairAtStation` が追加されたとき、それが station family に
分類されることまでは保証しない（外側で直接処理してもコンパイルは通る）。
それでも「非 station コマンドをこのモジュールに渡せない」という制限自体は
有効なので、維持する。

### `TryFrom<ClientCommand> for StationDispatchCommand`（`Error = ClientCommand`）

`Err` に元の値を返す lossless な部分集合変換は Rust として妥当だが、変換用の
8アームと実行用の8アームの両方が必要になり、現在の「外側8 + 内側8」を別の形に
移すだけで数は減らない。加えて外側では変換失敗後の `ClientCommand` を再度
match する必要があり、型システムは「`Err` 側に station バリアントは含まれない」
とは理解しない。

### `#[non_exhaustive]`

逆効果。定義 crate の外側でワイルドカードアームを強制する属性であり、
今回維持したい網羅性チェックをむしろ弱める。下流 crate に対してバリアント追加を
非破壊変更として許すための機構であって、内部ディスパッチャ整理の道具ではない。

### macro / trait によるディスパッチ生成

macro はバリアント一覧を一元化できるが、単純な制御フローを隠し、dawn-wire と
dawn-sector の結合を強める。trait 化しても `ClientCommand` から具体的な payload を
取り出す最初の match は残り、処理が各コマンド型へ散らばって「ワイヤ入口を1箇所で
読める」という現在の利点を失う。20バリアントでは割に合わない。

### `*_owned` のシグネチャ統一

`bool`（11個）、`Result<(), ModuleActivationRejection>`（2個）、
`StationOperationOutcome`（5個）、`Result<ShipId, StationOperationRejection>`（2個）
という4種類の戻り値は、**ドメイン操作の意味が違うことを正しく表している**。
共通型へ潰すと情報を失うか、巨大で曖昧な enum になる。

とくに `ModuleActivationRejection` は、以前 `bool` に潰されていたために
「docked で拒否された」と「所有していないので拒否された」が区別できなくなって
いた箇所を、named な拒否理由へ直したものである（同レビューの候補1）。これを
再び潰すのは、直したばかりの情報損失を再導入することになる。

ディスパッチャが戻り値を捨てているのは、結果が不要という意味ではなく層が違う
ためである:

- 葉の `*_owned`: ドメイン上の成否と拒否理由を返す。
- ディスパッチャ: サーバアダプタが次に行うべき effect（`ClientCommandFollowup`）
  だけへ写像する。

将来クライアントへ明示的な拒否理由を返す必要が出たときは、葉を統一するのでは
なく、外側の境界に写像用の型を足す。

## Consequences

- `ClientCommand` にバリアントを追加すると、外側 match の非網羅性でコンパイルが
  失敗する。変更検出器として機能し続ける。
- flight / module / loadout / station の各ディスパッチャは、自分の閉じた family enum だけを受け取る。
- active ship はコマンド受信境界で一度だけ解決される。
- コマンドハンドラのドメイン固有な結果型と拒否理由が維持される。
- ディスパッチャの行数削減そのものは目的にしない。外側の網羅 match は薄い family
  分類表として残し、family policy と follow-up 射影を混在させない。

## Reconsider when

- ワイヤプロトコル自体で Flight / Module / Station などのコマンドファミリを
  versioning・認可・レート制限の単位にする必要が出たとき。そのときは
  `ClientCommand` を入れ子 enum（`ClientCommand::Station(StationCommand)` など）へ
  変える。これが「station コマンドは閉じた部分集合」を型で完全に表す唯一の方法だが、
  postcard は位置依存でフィールドを読むため（ADR-0017 §6）ワイヤ形式の破壊変更に
  なり、client 生成コード・decode・テストをまとめて変える。ディスパッチャの60行を
  減らすために行う変更ではない。
- コマンドのメタデータから複数の成果物を生成する必要が生じ、複数の明示的 match の
  同期漏れが実際の不具合原因になったとき。
- コマンド拒否を統一されたワイヤ応答としてクライアントへ返す設計を導入するとき。

## Implementation checklist

- [x] `dispatch_station_command` に `active_ship: Option<ShipId>` を追加し、
      Dock / Undock の再解決を削除。
- [x] `apply_client_command` の8つの呼び出し側から解決済みの値を渡す。
- [x] `StationDispatchOutcome` → `StationDispatchEffect` へ改名。
- [x] `SelectActiveShip` の次のコマンドが新しい active ship に届くことを、
      公開 API 経由のテストで固定。
- [x] `command_station.rs` のモジュール doc に、private enum を残す理由を記載。
- [x] flight / module / loadout / station を private な閉じた family enum へ分類。
- [x] family-local effect を一箇所で `ClientCommandFollowup` へ射影。
- [x] family 選択、拒否経路、状態変更、全 follow-up variant の回帰テストを追加。
