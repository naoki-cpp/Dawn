---
date: 2026-07-04
---

# Carbon Engine（CCP/EVE Online 内製エンジン）との設計比較

このドキュメントは [eve-reference.md](./eve-reference.md) §9.1「CarbonIO & BlueNet」の
一部として書かれた内容を独立させたもの。EVE Online 全般の技術資料は引き続き
`eve-reference.md` を参照し、本ドキュメントは **2026-07-01 に MIT ライセンスで
オープンソース化された Carbon Engine 実ソースコードとの比較**に特化する。

## 読み方の前提

EVE は 2003 年から20年以上、単一シャードで実際に数千人同時接続を本番運用してきた
ゲームである。本ドキュメントで確認する `destiny`/`db`/`scheduler` 等の設計は、その
年月の実戦で磨かれた結果であり、当て推量ではない。一方 dawn は本番負荷を一度も
経験していない。

以前のバージョンのこのドキュメントは「EVE と一致した点 → dawn の設計が裏付けられた」
「EVE と違う点 → dawn は意図的にそう選んだ、今は変更不要」という**非対称な確証バイアス**
に陥っていた。「dawn は違う道を選んだ」ことと「その道が正しい」ことは別の話であり、
以下では相違点ごとに **EVE が実戦でその設計にたどり着いた理由** と **dawn の選択が
まだ実戦で試されていないという留保** を必ず併記する（`eve-reference.md` §11 と同じ
趣旨・同じ厳しさを Carbon 比較に適用する）。

## 採用可否

**採用しない。** ただしこれはコード流用の可否についての判断であり、「dawn の設計が
EVE より優れている」という意味では全くない。理由は3つ——

1. 全リポジトリが C++/C/Python 前提で Rust バインディングが存在せず、`vcpkg`/CMake の
   ビルド系列を dawn の Cargo workspace に持ち込むことになる（AI_DEVELOPMENT_GUIDE.md の
   「新規依存追加は DAG とコストを吟味」方針に反する）。
2. `destiny`/`blue` は CarbonIO/BlueNet と同じ Stackless Python 時代の「1 ソーラー
   システム＝1 モノリシックノード」設計を C++ で下支えする土台であり、dawn とは
   Cargo/Rust という技術スタックの前提が根本的に異なる。
3. 一部モジュールは "commercial license dependencies remain" と報じられており、MIT 表記
   だけでは依存関係の法的検証が済まない。

**採用しない = dawn の代替設計が優れていることの証明ではない。** 以下の各項目は
その前提で読むこと。

## `destiny`（EVE の物理/衝突シミュレーション本体、C++）を実際に読んだ設計比較

（コード流用ではなく設計参照のみ。以下は `destiny` リポジトリの実ソース確認済み）

### 座標精度 — dawn のアンカー方式は「今も現在進行形でバグを生んでいる」設計

`Ballpark.h` の `const double AU = .1495978707e12;` は dawn の `UNITS_PER_AU`
（1.495978707e11）と完全一致する。しかし `Ball.h` の位置表現 `Vector3d mNewPos` は
**常に素の f64、アンカー再基準なし**であり、EVE は 20 年以上この方式だけで真 AU
スケールの座標精度を満たしてきた。

dawn の ADR-0029（アンカー＋f32オフセット）は、このセッションだけでも
f32-cast-before-subtract の精度バグを複数回踏んでいる（`entity_absolute_f64`/
`dest_in_ship_frame_abs` への集約、PR #68 等、いずれもバグ修正コミット）。これは
「将来の設計変更の機会に再検討する価値がある」という悠長な話ではなく、**現時点で
実際にバグを生み続けている設計**である。EVE が同じ精度要件を素の f64 だけで
満たしているという事実は、「dawn のアンカー方式は本当に必要だったのか」という
疑いを強める方向の証拠であり、単なる参考情報として棚上げするべきではない。
dawn がアンカー方式を導入した動機（クライアント f32 レンダリング・帯域）は理解できるが、
サーバー権威状態まで f32 の影を引きずらせる必要が本当にあったのかは、繰り返しバグを
踏んでいる時点で疑うべき設計判断である。
**再評価トリガー**: 次に anchor 関連のバグを踏んだ時点で、素の f64 移行の再設計コストを
正式に見積もる（先送りを重ねているだけという自覚を持つ）。

### 空間分割 — dawn の固定グリッドが「機能している」のはまだ EVE 規模を経験していないから

`Partition.h` は空間を箱（Box）の階層（`mFineLevel` で細かい格子、`mGridBase` で
その上に粗い格子）に分割し、**ボールが存在する箱だけを実際に確保する**
（"mostly empty space, as it should be"）。dawn の AoI（`dawn-sector/src/aoi.rs`、
ADR-0019）は固定 3×3×3 セルグリッドで、密度に関わらず固定サイズを常に確保・走査する。

固定グリッドは、Sector が疎（大半のセルが空）になるケースで**確実に**無駄な走査・
メモリを払う。これは推測ではなく固定グリッドの数学的性質そのものである。
「現行グリッドは機能している」という根拠は、**dawn がまだ EVE スケールの疎な
Sector を一度も運用したことがない**ことの裏返しにすぎない。EVE がわざわざ疎な
階層 Box 分割を実装したのは、固定グリッドがスケールしないと**実戦で学んだ**
からである。dawn が同じ壁に当たらない保証はどこにもなく、「今は不要」は
「まだ壁に当たっていないだけ」と読み替えるべきである。

### Bubble（間引き） — dawn は「未検証の利益」のために「実証済みの最適化」を放棄している

`bubbleKeepAlives`/`InitializeBubbles` 等: EVE は「interactive（プレイヤー操作）な
船を含むクラスタだけ活性化」という、シミュレーション自体を間引く機構を持つ。この
最適化の効果は EVE の 20 年の本番実績で実証済みである。

対して dawn は Sector 内を常に全船等しくシミュレートし、配信だけを AoI でフィルタする
（ADR-0019）。この選択の根拠は INV-002（決定論的リプレイ）だが、**決定論的リプレイが
実際にどれだけの価値を生むかは dawn 側では未検証**である（リプレイ機能自体、dawn に
まだ実装されていない）。つまり dawn は「まだ価値が実証されていない利益」のために
「価値が実証済みの最適化」を放棄している構図になっている。
（`eve-reference.md` §11.2 で INV-001/002/FBD-001 の内部矛盾は指摘済みだが、
「INV-002 がそもそも他の設計判断を犠牲にしてまで守る価値があるか」自体は
まだ検討されていない。）
過密 Sector で間引きが必要になった時点で、INV-002 を弱めるか bubble 相当を入れるかの
二者択一に直面する。今のうちに「INV-002 を弱めても良い条件」を明文化しておかないと、
負荷が出た瞬間に不変条件を場当たり的に破ることになりかねない。

### Ball のモード体系 — EVE にあって dawn にない機能は「まだ実装していないだけ」であって「不要」ではない

`DstConstants.h` の `DstBallMode`: `GOTO`（≈dawn Move）・`FOLLOW`（≈Approach）・
`STOP`（≈Stop）・`WARP`（≈Warp）・`ORBIT`（≈Orbit）・`MISSILE`（誘導弾）・
`MUSHROOM`（拡大する球状 AoE）・`BOID`（群れ行動）・`TROLL`（デブリ物理）・
`FIELD`（力場ボール）・`RIGID`（不動）・`FORMATION`（編隊飛行）。dawn のコマンド集合と
GOTO/FOLLOW/STOP/WARP/ORBIT はほぼ1対1で対応するが、MISSILE/MUSHROOM/BOID/TROLL/
FIELD/FORMATION は dawn にまだ存在しない。

dawn に KeepAtRange 相当のモードが EVE 側に見当たらない点を「dawn 独自の優位な拡張」
と捉えがちだが、逆から見れば「EVE の 20 年の設計陣が KeepAtRange 型の操船モードを
必要としなかった」という事実でもある。KeepAtRange が本当にプレイヤー体験を
改善するのか、単に dawn 独自の複雑さが増えただけなのかは、実プレイヤーの
フィードバックなしには判断できない。

### Warp の物理式 — dawn の smoothstep モデルは「テスト済み」の意味が違う

`Ballpark.cpp::SetupWarpConstants` は導出過程がコメントされた完全な閉形式の指数
加減速モデルを持つ: 加速フェーズ `x = exp(ACC·t)`、巡航フェーズは等速、減速フェーズは
指数減衰。距離が短すぎて巡航フェーズがマイナスになる場合は最高速度 `warpSpeed` 自体を
下げて調整する（`warpSpeed = min(warpSpeed, (D+1)·ACC·DEC/(ACC+DEC))`）。

dawn の warp（ADR-0022、`node/warp.rs`）はパラメトリックな smoothstep イージングで
近似している。「現行 smoothstep は機能しておりテスト済み」という記述は、**単体
テストが数個通っている**ことを指しているに過ぎない。EVE の閉形式指数モデルは、
warp 中の割り込み・warp scrambler/disruptor によるキャンセル・warp core stabilizer の
スタック・bumping（他船との衝突による warp 阻害）等、**数百万プレイヤー時間の
悪用探索**を経て現在の形に収束した式である。dawn の smoothstep モデルは、こうした
edge case（途中キャンセル時の速度連続性、極端に短い/長い距離での挙動、複数の
warp 妨害効果の重ね合わせ）が**まだ一つも実戦で試されていない**。「今は機能している」
を「十分検証されている」と混同しないこと。Tackle/Warp Disruptor 実装（ADR-0033 系）が
進むほど、この差は表面化しやすい。数式自体は公開ソースとして確認済みなので、
問題が顕在化した際の具体的な参照式として記録しておく。

### `Ballpark::Integrate` — 唯一、dawn の設計が実戦の裏付けを得られた項目

`v(t) = (m·a - (m·a - v0·k)·exp(-k/m·t)) / k` という抗力係数 `k` 付きの指数減衰
積分器で、dawn の `τ = mass × inertia_modifier / MASS_SCALE`、
`α = 1 - exp(-1/τ)` という近似（`dawn-ecs/src/systems/movement.rs`、ADR-0023）と
本質的に同じ物理（質量×抗力係数による指数収束）。これは dawn が独自に導出した
モデルが、EVE の実際の本番実装と数式レベルで一致していたという意味で、数少ない
正当な「裏付け」の一つと言える（他の一致点である AU 定数も含め、この2点は本ドキュメントの
中で最も確度の高い確認事項）。

### Tick 内イベント — 「疎結合の方が優れている」は性能データを伴わない美意識の主張

`DstEventTypeChooser`: `DST_PROXIMITY`/`DST_RANGE`/`DST_PARTITION`/
`DST_WARPACTIVATION`/`DST_WARPEXIT` 等、物理 tick 自体が発行するイベント。dawn は
これを ECS System（Capacitor/Combat/Repair 等）と `DomainEvent` の分離で表現している。

EVE が物理エンジンにイベント発行を埋め込んでいるのは無知だったからではなく、
**物理 tick の中で直接イベントを発行する方が間接層・アロケーションを減らせる**という、
大規模戦闘のホットパス最適化の可能性がある。dawn の ECS System + `DomainEvent` 分離は
設計上は見通しが良いが、2,670 隻規模の戦闘で同じスループットを出せるかは一度も
測定されていない。「dawn の設計の方が優れている」と実測せずに結論するのは早計であり、
本ドキュメントの旧版がそう書いていたのは誤りである。

## Carbon の他リポジトリ（29 中、dawn に関連しうる4つを実ソース確認）

### `pathfinder`（EVE マップ上の経路探索）

`EveMapPathfinderCache.h`: open list を `std::priority_queue`、closed list を
**ハッシュマップではなくハンドル（`EveMapClosedListNodeID`）添字の連続 `std::vector`**
にする設計で、キャッシュ局所性とアロケーション回避を狙う。`EvePathfinder.h` は
「クエリごとに独立キャッシュを持たせてマルチスレッド／時分割可能」と明記。

dawn は現状ソーラーシステム間の複数ホップ経路探索を持たない（warp/jump は単一
Sector 内の操舵、`docs/architecture/tick-model.md`）。これは「dawn には不要な機能」
ではなく「dawn がまだそこまでの規模の世界を作っていないから存在しない」機能である。
将来 Sector 間ルーティングを実装する場合、`HashMap` ではなく `Vec<ClosedNode>` を
`NodeId` で添字するこのパターンは Rust に移植しやすい具体的なテンプレートになる。
ゴール判定を `IEvePathfinderGoal` インターフェースで分離（最短経路 vs Nジャンプ
以内の到達可能性フラッドフィル）している設計も参考になる。

### `io`（低レベルネットワーキング）

中身は CPython の `socket`/`ssl`/`select` を Stackless tasklet 用にパッチした C
拡張で、`protocol.h` は MachoNet パケットの4バイトヘッダ＋zlib/snappy 圧縮フラグの
みである。dawn の ADR-0007（WebSocket + JSON、Hello/Welcome/Redirect ハンドシェイク）
とは設計世代が異なる（協調的シングルプロセス vs dawn の非同期 Tokio）。dawn の
非同期モデルはこの世代の設計をまるごと置き換えるものであり、ここは素直に世代の
違いとして受け取ってよい（EVE も現在はこの設計のままではない可能性が高く、
20 年前のアーキテクチャと現行 dawn を比べても公平ではない）。

### `db`（ゲームサーバーの DB アクセス層） — 「採らなかった道の裏付け」と呼ぶのは公平でない

`SessionPool.h`/`.cpp`: 空き `CSession` が無ければ呼び出し tasklet を tasklet 間
チャンネルでブロックする、min/max ウォーターマーク付きのコネクションプール
（ATL OLE DB、ワーカースレッドでアイドルセッションを事前ウォームアップ）。
イベントソーシング・追記専用ログ・リプレイの概念は無く、「SQL の行を直接変更する」
同期 RDBMS モデルである。これは ADR-0001 で dawn が却下した選択肢（案A：mutable
state sync）そのものだが、これを「却下して正解だった裏付け」と呼ぶのは自己の結論を
確認しただけの循環論法に近い。

EVE の市場・スキル・資産システムは、この「退屈な」同期 RDBMS モデルの上で **20 年間、
数百万アカウントの経済データを一度も破損させずに**運用されてきた。ACID 保証・
バックアップ/レプリケーションツールの成熟度・運用ノウハウの蓄積は、dawn の自作
イベントストアには存在しない。dawn のイベントソーシングは「原理的には優れている」が
「実戦で 20 年分の耐久性を証明していない」。この非対称を無視して「EVE は古い設計」と
読むのは、実績のある技術を過小評価している可能性がある。イベントストアの障害復旧・
破損検知・運用ツールの成熟度は、Phase 8 で本番運用を語る前に正直に見積もるべき負債
として扱うこと。

### `scheduler`（Greenlet/tasklet 協調スケジューラ） — TiDi の存在自体が EVE の逐次処理も限界に当たった証拠

`ScheduleManager`/`Tasklet` に `RunNTasklets`/`RunTaskletsForTime(timeout)` という
tasklet数・時間予算の両方で打ち切れる実行制御があり、tasklet 間通信は共有メモリの
ロックではなく CSP 的な `PyChannel`（ランデブー）である。

これを「機構は不要、Tokio + 固定パイプラインで解決済み」と切り捨てるのは早計である。
**TiDi が存在すること自体が「EVE の逐次的 tick 処理はいずれ限界に当たる」という
実証結果**である。EVE は Stackless Python の非同期実行系（tasklet）と時間予算による
打ち切り機構を既に持っていた上で、それでも TiDi が必要になった。dawn の `tick.rs` も
本質的には同じ「1 tick で全 Sector を逐次処理する」設計であり、TiDi（ADR-0018）を
「最後の手段」としてすでに用意しているのは、dawn 自身もこの限界を見越しているから
に他ならない。「Tokio があるから大丈夫」という楽観は、EVE も非同期実行系を持っていた
上で TiDi が必要になったという事実によって反証される。dawn の tick 予算・過負荷対応
（ADR-0018、人口上限）の設計思想は、EVE の tasklet 時間予算と方向性としては同じ
問題を後追いで解いているに過ぎない。

## 残り 20 リポジトリ（`.github` を除く 28/29 を実クローンして確認済み）

以下は dawn との関連有無を判断せず、事実として何であるかのみ記録する（この範囲は
dawn の設計と直接競合しない周辺ツール・クライアント資産のため、批判的検討の対象外）:

- **`core`** — 低レベル共通基盤（C++）。`CCPMemoryTracker`/`CCPStatistics`/
  `CCPTelemetry`/`CCPLog`/`CCPHash`/`CcpSecureCrt`/`CcpSemaphore` 等、OS 抽象化と
  計測・診断のプリミティブ集。全 Carbon コンポーネントが依存する土台。
- **`math`** — ベクトル/行列/クォータニオン/平面/球/AABB/AxisAlignedEllipsoid 等の
  基礎数学ライブラリ（`include/Vector3.h`, `Matrix.h`, `Quaternion.h`, `Plane.h`,
  `Sphere.h`, `AxisAlignedBox.h` など）。C++、ヘッダ+inline 実装中心。
- **`geo2`** — Microsoft DirectXMath 上に構築した Python 向け数学ライブラリ
  （`Geo2.cpp`/`Vector.cpp`）。`math` とは別に Python 露出用の薄いラッパー層。
- **`parser`** — 数式パーサ（`ccpparser.h`/`.cpp`）。C++、独立した式評価ライブラリ。
- **`prometheus`** — `prometheus-cpp`（jupp0r/prometheus-cpp）をラップして Python
  拡張として露出する `prometheus_module`（Counter/Gauge/Histogram/Summary
  対応、`registry.Serve(port)` でメトリクスを pull 提供）。dawn には現状メトリクス
  基盤が無く、これは純粋に「まだ持っていない運用ツール」として認識しておくべき
  （観測性の欠如は本番運用に近づくほど負債化する）。
- **`grpc`** — プロジェクト固有の Python gRPC モジュールを作るための共通ビルド
  基盤（生成コードのパッケージング規約を統一する土台であり、gRPC 自体の実装ではない）。
- **`resources`** — Carbon タイトル向けリソースファイル（`BundleResourceGroup` 等）の
  管理・変換・配信ツール群。CLI とライブラリの両方を提供。
- **`vcpkg-registry`** — CCP 独自の vcpkg レジストリ（ツールチェイン・triplet 定義、
  `update_ports.py`）。ビルドインフラであり実行時コードではない。
- **`blue`** — "Python と C++ の接着剤"。組み込み Python インタプリタ・ゲームループ・
  リソースロードを提供する中核ライブラリ（`BlueMain.h`/`BlueNet.h`/`BlueAsyncRes.h`/
  `BitPacker.h`/`Base64.h`）。`destiny` はこの上で動く。Perforce 依存の外部ビルド
  手順が README に明記されており、単体でのビルドは前提としていない。
- **`blueexposure`** — C++ クラス/関数から Python ラッパーを自動生成するコード生成
  ライブラリ（`BlueClasses.cpp`/`BluePythonThunkers.cpp`/`BlueMemberIterator.cpp`）。
  `blue` の相棒。
- **`pdm`（Platform Detection Module）** — OS 非依存のマシン情報収集ライブラリ
  （CPU拡張命令検出 `cpu_extensions.h` 含む）、CLI (`pdmCLI.exe`) 付き。
- **`pdm-proto-wrapper`** — `pdm` の出力を protobuf でラップする C++ ライブラリ
  （`protobuf_launcher.cpp`、`semver.cpp` によるバージョン管理）。
- **`exefile`** — 最終実行ファイルをビルドするための構成要素（`Crashpad.cpp` に
  よるクラッシュレポート連携、`BlueInterface.cpp` で `blue` に接続）。
- **`exefileconsole`** — `exefile` プロセスを新規コンソールを起動せずにシェルから
  実行するラッパー（`ExeFileConsole.cpp`、実体は数百行程度の小さなツール）。
- **`trinityaudioapi`** — Carbon Trinity（レンダラ）と Carbon Audio 間で共有する
  オーディオインターフェース定義のみを集約した「単一の真実源」リポジトリ
  （`ITr2AudEmitter.h` 等、実装は持たない）。
- **`imageio`** — ビットマップ画像の読み書き・相互変換を行う基礎ライブラリ。
- **`localization`** — ローカライゼーションフレームワーク（文字列テーブル管理等）。
- **`d3dinfo`** — PC の DirectX 対応機能を検出する Python 露出ライブラリ。
- **`spacemouse`** — 3Dconnexion SpaceMouse 入力を Python に公開する小規模拡張。
- **`ime`** — Windows/macOS の IME（入力方式エディタ）機能を Python に公開する
  Carbon/Blue 拡張（キーボードレイアウト・変換ヘルパー）。
- **`imagetools`** — 画像データの読み込み・加工・圧縮を行う Python 拡張＋C++ ライブラリ。
- **`mesh`** — 3D メッシュ・スケルトン・アニメーションデータのシリアライズと基本的な
  アニメーションランタイムを提供（`cmfprocessor`/`viewer` ツール付属）。
- **`trinity`** — Carbon Game Engine のレンダリングエンジン本体。
- **`audio`** — Wwise SDK をラップした Carbon のオーディオコンポーネント（EVE Online /
  EVE Frontier 共通）。独自の音優先度システムを含み、Blue 経由で Python に露出。
- **`videoplayer`** — Carbon Trinity レンダリングエンジン上に実装された動画プレイヤー。
- **`spatial-audio-clustering`**（Apache-2.0、他は概ね MIT）— Wwise 用プラグイン。
  近接するオーディオオブジェクトを **K-Means 的手法**（`KMeansTest.h`,
  `SpatialClustering.cpp`）で動的にクラスタリングし、同時再生音声数を削減する。
  対象は音声オブジェクトの空間クラスタリングであり、`destiny` の空間分割
  （Partition.h の疎な階層 Box）とは別レイヤー・別アルゴリズム。
- **`red-to-black-converter`** — `.red` ファイルを `.black` ファイルへ変換するツール。
  README に「必要な依存関係が未オープンソース化」と明記されており、単体では
  ビルド・実行不可（オープンソース化が部分的であることを示す実例）。
- **`.github`** — Organization 共通の Issue/PR テンプレート等のメタリポジトリ
  （未クローン、コードなし）。

## まとめ

Carbon の実ソースを読んで得られた一致点は AU 定数と `Ballpark::Integrate` の指数減衰
積分器の2点のみであり、これらは確かに dawn の独自設計を裏付ける。しかし相違点の
大半（アンカー方式・固定グリッド・全シミュレート方針・smoothstep warp・疎結合
イベント・イベントソーシング・逐次 tick 処理）は、**dawn がまだ実戦で試されていない
だけ**という留保付きで読むべきであり、「dawn の方が優れている」という結論を急ぐ
べきではない。EVE の各設計は例外なく実際の負荷・実際のプレイヤー・実際の障害を
経て今の形になっており、dawn が同じ規模の実戦を経験するまでは、両者の優劣は
判定不能というのが誠実な結論である。

## dawn に取り入れるべき機能の検討（2026-07-04）

`destiny`/`pathfinder` の実ソースをさらに深掘りし、コード流用ではなく**機能・仕組みとして
輸入価値がある**候補を洗い出した。各項目は開発者判断つきで記録する（再検討時に同じ
調査を繰り返さないため）。

### 1. クローキング（`isCloaked` フラグ） — ゲームバランス要検討

`Ballpark.cpp` 全体で `isCloaked` という単純な bool フラグが、ミサイル追尾・Orbit・
編隊追従などあらゆるターゲティング系操作の入口でチェックされている
（3850, 3921, 3987, 4048 行目）。実装コスト自体は低い（ロック/追従系コマンドの
先頭に `if target.is_cloaked { reject }` を1行挟むだけ）。

**判断: 保留。** 文字通り「見えなくなる」機能はゲームバランスへの影響が大きく、
実装コストの低さとは別に導入の是非自体を先に検討する必要がある。

### 2. 誘導ミサイル（`DSTBALL_MISSILE`） — ゲームバランス + 性能要検討

`MissileFollow()`（Ballpark.cpp:3798-3873）: follow range を負値にして中心を狙う、
`mEffectStamp` で発射直後は直進させ誘導開始を遅らせる、対象が死亡済み/クローク中/
別バブルなら発射キャンセル、という設計。

**判断: 保留。** (1) タレット戦闘との駆け引き（回避運動・射線・距離管理）が誘導兵器の
導入で薄れる可能性があり、ゲームデザイン上の検討が必要。(2) 誘導計算（毎 tick の
追尾方向再計算・当たり判定）は瞬間ヒット式より確実に計算コストが重く、dawn の
tick 予算（ADR-0018）への影響を先に見積もる必要がある。

### 3. 残骸/デブリ物理（`DSTBALL_TROLL`） — 追加調査してから判断

`SetBallTroll()` → `TrollReady()` → `PetrifyTroll()`（Ballpark.cpp:6290-6330）:
一定時間 `delay` だけ自由落下させ、期限が来たら `SetBallFree(false)` で完全固定する
状態機械。船破壊時の残骸（wreck）演出に使える。

**判断: 保留。** 仕組み自体は単純だが、実装を完全に把握してから採用可否を判断する
（今回の調査は `Ballpark.cpp` の該当関数のみを読んだ限定的な確認）。

### 4. スマートボム型 AoE（`DSTBALL_MUSHROOM`） — 前提機能が未実装

`radius(t) = max_radius × (経過時間/持続時間)^0.25` という四分の一乗の膨張曲線
（Ballpark.cpp:569）で球状衝突判定を成長させ、`timeFraction >= 1.0` で自壊する。

**判断: 保留。** スマートボムは EVE では主にドローンとミサイルを破壊する対抗手段
だが、dawn にはドローンもミサイルもまだ実装されていない。対抗すべき対象が
存在しない状態でスマートボムだけ導入しても意味がない。2 のミサイル実装判断が
先に必要。

### 5. 編隊飛行（`DSTBALL_FORMATION`） — 不要

リーダー船が `mFormations` というスロットの配列を持ち、フォロワーはスロット番号を
1つ予約して追従する（Ballpark.cpp:3960-4001）。

**判断: 現時点では不要。**

### 6. ルート回避（`EveStandardFloodfillGoal`） — 将来機能・前提が未定

`pathfinder/EveStandardFloodfillGoal.h`: オートパイロット経路計算に、低セキュリティ帯
回避（無視/コスト加算/絶対禁止の3モード）・特定システム迂回・複数出発点・中間目標地点を
サポートするインターフェース（`IEvePathfinderGoal`）。

**判断: 将来向け機能として保留。** そもそも dawn がハイセキュリティ/ローセキュリティ
のようなセキュリティレーティング概念を持つ世界設計にするか自体が未定であり、
その前提が固まってから再検討する。
