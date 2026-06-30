#!/usr/bin/env bash
# deploy-pi-cluster.sh
#
# Builds `sector-node` on the host PC (optional), packages only the runtime
# artifact plus config/data files, and deploys that bundle to the Raspberry Pi
# cluster over SSH.
#
# Usage:
#   scripts/deploy-pi-cluster.sh
#   scripts/deploy-pi-cluster.sh --build-host
#   scripts/deploy-pi-cluster.sh --artifact-path /abs/path/to/sector-node
#   scripts/deploy-pi-cluster.sh --target-triple arm-unknown-linux-gnueabihf
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
nodes=("node-0.local" "node-1.local" "node-2.local")
custom_nodes=0
user="dawn"
remote_repo_path="/home/dawn/Dawn"
interface="wlan0"
no_progress=0
build_host=0
target_triple="arm-unknown-linux-gnueabihf"
artifact_path=""

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
		--interface)
			interface="$2"
			shift 2
			;;
		--artifact-path)
			artifact_path="$2"
			shift 2
			;;
		--target-triple)
			target_triple="$2"
			shift 2
			;;
		--build-host)
			build_host=1
			shift
			;;
		--no-progress)
			no_progress=1
			shift
			;;
		*)
			echo "Unknown argument: $1" >&2
			exit 1
			;;
	esac
done

if [[ ${#nodes[@]} -ne 3 ]]; then
	echo "This script currently expects exactly 3 nodes for auto-generated cluster config." >&2
	exit 1
fi

if [[ -z "$artifact_path" ]]; then
	artifact_path="$repo_root/target/$target_triple/release/sector-node"
fi

stage_dir="$(mktemp -d "/tmp/dawn-deploy-stage.XXXXXX")"
archive_name="dawn-deploy-$(date +%Y%m%d%H%M%S)-$$.tar.gz"
archive_path="$(mktemp "/tmp/${archive_name}.XXXXXX")"
node_ips=()
node_statuses=()
node_details=()
remote_parent_path="$(dirname "$remote_repo_path")"
trap 'rm -rf "$stage_dir"; rm -f "$archive_path"' EXIT

progress_enabled=0
progress_lines=0
if [[ -t 1 && $no_progress -eq 0 && "${TERM:-}" != "dumb" ]]; then
	progress_enabled=1
fi

announce() {
	if [[ $progress_enabled -eq 0 ]]; then
		echo "$@"
	fi
}

progress_bar() {
	local status="$1"
	local width=6
	local filled=0

	case "$status" in
		pending)   filled=0 ;;
		detecting) filled=1 ;;
		ready)     filled=2 ;;
		copying)   filled=3 ;;
		expanding) filled=4 ;;
		done)      filled=6 ;;
		failed)    filled=6 ;;
		*)         filled=0 ;;
	esac

	if [[ "$status" == "failed" ]]; then
		printf '[%*s]' "$width" '' | tr ' ' '!'
		return
	fi

	printf '['
	for ((i=0; i<filled; i++)); do
		printf '#'
	done
	for ((i=filled; i<width; i++)); do
		printf '.'
	done
	printf ']'
}

render_progress() {
	[[ $progress_enabled -eq 1 ]] || return 0

	if [[ $progress_lines -gt 0 ]]; then
		printf '\033[%sA' "$progress_lines"
	fi

	local lines=0
	printf '\r\033[2KPi deploy progress\n'
	lines=$((lines + 1))

	for idx in "${!nodes[@]}"; do
		local detail=""
		if [[ -n "${node_details[$idx]:-}" ]]; then
			detail=": ${node_details[$idx]}"
		fi
		printf '\r\033[2K  %-12s %s %s%s\n' "${nodes[$idx]}" "$(progress_bar "${node_statuses[$idx]}")" "${node_statuses[$idx]}" "$detail"
		lines=$((lines + 1))
	done

	progress_lines=$lines
}

finish_progress() {
	[[ $progress_enabled -eq 1 ]] || return 0
	if [[ $progress_lines -gt 0 ]]; then
		printf '\r\033[2K\n'
		progress_lines=0
	fi
}

set_node_status() {
	local node_name="$1"
	local status="$2"
	local idx

	for idx in "${!nodes[@]}"; do
		if [[ "${nodes[$idx]}" == "$node_name" ]]; then
			node_statuses[$idx]="$status"
			render_progress
			return
		fi
	done
}

set_node_detail() {
	local node_name="$1"
	local detail="$2"
	local idx

	for idx in "${!nodes[@]}"; do
		if [[ "${nodes[$idx]}" == "$node_name" ]]; then
			node_details[$idx]="$detail"
			render_progress
			return
		fi
	done
}

clear_node_detail() {
	set_node_detail "$1" ""
}

detect_node_ip() {
	local remote="$1"
	ssh "$remote" "bash -lc 'ip -o -4 addr show dev \"$interface\" | awk '\''{split(\$4,a,\"/\"); print a[1]; exit}'\'''"
}

ensure_unique_node_ips() {
	local -A seen_ips=()
	local idx
	local node
	local ip

	for idx in "${!nodes[@]}"; do
		node="${nodes[$idx]}"
		ip="${node_ips[$idx]}"
		if [[ -n "${seen_ips[$ip]:-}" ]]; then
			echo "Duplicate node IP detected: ${seen_ips[$ip]} and ${node} both resolve to ${ip}" >&2
			echo "This usually means multiple hostnames are pointing at the same Raspberry Pi." >&2
			echo "Check Raspberry Pi Imager hostnames, ~/.ssh/config HostName entries, and .local resolution." >&2
			exit 1
		fi
		seen_ips["$ip"]="$node"
	done
}

build_host_artifact() {
	announce "Building sector-node on host for target $target_triple"
	"$repo_root/scripts/setup-pi-cluster.sh" --host-tools >/dev/null
	(
		cd "$repo_root"
		local tool_bin_dir=""
		local python_bin=""
		local cargo_zigbuild_bin=""
		tool_bin_dir="$("$repo_root/scripts/setup-pi-cluster.sh" --print-tool-bin)"
		python_bin="$("$repo_root/scripts/setup-pi-cluster.sh" --print-python)"
		cargo_zigbuild_bin="$("$repo_root/scripts/setup-pi-cluster.sh" --print-cargo-zigbuild)"

		export PATH="$tool_bin_dir:$PATH"
		export CARGO_ZIGBUILD_PYTHON_PATH="$python_bin"

		if [[ -z "$cargo_zigbuild_bin" || ! -x "$cargo_zigbuild_bin" ]]; then
			echo "cargo-zigbuild was not found in the repo-local tool env." >&2
			echo "Run scripts/setup-pi-cluster.sh --host-tools first." >&2
			exit 1
		fi

		"$cargo_zigbuild_bin" zigbuild -p dawn-sector-node --release --target "$target_triple"
	)
}

ensure_host_artifact() {
	if [[ -f "$artifact_path" && $build_host -eq 0 ]]; then
		announce "Using existing host artifact: $artifact_path"
		return 0
	fi

	announce "Ensuring Rust target is installed: $target_triple"
	rustup target add "$target_triple"
	build_host_artifact
}

verify_runtime_inputs() {
	if [[ ! -f "$artifact_path" ]]; then
		echo "Artifact not found: $artifact_path" >&2
		echo "deploy-pi-cluster.sh should have built it automatically." >&2
		exit 1
	fi

	if [[ ! -f "$repo_root/data/galaxy.toml" ]]; then
		echo "Required runtime data file missing: $repo_root/data/galaxy.toml" >&2
		exit 1
	fi
}

rewrite_cluster_configs() {
	local config_dir="$stage_dir/crates/dawn-sector-node/config"
	local node0_ip="$1"
	local node1_ip="$2"
	local node2_ip="$3"

	mkdir -p "$config_dir"

	cat > "$config_dir/node-0.toml" <<EOF
# dawn-sector-node configuration - Node 0 (Sector 0)
# Usage: sector-node config/node-0.toml

node_id    = 0
sector_id  = 0
ws_addr    = "0.0.0.0:7878"
raft_addr  = "0.0.0.0:7900"
repl_addr  = "0.0.0.0:7910"
npc_ships  = 0
pop_cap    = 50

[[peers]]
node_id   = 1
raft_addr = "${node1_ip}:7901"
repl_addr = "${node1_ip}:7911"
ws_addr   = "${node1_ip}:7879"

[[peers]]
node_id   = 2
raft_addr = "${node2_ip}:7902"
repl_addr = "${node2_ip}:7912"
ws_addr   = "${node2_ip}:7880"
EOF

	cat > "$config_dir/node-1.toml" <<EOF
# dawn-sector-node configuration - Node 1 (Sector 1)
# Usage: sector-node config/node-1.toml

node_id    = 1
sector_id  = 1
ws_addr    = "0.0.0.0:7879"
raft_addr  = "0.0.0.0:7901"
repl_addr  = "0.0.0.0:7911"
npc_ships  = 0
pop_cap    = 50

[[peers]]
node_id   = 0
raft_addr = "${node0_ip}:7900"
repl_addr = "${node0_ip}:7910"
ws_addr   = "${node0_ip}:7878"

[[peers]]
node_id   = 2
raft_addr = "${node2_ip}:7902"
repl_addr = "${node2_ip}:7912"
ws_addr   = "${node2_ip}:7880"
EOF

	cat > "$config_dir/node-2.toml" <<EOF
# dawn-sector-node configuration - Node 2 (Sector 2)
# Usage: sector-node config/node-2.toml

node_id    = 2
sector_id  = 2
ws_addr    = "0.0.0.0:7880"
raft_addr  = "0.0.0.0:7902"
repl_addr  = "0.0.0.0:7912"
npc_ships  = 0
pop_cap    = 50

[[peers]]
node_id   = 0
raft_addr = "${node0_ip}:7900"
repl_addr = "${node0_ip}:7910"
ws_addr   = "${node0_ip}:7878"

[[peers]]
node_id   = 1
raft_addr = "${node1_ip}:7901"
repl_addr = "${node1_ip}:7911"
ws_addr   = "${node1_ip}:7879"
EOF
}

stage_runtime_bundle() {
	mkdir -p "$stage_dir/target/release"
	cp "$artifact_path" "$stage_dir/target/release/sector-node"
	chmod +x "$stage_dir/target/release/sector-node"

	mkdir -p "$stage_dir/data"
	cp "$repo_root/data/galaxy.toml" "$stage_dir/data/galaxy.toml"
	if [[ -f "$repo_root/data/modules.toml" ]]; then
		cp "$repo_root/data/modules.toml" "$stage_dir/data/modules.toml"
	fi
	if [[ -f "$repo_root/data/ship_types.toml" ]]; then
		cp "$repo_root/data/ship_types.toml" "$stage_dir/data/ship_types.toml"
	fi

	rewrite_cluster_configs "${node_ips[0]}" "${node_ips[1]}" "${node_ips[2]}"
}

for node in "${nodes[@]}"; do
	node_statuses+=("pending")
	node_details+=("")
done

ensure_host_artifact
verify_runtime_inputs

render_progress

for node in "${nodes[@]}"; do
	remote="${user}@${node}"
	set_node_status "$node" "detecting"
	announce "Detecting $interface IPv4 on $remote"
	ip="$(detect_node_ip "$remote")"
	if [[ -z "$ip" ]]; then
		clear_node_detail "$node"
		set_node_status "$node" "failed"
		echo "Could not detect an IPv4 address for $remote on interface $interface" >&2
		exit 1
	fi
	announce "  $remote -> $ip"
	node_ips+=("$ip")
	set_node_detail "$node" "$ip"
	set_node_status "$node" "ready"
done

ensure_unique_node_ips
stage_runtime_bundle

remote_archive_path="/tmp/$archive_name"
announce "Creating deployment archive: $archive_path"
tar -czf "$archive_path" -C "$stage_dir" .

for node in "${nodes[@]}"; do
	remote="${user}@${node}"

	set_node_status "$node" "copying"
	clear_node_detail "$node"
	announce "Copying archive to $remote"
	scp -q "$archive_path" "${remote}:${remote_archive_path}"

	set_node_status "$node" "expanding"
	announce "Expanding archive on $remote"
	ssh "$remote" \
		REMOTE_ARCHIVE_PATH="$remote_archive_path" \
		REMOTE_REPO_PATH="$remote_repo_path" \
		REMOTE_PARENT_PATH="$remote_parent_path" \
		'bash -s' <<'EOF'
set -euo pipefail
mkdir -p "$REMOTE_PARENT_PATH"
rm -rf "${REMOTE_REPO_PATH}.new"
mkdir -p "${REMOTE_REPO_PATH}.new"
tar -xzf "$REMOTE_ARCHIVE_PATH" -C "${REMOTE_REPO_PATH}.new"
if [ -d "$REMOTE_REPO_PATH/logs" ]; then
	mv "$REMOTE_REPO_PATH/logs" "${REMOTE_REPO_PATH}.new/logs"
fi
rm -rf "${REMOTE_REPO_PATH}.prev"
if [ -d "$REMOTE_REPO_PATH" ]; then
	mv "$REMOTE_REPO_PATH" "${REMOTE_REPO_PATH}.prev"
fi
mv "${REMOTE_REPO_PATH}.new" "$REMOTE_REPO_PATH"
chmod +x "$REMOTE_REPO_PATH/target/release/sector-node"
rm -f "$REMOTE_ARCHIVE_PATH"
EOF

	set_node_status "$node" "done"
done

finish_progress
echo "Deployment complete. Start the cluster with: scripts/run-pi-cluster.sh"
