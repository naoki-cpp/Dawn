---
scope    : 8D-5 Raspberry Pi 実機検証ハードウェアメモ
audience : Human Developer / AI Agent
status   : planning; hardware not purchased
related  : docs/architecture-review-server.md Phase 8 / docs/roadmap.md Phase 8D
date     : 2026-06-20
---

# 8D-5 ハードウェア検討メモ

8D-5 は `dawn-sector-node` を物理ノード上で動かすための実機検証である。
目的は「最高性能を測ること」ではない。TCP Raft + TCP replication の配線が、
小型で不安定になりやすい実機環境でも診断可能な形で動くかを確認する。

## 目的

3 台の物理 Sector Node を起動し、以下を観測する。

- 全 peer 間で TCP 到達性がある
- Raft leader election が成立する
- leader 切断後に failover する
- 実ネットワーク遅延下で event replication が破綻しない
- 低スペック環境で tick loop が極端に詰まらない
- 既存の観測ログだけで失敗原因を切り分けられる

成功条件は「失敗しないこと」ではなく「失敗しても説明できること」。
この段階では production-ready 判定まではしない。

## 現時点の推奨

初回は **Raspberry Pi Zero 2 W x3** で行う。理由は単純で、8D-5 の目的は
「安い実機で分散配線の弱点を見つけること」だからである。Zero 2 W は 1GHz
quad-core 64-bit Cortex-A53 / 512MB RAM / 2.4GHz WiFi という制約があり、
遅延・パケロス・弱い CPU・電源問題が表面化しやすい。これは 8D-5 の目的に合う。

ルーターも買わない。最初は以下の優先順で試す。

1. **既存PCの WiFi hotspot**: 追加購入なしで一番成功確度が高い。
2. **ESP32 SoftAP**: ルーター代替の最安追加ハード候補。ただし station 間 TCP が
   安定して通るかを先に smoke test する。
3. **Zero 2 W の1台を AP 化**: 追加ハードなし。ただし AP 役の node を落とすと
   全ネットワークが落ちるため、leader-failover 検証には不向き。

Pi 5 + 有線 switch は診断しやすいが、今回の「最安で始める」方針から外す。
ClusterHAT も初回では保留する。

## 候補比較

| 構成 | Network | 利点 | リスク | 判定 |
|---|---|---|---|---|
| 3x Zero 2 W + 既存PC hotspot | WiFi | 追加 network 機材なし。最も簡単 | PC を常時起動する必要あり | 最初に試す |
| 3x Zero 2 W + ESP32 SoftAP | WiFi SoftAP | ルーター不要。追加費用が小さい | ESP32 firmware 準備が必要。station 間 TCP を要確認 | 最安独立構成候補 |
| 3x Zero 2 W / node-0 AP | WiFi SoftAP | 追加ハードなし | AP node を落とすと全体が落ちる | smoke test 限定 |
| 3x Zero 2 W + USB Ethernet | USB Ethernet | Zero 2 W でも有線化できる | adapter / OTG / 給電が面倒 | WiFi がダメなら |
| 3x Pi 5 + switch | native Gigabit Ethernet | 最も診断しやすい | コスト高 | 後回し |
| ClusterHAT | USB gadget network | 配線がまとまる | 実 LAN とは違う。入手性確認が必要 | 後回し |

## 買い物リスト

### 最小構成 A: 既存PC hotspot を使う

追加 network 機材を買わない。まずこれで試す。

必須:

- Raspberry Pi Zero 2 W x3
- microSD 32GB 以上 x3
- 5V micro USB 電源 x3

あると楽:

- mini HDMI adapter x1
- USB OTG adapter x1
- keyboard x1
- Pi Zero 2 W case x3

既存PC側:

- Windows mobile hotspot / macOS Internet Sharing / Linux hotspot のいずれか
- 3台の Pi に固定 IP を振るか、DHCP lease を控える

### 最小構成 B: ESP32 SoftAP を使う

PC hotspot を使いたくない場合の最安独立構成。ESP32 は router というより
「簡易AP」として使う。Internet 接続は不要。

必須:

- Raspberry Pi Zero 2 W x3
- microSD 32GB 以上 x3
- 5V micro USB 電源 x3
- ESP32 dev board x1
- ESP32 用 USB cable x1

注意:

- ESP32 側に SoftAP firmware が必要。
- 3台の Pi が同じ ESP32 AP に接続できることを確認する。
- Pi 同士で `ping` と TCP 接続が通ることを確認してから Dawn を動かす。
- ESP32 の AP mode は Espressif 公式 docs 上も「stations connect to the ESP32」
  という位置づけなので、Dawn の peer-to-peer TCP で使えるかは実測で判断する。

### 追加購入を避けるために後回し

- WiFi router
- Ethernet switch
- USB Ethernet adapter
- ClusterHAT
- Raspberry Pi 5

## Network Plan

IP は固定する。DHCP でも smoke test はできるが、ログ照合と config 管理が面倒になる。
ESP32 SoftAP / PC hotspot のどちらでも、最終的に Pi 3台が同じ IPv4 subnet に
入っていればよい。

例:

| Node | Sector | IP | Config |
|---|---:|---|---|
| node-0 | 0 | `192.168.10.10` | `crates/dawn-sector-node/config/node-0.toml` |
| node-1 | 1 | `192.168.10.11` | `crates/dawn-sector-node/config/node-1.toml` |
| node-2 | 2 | `192.168.10.12` | `crates/dawn-sector-node/config/node-2.toml` |

起動前に疎通を確認する。

```bash
ping 192.168.10.10
ping 192.168.10.11
ping 192.168.10.12
```

ESP32 / hotspot 構成で最初に確認すること:

```bash
# node-0 から
ping 192.168.10.11
ping 192.168.10.12

# node-1 から
ping 192.168.10.10
ping 192.168.10.12
```

ping が通らない場合、Dawn はまだ起動しない。AP 側の client isolation や subnet を疑う。

各 Pi で併せて採取する。

```bash
ip addr
ip route
uname -a
vcgencmd measure_temp
```

## Build And Run

初回は各 Pi 上で build してよい。遅いが、cross compile の問題と実行時問題を混ぜずに済む。

各 node で config を変えて実行する。

```bash
cargo build -p dawn-sector-node --release
RUST_LOG=info ./target/release/sector-node crates/dawn-sector-node/config/node-0.toml 2>node-0.err.log
```

node-1 / node-2 では対応する config とログ名を使う。
stdout と stderr は両方保存する。8D-5 向け観測ログは主に stderr に出る。

## 観測するログ

すでに仕込んである観測点:

- Raft role transition: `dawn-consensus/src/actor.rs`
- TCP reconnect / connection loss: `dawn-consensus/src/tcp_transport.rs`
- tick overrun: `crates/dawn-sector-node/src/main.rs`

共有してほしい成果物:

- `node-0.err.log`, `node-1.err.log`, `node-2.err.log`
- 各 node の config
- hardware model と OS version
- network 種別（WiFi / USB Ethernet / native Ethernet）
- 手動で network を切断・復帰した時刻

## 検証手順

1. 3 node を起動する。
2. leader が 1 つだけ選出されるまで待つ。
3. Pi 同士の `ping` が通ることを確認する。
4. 60 秒ほど通常 tick を観測する。
5. 現在の leader の network を切断する。
6. 残り 2 node で新 leader が選出されることを確認する。
7. 旧 leader の network を復帰する。
8. 旧 leader が crash / reconnect storm / split leadership なしに戻ることを確認する。
9. さらに 5 分ほど放置する。

## 合格ライン

- 起動後、leader がちょうど 1 つになる
- leader 切断後、残存 node が新 leader を選ぶ
- 旧 leader 復帰後、leader が複数に固定されない
- tick overrun が少なく、発生タイミングを説明できる
- TCP reconnect log が手動 fault injection と対応する
- node process が予期せず終了しない

## 失敗パターンと次の一手

| 症状 | 疑う場所 | 次の一手 |
|---|---|---|
| fault injection なしで leader が頻繁に変わる | network jitter / election timeout | TCP reconnect log を見る。有線で再試験 |
| 1 node だけ join しない | config / IP / port mismatch | config と `ip addr` を照合 |
| 全 node で tick overrun | CPU 不足 / blocking I/O | ship count を下げる。`top` と温度を採取 |
| reconnect 時だけ tick overrun | transport 処理が simulation を圧迫 | sector-node main loop 周辺を確認 |
| process が落ちる | panic / resource exhaustion | `RUST_BACKTRACE=1` で再実行 |

## 初回ではやらないこと

- TLS
- 認証
- Internet 公開
- NAT traversal
- 長時間の永続化耐久試験
- Pi model 間の性能比較
- 有線化

これらは別タスク。8D-5 はまず物理 network sanity check に絞る。

## 参照

- Raspberry Pi Zero 2 W official specs:
  https://www.raspberrypi.com/products/raspberry-pi-zero-2-w/
- Raspberry Pi 5 official specs:
  https://www.raspberrypi.com/products/raspberry-pi-5/
- ClusterHAT project page:
  https://clusterhat.com/
- ESP-IDF Wi-Fi docs:
  https://docs.espressif.com/projects/esp-idf/en/latest/esp32/api-reference/network/esp_wifi.html
