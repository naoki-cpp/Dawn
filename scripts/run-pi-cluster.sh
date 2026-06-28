#!/usr/bin/env bash
# run-pi-cluster.sh
#
# Starts the three sector-node processes on the Raspberry Pi cluster over SSH.
# By default it preserves any existing process. Use --replace to stop an
# existing sector-node on each node before starting a new one.
#
# Usage:
#   scripts/run-pi-cluster.sh
#   scripts/run-pi-cluster.sh --replace
#   scripts/run-pi-cluster.sh --node node-0.local --node node-1.local --node node-2.local
set -euo pipefail

nodes=("node-0.local" "node-1.local" "node-2.local")
custom_nodes=0
user="dawn"
remote_repo_path="/home/dawn/Dawn"
replace=0
rust_log="info"

while [[ $# -gt 0 ]]; do
	case "$1" in
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
		--replace)
			replace=1
			shift
			;;
		--rust-log)
			rust_log="$2"
			shift 2
			;;
		*)
			echo "Unknown argument: $1" >&2
			exit 1
			;;
	esac
done

if [[ ${#nodes[@]} -ne 3 ]]; then
	echo "This script expects exactly 3 nodes." >&2
	exit 1
fi

configs=("node-0" "node-1" "node-2")

for idx in "${!nodes[@]}"; do
	node="${nodes[$idx]}"
	config_name="${configs[$idx]}"
	remote="${user}@${node}"

	echo "Starting $config_name on $remote"
	ssh "$remote" \
		REMOTE_REPO_PATH="$remote_repo_path" \
		CONFIG_NAME="$config_name" \
		REPLACE="$replace" \
		RUST_LOG_VALUE="$rust_log" \
		'bash -s' <<'EOF'
set -euo pipefail
cd "$REMOTE_REPO_PATH"
mkdir -p logs

if [[ "$REPLACE" = "1" ]]; then
	pkill -u "$USER" -f 'target/release/sector-node' || true
	sleep 1
fi

if pgrep -u "$USER" -f 'target/release/sector-node' >/dev/null; then
	echo "sector-node is already running; use --replace to restart it." >&2
	exit 1
fi

nohup env RUST_LOG="$RUST_LOG_VALUE" \
	./target/release/sector-node "crates/dawn-sector-node/config/${CONFIG_NAME}.toml" \
	>"logs/${CONFIG_NAME}.out.log" \
	2>"logs/${CONFIG_NAME}.err.log" \
	</dev/null &

pid=$!
echo "  pid=$pid logs/${CONFIG_NAME}.out.log logs/${CONFIG_NAME}.err.log"
EOF
done
