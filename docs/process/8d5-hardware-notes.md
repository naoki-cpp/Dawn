---
scope    : 8D-5 Raspberry Pi cluster hardware validation — reproducible procedure and result
audience : Human Developer / AI Agent
status   : done; 3 checks (reachability / tick-sla / failover) PASS on 2026-07-01
related  : docs/process/roadmap.md "8D. 分散インフラ" item 5, scripts/verify-pi-cluster.sh
date     : 2026-07-01
---

# 8D-5 Hardware Validation

Validates that the TCP Raft + TCP replication wiring works, and fails in a
diagnosable way, on small/unstable physical hardware. Not a performance
benchmark and not a production-readiness gate.

## Hardware used

3x Raspberry Pi Zero W, hostnames `node-0.local` / `node-1.local` /
`node-2.local`, user `dawn`, joined to a PC-hosted Wi-Fi hotspot
(`192.168.137.0/24` in the run below; any shared subnet works).

| Node | Sector | Config |
|---|---:|---|
| node-0 | 0 | `crates/dawn-server/config/node-0.toml` |
| node-1 | 1 | `crates/dawn-server/config/node-1.toml` |
| node-2 | 2 | `crates/dawn-server/config/node-2.toml` |

## Procedure

### 1. Flash and provision the Pis (first time only)

- Image: `Raspberry Pi OS Lite (32-bit)`, written with Raspberry Pi Imager.
- In Imager's `Edit Settings`, set hostnames `node-0.local` / `node-1.local` /
  `node-2.local`, the same username (`dawn`)/password and Wi-Fi SSID/password
  on all three cards, `Enable SSH = ON` (password auth for first boot).
- If `ssh dawn@node-0.local` returns `Connection refused` after boot, the
  Imager's `Enable SSH` setting did not take — reinsert the card and add an
  empty file named exactly `ssh` (no extension) to the boot partition root.
- First login, then update each Pi:
  ```bash
  ssh dawn@node-0.local
  sudo apt update && sudo apt upgrade -y
  ```
- Install the deploy SSH key from the PC (creates a dedicated key, appends it
  to each Pi's `authorized_keys`, updates `~/.ssh/config`):
  ```bash
  scripts/setup-pi-cluster.sh --ssh
  ```

### 2. Build and deploy

Build on the host PC and deploy only the runtime artifact + config/data — do
not clone the repo onto each Pi.

```bash
scripts/setup-pi-cluster.sh --host-tools   # repo-local cargo-zigbuild env, once
scripts/deploy-pi-cluster.sh --no-progress
scripts/run-pi-cluster.sh --replace
```

`deploy-pi-cluster.sh` builds `sector-node` for `arm-unknown-linux-gnueabihf`
if the artifact isn't already present, detects each node's current `wlan0`
IPv4, rewrites the three `node-*.toml` peer addresses to match, and ships the
binary + `data/*.toml` + configs to each Pi over scp. `run-pi-cluster.sh
--replace` restarts any previously running `sector-node` on each node.

### 3. Verify

```bash
scripts/verify-pi-cluster.sh reachability
scripts/verify-pi-cluster.sh tick-sla --window-seconds 90 --max-overrun-rate 0.01
scripts/verify-pi-cluster.sh failover --timeout-seconds 15
scripts/verify-pi-cluster.sh restart-recovery --restart-wait-seconds 10
```

`reachability`, `tick-sla`, and `restart-recovery` are fully automated against
the running cluster. `failover` watches the Raft role-transition logs for a
new leader; the network partition itself must still be induced manually on
the current leader, e.g.:

```bash
# on the leader node, immediately before running `verify-pi-cluster.sh failover`
sudo ip link set wlan0 down; sleep 12; sudo ip link set wlan0 up
```

(`iptables`/`nft` were not present on the Raspberry Pi OS Lite image used
here, so dropping the link is the simplest available fault injection.)

`tick-sla`'s window should span at least one checkpoint interval (default
`checkpoint_interval_ticks=600` = 60s) so a slow SD-card snapshot write would
actually show up as an overrun — hence 90s rather than 60s above.
`restart-recovery` (added 2026-07-01 alongside production persistence wiring)
kills and restarts `node-0`'s `sector-node` process and checks its log for
`restoring from snapshot` with a nonzero tick, instead of starting over from
genesis.

## Result (2026-07-01)

| Check | Result |
|---|---|
| `reachability` | PASS — all 3 nodes ssh-reachable, `sector-node` running, ws/raft/repl ports listening |
| `tick-sla` (60s window) | PASS — 0 overruns / ~600 ticks on all 3 nodes (rate 0.0000) |
| `failover` (node-2, the leader, partitioned for 12s) | PASS — node-0 transitioned to Leader within 15s; after node-2 rejoined, all nodes converged on a single leader (node-0, term 11), no split leadership |
| `tick-sla` (90s window, spans a checkpoint) | PASS — 0 overruns / ~900 ticks on all 3 nodes (rate 0.0000); checkpoint snapshot writes did not blow the tick budget |
| `restart-recovery` (node-0, after production persistence wiring) | PASS — killed and restarted `sector-node`; log showed `restoring from snapshot (tick=9631, log_index=0)`, ticks continued instead of resetting to genesis |

### Bugs found and fixed while running this

- `scripts/deploy-pi-cluster.sh`: `render_progress` / `finish_progress`
  returned the falsy status of their own guard condition on early exit, which
  killed the whole script under `set -e` whenever progress rendering was
  disabled (`--no-progress`, or any non-tty invocation). Fixed by returning
  `0` explicitly.
- `scripts/verify-pi-cluster.sh`: the `tick-sla` total-tick estimate collapsed
  to `1` whenever the log file had no new overrun lines (the normal/healthy
  case), making the overrun rate meaningless. Fixed to use a fixed
  `window_seconds * 1000 / 100` estimate.
- `scripts/verify-pi-cluster.sh`: `restart-recovery`'s `pkill` was passed as
  an inline ssh command-string argument. The remote shell invoking it has
  `pkill -f 'target/release/sector-node' ...` in its own argv, which is
  exactly the pattern being matched — `pkill -f` killed its own invoking
  shell before reaching the real target, severing the SSH channel (ssh exits
  255, `exit-signal`). Fixed by moving the kill into the same heredoc script
  as the restart (mirroring `run-pi-cluster.sh`, whose remote process is
  just `bash -s` with no self-matching substring in its argv).
- `scripts/verify-pi-cluster.sh`: `restart-recovery` initially read
  `restoring from snapshot` / `no snapshot at` from `.err.log`; both are
  `println!` (stdout), so they land in `.out.log`. The check silently never
  matched until this was fixed.

### Non-bug observation

On startup, `[RaftTransport] ... connect failed (attempt #N)` logs dozens of
times while peers are still coming up, then goes silent once the reconnect
succeeds (successful reconnects aren't logged). This looks like a stuck log
but isn't — confirmed live connectivity with `nc -zv` during one such run.

## Failure patterns and next steps

| Symptom | Suspect | Next step |
|---|---|---|
| Leader changes repeatedly with no fault injection | network jitter / election timeout | check TCP reconnect log; retest over wired Ethernet |
| One node never joins | config / IP / port mismatch | diff its config against `ip addr` on that node |
| Tick overrun on every node | CPU starvation / blocking I/O | lower ship count; capture `top` and `vcgencmd measure_temp` |
| Tick overrun only around reconnect | transport work blocking the simulation loop | inspect the `sector-node` main loop around the replication/Raft drain |
| Process exits | panic / resource exhaustion | rerun with `RUST_BACKTRACE=1` |

## Out of scope for this pass

TLS, auth, public internet exposure, NAT traversal, long-duration durability
soak tests, cross-Pi-model performance comparison, wired Ethernet. These are
separate tasks — 8D-5 is a physical network sanity check only.

## References

- https://www.raspberrypi.com/products/raspberry-pi-zero-w/
- https://www.raspberrypi.com/software/operating-systems/
- https://www.raspberrypi.com/software/
