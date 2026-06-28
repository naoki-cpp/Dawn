---
scope    : 8D-5 Raspberry Pi 実機検証ハードウェアメモ
audience : Human Developer / AI Agent
status   : in_progress; Raspberry Pi Zero W x3 deployed from host PC
related  : docs/architecture/architecture-review-server.md Phase 8 / docs/process/roadmap.md Phase 8D
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

- Raspberry Pi Zero W official product page:
  https://www.raspberrypi.com/products/raspberry-pi-zero-w/
- Raspberry Pi OS official page:
  https://www.raspberrypi.com/software/operating-systems/
- Raspberry Pi Imager official page:
  https://www.raspberrypi.com/software/

## Raspberry Pi Zero W initial setup notes

This section records the first hands-on setup path for the currently available
hardware: **Raspberry Pi Zero W x3** with an existing PC used as the Wi-Fi
hotspot.

### OS choice

Use:

```text
Raspberry Pi OS Lite (32-bit)
```

Do not enable Raspberry Pi Connect for this setup. SSH is enough and has lower
overhead.

### PC hotspot setup

Use the existing PC as the first network access point.

Windows:

```text
Settings -> Network & internet -> Mobile hotspot
```

Recommended values:

```text
SSID      = DawnLab
Band      = 2.4 GHz, if selectable
Password  = any WPA2 password with 8+ characters
```

### Flash the OS with Raspberry Pi Imager

Create one microSD card per node.

For each card, select:

```text
Raspberry Pi Device = Raspberry Pi Zero
Operating System    = Raspberry Pi OS (other) -> Raspberry Pi OS Lite (32-bit)
Storage             = the target microSD card
```

Open `Edit Settings` before writing the image.

Set hostnames:

```text
node-0.local
node-1.local
node-2.local
```

Set the same user, password, Wi-Fi SSID, and Wi-Fi password on all three nodes.
For the current 8D-5 hardware setup, use `dawn` as the Linux username.

Recommended Imager options:

```text
Configure wireless LAN = ON
Wireless LAN country   = JP
Time zone              = Asia/Tokyo
Keyboard layout        = jp, or the developer's actual keyboard layout
Username               = dawn
Enable SSH             = ON
SSH authentication     = password authentication for the first setup
Enable Raspberry Pi Connect = OFF
```

After writing and verification finish, insert each microSD card into the
matching Pi and power the nodes on.

Observed on the current setup:

- Even with `Enable SSH = ON` in Raspberry Pi Imager, SSH did not start on the
  first boot until an empty `ssh` file was placed in the boot partition root.
- If `ssh dawn@node-0.local` returns `Connection refused`, reinsert the card
  into the PC and create this file in the boot partition:

```text
ssh
```

- The file name must be exactly `ssh` with no extension.

### First SSH login

Wait a few minutes for the first boot, then connect from the PC:

```bash
ssh dawn@node-0.local
ssh dawn@node-1.local
ssh dawn@node-2.local
```

Update the OS:

```bash
sudo apt update
sudo apt upgrade -y
```

### SSH key automation

After the first password-based login is confirmed, the SSH key setup can be
automated from the PC with:

```bash
scripts/setup-pi-cluster.sh --ssh
```

This script creates a dedicated key, appends the public key to each Pi's
`authorized_keys`, and updates `~/.ssh/config` on the PC for:

```text
node-0.local
node-1.local
node-2.local
```

Before installing the key, the script now checks that each target reports the
expected hostname, and that hostname/IP/machine-id are not duplicated across
the three nodes.

Observed on the current setup:

- Existing `~/.ssh/config` content may not end with a newline. The script now
  guards against that before appending new `Host` blocks.
- The first run still prompts for each Pi password once while the public key is
  being installed.

### Preferred deploy path

For the current private-repo workflow, prefer deploying from the PC instead of
cloning the repository separately on each Pi.

The preferred runtime model is:

- build `sector-node` on the host PC
- deploy only the runtime artifact plus config/data files to each Pi
- run the same prebuilt binary on all three nodes

This path has been exercised on the current setup.

Run from the PC:

```bash
scripts/deploy-pi-cluster.sh
scripts/deploy-pi-cluster.sh --artifact-path /absolute/path/to/sector-node
scripts/deploy-pi-cluster.sh --no-progress
scripts/run-pi-cluster.sh
```

`scripts/deploy-pi-cluster.sh` now does the host-side preparation itself:

- creates or reuses the repo-local tool environment with `scripts/setup-pi-cluster.sh --host-tools`
- exports the local tool bin directory into `PATH`
- exports `CARGO_ZIGBUILD_PYTHON_PATH`
- runs `rustup target add arm-unknown-linux-gnueabihf`
- builds the host artifact first if it does not already exist
- skips the build if the host artifact is already present
- `--build-host` forces a rebuild before deployment
- connects to the Pi nodes only after the artifact is ready

It then:

- connects to `node-0.local`, `node-1.local`, and `node-2.local`
- detects each node's current IPv4 address on `wlan0`
- rewrites `crates/dawn-sector-node/config/node-0.toml`,
  `node-1.toml`, and `node-2.toml` inside the deployment bundle
- can use the repo-local `.tools/python/cargo-zigbuild/` virtual environment
  for host-side cross-compilation without touching the global Python env
- packages only the runtime files needed on the Pi:
  `target/release/sector-node`, `crates/dawn-sector-node/config/node-*.toml`,
  `data/galaxy.toml`, and the optional `data/modules.toml` /
  `data/ship_types.toml`
- copies that runtime bundle to each Pi
- preserves each Pi's `logs/` directory across deploys
- shows a per-node progress bar for detect/copy/expand/done
- `--no-progress` disables terminal redraw if the terminal does not render it cleanly
- aborts early if two node hostnames resolve to the same IPv4 address

The generated cluster configs use:

```toml
npc_ships = 0
pop_cap   = 50
```

The default artifact path is:

```text
target/arm-unknown-linux-gnueabihf/release/sector-node
```

For host-side cross-compilation, the recommended setup is:

```bash
scripts/setup-pi-cluster.sh --host-tools
export PATH="$(scripts/setup-pi-cluster.sh --print-tool-bin):$PATH"
export CARGO_ZIGBUILD_PYTHON_PATH="$(scripts/setup-pi-cluster.sh --print-python)"
rustup target add arm-unknown-linux-gnueabihf
```

This keeps the global Python environment untouched and pins the helper tools
inside the repository workspace.

If the one-shot path is preferred, use:

```bash
scripts/deploy-pi-cluster.sh
```

If the artifact is built elsewhere, pass it with `--artifact-path`.

### Manual path

If the deploy script is not being used, clone or copy Dawn onto each Pi and
edit the node configs by hand.

On each Pi:

```bash
git clone <Dawn repo URL>
cd Dawn
```

### Dawn node config for real hardware

The checked-in configs use `127.0.0.1` for local three-process testing. For
three physical Pis, keep each node's bind addresses as `0.0.0.0`, and replace
only the peer addresses with the actual Pi IP addresses.

Example `node-0.toml` peer section:

```toml
[[peers]]
node_id   = 1
raft_addr = "192.168.137.11:7901"
repl_addr = "192.168.137.11:7911"
ws_addr   = "192.168.137.11:7879"

[[peers]]
node_id   = 2
raft_addr = "192.168.137.12:7902"
repl_addr = "192.168.137.12:7912"
ws_addr   = "192.168.137.12:7880"
```

Apply the same rule to `node-1.toml` and `node-2.toml`: each file should point
at the other two physical nodes.

### Build and run

The preferred start command from the PC is:

```bash
scripts/run-pi-cluster.sh
```

Use this to restart a previous run:

```bash
scripts/run-pi-cluster.sh --replace
```

### Client connection

The Godot client can connect to the physical cluster by writing the entry
WebSocket URL to `client/server_url.txt` before launch.
Use one node as the entry point, for example:

```bash
printf '%s\n' 'ws://node-0.local:7878' > client/server_url.txt
```

If `.local` resolution is not available on the client machine, use the node's
hotspot IP instead:

```bash
printf '%s\n' 'ws://192.168.137.xxx:7878' > client/server_url.txt
```

The client now accepts the server's `Redirect` message and reconnects to the
destination node automatically on inter-sector jumps.

Each node writes logs under:

```text
~/Dawn/logs/node-0.out.log
~/Dawn/logs/node-0.err.log
~/Dawn/logs/node-1.out.log
~/Dawn/logs/node-1.err.log
~/Dawn/logs/node-2.out.log
~/Dawn/logs/node-2.err.log
```

If the run script is not being used, start the nodes by hand on each Pi.

The deployed bundle on each Pi contains the prebuilt binary plus config/data.
If starting by hand:

```bash
cd ~/Dawn
```

Run one node per Pi:

```bash
RUST_LOG=info ./target/release/sector-node crates/dawn-sector-node/config/node-0.toml 2>node-0.err.log
RUST_LOG=info ./target/release/sector-node crates/dawn-sector-node/config/node-1.toml 2>node-1.err.log
RUST_LOG=info ./target/release/sector-node crates/dawn-sector-node/config/node-2.toml 2>node-2.err.log
```

Use the matching command on the matching physical node.
