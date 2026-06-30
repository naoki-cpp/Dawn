#!/usr/bin/env bash
# verify-pi-cluster.sh
#
# Runs the 8D-5 Raspberry Pi cluster validation checklist (roadmap.md "8D.
# 分散インフラ" item 5) against a cluster already deployed and started via
# deploy-pi-cluster.sh / run-pi-cluster.sh. Each check has an explicit
# pass/fail criterion so a single run produces a verdict, not just logs.
#
# Checks:
#   reachability      - all 3 nodes are up and listening on ws/raft/repl ports
#   tick-sla          - "[Node] tick overrun" rate in logs stays under threshold.
#                        Pass --window-seconds long enough to span at least one
#                        checkpoint (default checkpoint_interval_ticks=600 *
#                        100ms = 60s) so a slow SD-card snapshot write would
#                        actually show up as an overrun -- e.g. --window-seconds 90.
#   failover          - after the operator partitions the current Raft leader,
#                        a new leader is elected within the expected window
#   restart-recovery  - kill and restart one node's sector-node process, and
#                        confirm it logs "restoring from snapshot" with a
#                        nonzero tick instead of starting over from genesis
#                        (production persistence wiring, 2026-07-01)
#
# Usage:
#   scripts/verify-pi-cluster.sh reachability
#   scripts/verify-pi-cluster.sh tick-sla [--window-seconds 90] [--max-overrun-rate 0.01]
#   scripts/verify-pi-cluster.sh failover [--timeout-seconds 15]
#   scripts/verify-pi-cluster.sh restart-recovery [--restart-wait-seconds 10]
#   scripts/verify-pi-cluster.sh all
set -euo pipefail

nodes=("node-0.local" "node-1.local" "node-2.local")
custom_nodes=0
user="dawn"
remote_repo_path="/home/dawn/Dawn"
window_seconds=60
max_overrun_rate="0.01"
failover_timeout_seconds=15
restart_wait_seconds=10
command_name=""

ports_for_node() {
	# node-0 -> ws 7878 raft 7900 repl 7910, node-1 -> 7879/7901/7911, etc.
	local idx="$1"
	echo "$((7878 + idx)) $((7900 + idx)) $((7910 + idx))"
}

while [[ $# -gt 0 ]]; do
	case "$1" in
		reachability|tick-sla|failover|restart-recovery|all)
			command_name="$1"
			shift
			;;
		--node)
			if [[ $custom_nodes -eq 0 ]]; then
				nodes=()
				custom_nodes=1
			fi
			nodes+=("$2")
			shift 2
			;;
		--nodes)
			IFS=',' read -r -a nodes <<< "$2"
			custom_nodes=1
			shift 2
			;;
		--user)
			user="$2"
			shift 2
			;;
		--remote-repo-path)
			remote_repo_path="$2"
			shift 2
			;;
		--window-seconds)
			window_seconds="$2"
			shift 2
			;;
		--max-overrun-rate)
			max_overrun_rate="$2"
			shift 2
			;;
		--timeout-seconds)
			failover_timeout_seconds="$2"
			shift 2
			;;
		--restart-wait-seconds)
			restart_wait_seconds="$2"
			shift 2
			;;
		*)
			echo "Unknown argument: $1" >&2
			exit 1
			;;
	esac
done

if [[ -z "$command_name" ]]; then
	echo "Usage: scripts/verify-pi-cluster.sh {reachability|tick-sla|failover|all} [options]" >&2
	exit 1
fi

if [[ ${#nodes[@]} -ne 3 ]]; then
	echo "This script expects exactly 3 nodes." >&2
	exit 1
fi

configs=("node-0" "node-1" "node-2")

check_reachability() {
	local failed=0
	for idx in "${!nodes[@]}"; do
		local node="${nodes[$idx]}"
		local remote="${user}@${node}"
		read -r ws_port raft_port repl_port <<< "$(ports_for_node "$idx")"

		if ! ssh -o ConnectTimeout=5 "$remote" true; then
			echo "FAIL  $node: ssh unreachable"
			failed=1
			continue
		fi

		if ! ssh "$remote" "pgrep -u \"\$USER\" -f 'target/release/sector-node' >/dev/null"; then
			echo "FAIL  $node: sector-node process not running"
			failed=1
			continue
		fi

		local missing=""
		for port in "$ws_port" "$raft_port" "$repl_port"; do
			if ! ssh "$remote" "ss -ltn | awk '{print \$4}' | grep -q \":${port}\$\""; then
				missing="$missing $port"
			fi
		done

		if [[ -n "$missing" ]]; then
			echo "FAIL  $node: not listening on$missing"
			failed=1
		else
			echo "PASS  $node: reachable, sector-node running, ws/raft/repl ports open"
		fi
	done
	return $failed
}

check_tick_sla() {
	echo "Observing tick overrun rate for ${window_seconds}s (threshold: overrun rate < ${max_overrun_rate})..."
	local failed=0
	for idx in "${!nodes[@]}"; do
		local node="${nodes[$idx]}"
		local config="${configs[$idx]}"
		local remote="${user}@${node}"
		local log_path="${remote_repo_path}/logs/${config}.err.log"

		local before
		before="$(ssh "$remote" "wc -l < '$log_path' 2>/dev/null || echo 0")"
		sleep "$window_seconds"
		local after
		after="$(ssh "$remote" "wc -l < '$log_path' 2>/dev/null || echo 0")"

		local overruns
		overruns="$(ssh "$remote" "tail -n +$((before + 1)) '$log_path' 2>/dev/null | grep -c '\[Node\] tick overrun' || true")"
		# TICK_MS is fixed at 100ms (dawn-sector-node/src/main.rs); the tick budget
		# does not depend on whether any overrun lines were logged this window.
		local total_ticks=$((window_seconds * 1000 / 100))
		local rate
		rate="$(awk -v o="$overruns" -v t="$total_ticks" 'BEGIN { printf "%.4f", o / t }')"

		if awk -v r="$rate" -v m="$max_overrun_rate" 'BEGIN { exit !(r > m) }'; then
			echo "FAIL  $node: $overruns overruns / ~$total_ticks ticks (rate=$rate > $max_overrun_rate)"
			failed=1
		else
			echo "PASS  $node: $overruns overruns / ~$total_ticks ticks (rate=$rate <= $max_overrun_rate)"
		fi
	done
	return $failed
}

check_failover() {
	echo "Tail the Raft role-transition logs across all nodes, then manually partition"
	echo "the current leader (e.g. 'sudo iptables -A INPUT -p tcp --dport <raft_port> -j DROP'"
	echo "on that node, or unplug its network link)."
	echo
	echo "Watching for a new leader election within ${failover_timeout_seconds}s..."

	local deadline=$(( $(date +%s) + failover_timeout_seconds ))
	local elected=0
	while [[ "$(date +%s)" -lt "$deadline" ]]; do
		for idx in "${!nodes[@]}"; do
			local node="${nodes[$idx]}"
			local config="${configs[$idx]}"
			local remote="${user}@${node}"
			local log_path="${remote_repo_path}/logs/${config}.err.log"

			if ssh "$remote" "tail -n 20 '$log_path' 2>/dev/null | grep -q '→ Leader'"; then
				echo "PASS  $node observed a role transition to Leader within ${failover_timeout_seconds}s"
				elected=1
				break 2
			fi
		done
		sleep 1
	done

	if [[ $elected -eq 0 ]]; then
		echo "FAIL  no node logged a transition to Leader within ${failover_timeout_seconds}s"
		return 1
	fi
	return 0
}

check_restart_recovery() {
	# Single-node check: persistence is per-node, not a cluster property.
	# Startup/checkpoint lines are println! (stdout -> .out.log), unlike the
	# eprintln! diagnostics (tick overrun, Raft role transitions) the other
	# checks read from .err.log.
	local node="${nodes[0]}"
	local config="${configs[0]}"
	local remote="${user}@${node}"
	local log_path="${remote_repo_path}/logs/${config}.out.log"

	echo "Killing and restarting $node ($config) to verify snapshot recovery..."

	# pkill + restart in one heredoc script (not an inline ssh argument
	# string): when 'pkill -f target/release/sector-node' is passed as an
	# inline ssh command argument, the invoking remote shell's own argv
	# contains that same pattern and pkill kills its own shell, severing the
	# SSH channel (ssh exits 255, "exit-signal", before pkill even reaches
	# the real target). A heredoc's remote process is just `bash -s`, with
	# no self-matching substring -- the same reason run-pi-cluster.sh
	# already does it this way.
	ssh "$remote" \
		REMOTE_REPO_PATH="$remote_repo_path" \
		CONFIG_NAME="$config" \
		'bash -s' <<'EOF'
set -euo pipefail
cd "$REMOTE_REPO_PATH"
mkdir -p logs
pkill -u "$USER" -f 'target/release/sector-node' || true
sleep 2
nohup env RUST_LOG=info \
	./target/release/sector-node "crates/dawn-sector-node/config/${CONFIG_NAME}.toml" \
	>"logs/${CONFIG_NAME}.out.log" \
	2>"logs/${CONFIG_NAME}.err.log" \
	</dev/null &
EOF

	sleep "$restart_wait_seconds"

	local restore_line
	restore_line="$(ssh "$remote" "grep 'restoring from snapshot' '$log_path' 2>/dev/null | head -1 || true")"

	if [[ -z "$restore_line" ]]; then
		if ssh "$remote" "grep -q 'no snapshot at' '$log_path' 2>/dev/null"; then
			echo "FAIL  $node: restarted with no snapshot found -- checkpoint_interval_ticks may not have elapsed yet, or persistence paths are misconfigured"
		else
			echo "FAIL  $node: no startup persistence log line found within ${restart_wait_seconds}s"
		fi
		return 1
	fi

	local tick
	tick="$(echo "$restore_line" | grep -o 'tick=[0-9]*' | grep -o '[0-9]*')"
	if [[ -n "$tick" && "$tick" -gt 0 ]]; then
		echo "PASS  $node: $restore_line"
		return 0
	fi

	echo "FAIL  $node: restore log line found but tick was not a positive number: $restore_line"
	return 1
}

case "$command_name" in
	reachability)
		check_reachability
		;;
	tick-sla)
		check_tick_sla
		;;
	failover)
		check_failover
		;;
	restart-recovery)
		check_restart_recovery
		;;
	all)
		overall=0
		check_reachability || overall=1
		check_tick_sla || overall=1
		check_failover || overall=1
		check_restart_recovery || overall=1
		exit $overall
		;;
esac
