---
scope    : 8D-5（Raspberry Pi 実機検証）に向けたハードウェア検討メモ
audience : Human Developer
status   : 検討中・未購入（決定ではない）
related  : docs/architecture-review.md Phase 8 / Phase 9
date     : 2026-06-20
---

# 8D-5 ハードウェア検討メモ

3 物理ノードクラスタ（`dawn-sector-node` を3台で実行）を Raspberry Pi で組む際の
通信方式の検討。**結論は出ていない。購入前の調査段階。**

---

## 候補ボード: Raspberry Pi Zero 2 W

- WiFi 内蔵（802.11 b/g/n）。Ethernet は非搭載
- クアッドコア 64bit（Cortex-A53）。Rust の tokio + Raft + ECS tick を3並列で
  動かすには無印 Pi Zero（armv6 シングルコア）より十分な性能
- 無印 Pi Zero / Zero 1.3 は WiFi も Ethernet も非搭載のため除外

---

## 通信方式の選択肢

`TcpRaftTransport` / `TcpReplicationTransport` は `TcpStream` / `SocketAddr` ベースの
実装（8D-3）であるため、**通常の IP ネットワークが前提**。UART / I2C は
ポイントツーポイントの低速シリアル接続でこの設計に合わず、再実装が必要になるため不採用。

### 案A: WiFi（最も簡単）

3台を同一ルーターに接続するだけ。config の `peers` に各 IP を書くだけで動く。
配線不要だが WiFi の遅延・パケロスの影響を受ける。

### 案B: ClusterHAT

USB OTG 経由でコントローラー Pi（3B+/4 など）に接続し、コントローラーが
USB Ethernet gadget mode で各 Zero に IP を配布する仕組み。
基板上の有線通信が実現できるが、**日本国内からの入手性が悪く保留**。

### 案C: 自前で USB ガジェットモードを設定（ClusterHAT の内部動作を再現）

ClusterHAT の基板を使わず、Pi 4（コントローラー、USB-A ×4）+ Pi Zero 2 W ×3 +
USB-C to USB-A ケーブルのみで同じ構成を作る。
各 Zero の `/boot/config.txt` に `dtoverlay=dwc2`、`cmdline.txt` に
`modules-load=dwc2,g_ether` を追記するだけで HAT 基板は不要。
ケーブル・本体は国内代理店（スイッチサイエンス / KSY / RS コンポーネンツ等）で購入可能。

### 案D: USB-Ethernet ドングル + 通常スイッチ

各 Zero に USB-Ethernet ドングルを挿し、市販の Ethernet スイッチ
（バッファロー / エレコム等、国内で容易に入手可能）に接続。
ガジェットモードの設定が不要で、挿せば普通の LAN として認識されるため
トラブルシュートが容易。Zero 2 W の USB ポートは1つしかないため、
給電とデータ通信を両立するには Y字ケーブルや別経路の給電が必要な点に注意。

---

## 比較

| 案 | 配線 | 設定の複雑さ | 入手性（日本） |
|---|---|---|---|
| A: WiFi | なし | 最小 | ◎（既存ルーターのみ） |
| B: ClusterHAT | USB OTG | 中 | ✗ 入手困難 |
| C: 自前 USB gadget | USB OTG | 中（dwc2/g_ether 設定） | ◎ |
| D: USB-Ethernet + スイッチ | 有線 | 最小（標準Ethernet） | ◎ |

**現時点の所感**: 配線を増やしたくなければ案A、確実な有線接続を求めるなら案D が
トラブルシュートしやすく現実的。案C は配線をケーブル1本に抑えたい場合の選択肢。

---

## 次のアクション

ハードウェア未購入のため、まずは localhost 上で3プロセスを起動し
（`config/node-{0,1,2}.toml` を使用）、TCP 配線・Raft・replication の動作を
ソフトウェア面で先に検証する（実機購入前の前倒し検証）。→ 完了（2026-06-20）。

### 仕込んだ観測ログ（実機検証で症状を確定させるため）

実機の WiFi/USB 経由通信は localhost より遅延・パケロスが発生しやすい。
「どこに無理が出るか」を確定するには、Claude Code 側からは直接観測できないため、
実機で動かして得られたログをユーザーから共有してもらう前提で、以下を仕込んだ:

| ログ | 場所 | 検出する症状 |
|---|---|---|
| Raft role 遷移（Follower↔Candidate↔Leader） | `dawn-consensus/src/actor.rs` の `run()` | リーダー再選出の頻発 |
| TCP 再接続/接続失敗 | `dawn-consensus/src/tcp_transport.rs` の `outbound_loop` | WiFi/USB リンクの不安定さ |
| tick 処理時間オーバーラン | `dawn-sector-node/src/main.rs` の tick ループ | I/O がシミュレーションをブロックしていないか |

実機検証時はこれらの標準エラー出力をログファイルに保存し、症状（再起動・固まり等）が
出たタイミングと突き合わせて分析する。
