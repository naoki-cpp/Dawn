#!/usr/bin/env bash
# verify-pi-cluster.sh
#
# Runs the 8D-5 Raspberry Pi cluster validation checklist (roadmap.md "8D.
# 分散インフラ" item 5) against a cluster already deployed and started via
# deploy-pi-cluster.sh / run-pi-cluster.sh. Each check has an explicit
# pass/fail criterion so a single run produces a verdict, not just logs.
#
# Checks:
#   reachability  - all 3 nodes are up and listening on ws/raft/repl ports
#   tick-sla      - "[Node] tick overrun" rate in logs stays under threshold
#   failover      - after the operator partitions the current Raft leader,
#                   a new leader is elected within the expected window
#
# Usage:
#   scripts/verify-pi-cluster.sh reachability
#   scripts/verify-pi-cluster.sh tick-sla [--window-seconds 60] [--max-overrun-rate 0.01]
#   scripts/verify-pi-cluster.sh failover [--timeout-seconds 15]
#   scripts/verify-pi-cluster.sh all
set -euo pipefail

nodes=("node-0.local" "node-1.local" "node-2.local")
custom_nodes=0
user="dawn"
remote_repo_path="/home/dawn/Dawn"
window_seconds=60
max_overrun_rate="0.01"
failover_timeout_seconds=15
command_name=""

ports_for_node() {
	# node-0 -> ws 7878 raft 7900 repl 7910, node-1 -> 7879/7901/7911, etc.
	local idx="$1"
	echo "$((7878 + idx)) $((7900 + idx)) $((7910 + idx))"
}

while [[ $# -gt 0 ]]; do
	case "$1" in
		reachability|tick-sla|failover|all)
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
	all)
		overall=0
		check_reachability || overall=1
		check_tick_sla || overall=1
		check_failover || overall=1
		exit $overall
		;;
esac
