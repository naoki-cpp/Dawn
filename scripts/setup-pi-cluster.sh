#!/usr/bin/env bash
# setup-pi-cluster.sh
#
# Unified setup entrypoint for the Dawn Raspberry Pi cluster.
# - host-tools: create the repo-local cargo-zigbuild tool environment
# - ssh:        install the SSH key and optional ~/.ssh/config entries
# - all:        run host-tools, then ssh
#
# Usage:
#   scripts/setup-pi-cluster.sh
#   scripts/setup-pi-cluster.sh --host-tools
#   scripts/setup-pi-cluster.sh --ssh
#   scripts/setup-pi-cluster.sh --all
#   scripts/setup-pi-cluster.sh --print-tool-bin
#   scripts/setup-pi-cluster.sh --print-python
#   scripts/setup-pi-cluster.sh --print-cargo-zigbuild
#   scripts/setup-pi-cluster.sh --ssh --node node-0.local --node node-1.local --node node-2.local
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
venv_dir="$repo_root/.tools/python/cargo-zigbuild"

detect_venv_bin_dir() {
	case "$(uname -s)" in
		MINGW*|MSYS*|CYGWIN*) echo "$venv_dir/Scripts" ;;
		*) echo "$venv_dir/bin" ;;
	esac
}

venv_bin_dir="$(detect_venv_bin_dir)"
python_bin="$venv_bin_dir/python"
cargo_zigbuild_bin="$venv_bin_dir/cargo-zigbuild"
case "$(uname -s)" in
	MINGW*|MSYS*|CYGWIN*)
		python_bin="${python_bin}.exe"
		cargo_zigbuild_bin="${cargo_zigbuild_bin}.exe"
		;;
esac

setup_host_tools() {
	if [[ -x "$python_bin" && -x "$cargo_zigbuild_bin" ]]; then
		echo "cargo-zigbuild tool env already installed: $venv_dir"
		return 0
	fi

	mkdir -p "$repo_root/.tools/python"

	local python_launcher=()
	case "$(uname -s)" in
		MINGW*|MSYS*|CYGWIN*)
			if command -v py >/dev/null 2>&1 && py -3 --version >/dev/null 2>&1; then
				python_launcher=(py -3)
			elif command -v python >/dev/null 2>&1; then
				python_launcher=(python)
			elif command -v python3 >/dev/null 2>&1; then
				python_launcher=(python3)
			fi
			;;
		*)
			for candidate in python3 python; do
				if command -v "$candidate" >/dev/null 2>&1; then
					python_launcher=("$candidate")
					break
				fi
			done
			;;
	esac

	if [[ ${#python_launcher[@]} -eq 0 ]]; then
		echo "Python 3 was not found. Install Python first." >&2
		exit 1
	fi

	echo "Creating local Python virtual environment: $venv_dir"
	"${python_launcher[@]}" -m venv "$venv_dir"

	echo "Upgrading pip ..."
	"$python_bin" -m pip install --upgrade pip

	echo "Installing cargo-zigbuild into the local tool env ..."
	"$python_bin" -m pip install cargo-zigbuild

	echo "Installed local tool env: $venv_dir"
	echo "Add it to PATH in Git Bash with:"
	echo "  export PATH=\"$venv_bin_dir:\$PATH\""
	echo "  export CARGO_ZIGBUILD_PYTHON_PATH=\"$python_bin\""
}

setup_ssh() {
	local nodes=("node-0.local" "node-1.local" "node-2.local")
	local custom_nodes=0
	local user="dawn"
	local key_name="dawn_pi"
	local skip_config=0
	local force_keygen=0
	local facts_hostnames=()
	local facts_ips=()
	local facts_machine_ids=()

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
			--key-name)
				key_name="$2"
				shift 2
				;;
			--skip-config)
				skip_config=1
				shift
				;;
			--force-keygen)
				force_keygen=1
				shift
				;;
			*)
				echo "Unknown SSH setup argument: $1" >&2
				exit 1
				;;
		esac
	done

	if [[ ${#nodes[@]} -eq 0 ]]; then
		echo "No nodes configured" >&2
		exit 1
	fi

	expected_hostname_for_node() {
		local node="$1"
		printf '%s\n' "${node%.local}"
	}

	collect_remote_facts() {
		local remote="$1"
		ssh "$remote" "bash -lc 'host_name=\$(hostname); machine_id=\$(cat /etc/machine-id 2>/dev/null || true); ip=\$(hostname -I 2>/dev/null | awk '\''{print \$1}'\'' ); printf \"%s|%s|%s\n\" \"\$host_name\" \"\$ip\" \"\$machine_id\"'"
	}

	verify_unique_facts() {
		local -A seen_hostnames=()
		local -A seen_ips=()
		local -A seen_machine_ids=()
		local idx node expected_hostname actual_hostname ip machine_id

		for idx in "${!nodes[@]}"; do
			node="${nodes[$idx]}"
			expected_hostname="$(expected_hostname_for_node "$node")"
			actual_hostname="${facts_hostnames[$idx]}"
			ip="${facts_ips[$idx]}"
			machine_id="${facts_machine_ids[$idx]}"

			if [[ "$actual_hostname" != "$expected_hostname" ]]; then
				echo "Hostname mismatch: ${node} resolved to a Pi reporting hostname ${actual_hostname}" >&2
				echo "Expected hostname: ${expected_hostname}" >&2
				echo "Check Raspberry Pi Imager hostnames and ~/.ssh/config HostName entries." >&2
				exit 1
			fi

			if [[ -n "${seen_hostnames[$actual_hostname]:-}" ]]; then
				echo "Duplicate remote hostname detected: ${seen_hostnames[$actual_hostname]} and ${node} both report ${actual_hostname}" >&2
				exit 1
			fi
			seen_hostnames["$actual_hostname"]="$node"

			if [[ -n "$ip" ]]; then
				if [[ -n "${seen_ips[$ip]:-}" ]]; then
					echo "Duplicate remote IP detected: ${seen_ips[$ip]} and ${node} both report ${ip}" >&2
					exit 1
				fi
				seen_ips["$ip"]="$node"
			fi

			if [[ -n "$machine_id" ]]; then
				if [[ -n "${seen_machine_ids[$machine_id]:-}" ]]; then
					echo "Duplicate machine-id detected: ${seen_machine_ids[$machine_id]} and ${node} appear to be the same Raspberry Pi" >&2
					exit 1
				fi
				seen_machine_ids["$machine_id"]="$node"
			fi
		done
	}

	local ssh_dir="${HOME}/.ssh"
	local key_path="${ssh_dir}/${key_name}"
	local pubkey_path="${key_path}.pub"
	local config_path="${ssh_dir}/config"

	fix_windows_ssh_acl() {
		local path="$1"
		case "$(uname -s)" in
			MINGW*|MSYS*|CYGWIN*)
				if [[ -e "$path" ]]; then
					icacls "$path" /inheritance:r \
						/grant:r "${USERNAME}:F" \
						/grant:r "SYSTEM:F" \
						/grant:r "Administrators:F" >/dev/null
				fi
				;;
		esac
	}

	mkdir -p "$ssh_dir"
	chmod 700 "$ssh_dir"

	if [[ ! -f "$key_path" || ! -f "$pubkey_path" || $force_keygen -eq 1 ]]; then
		if [[ -f "$key_path" || -f "$pubkey_path" ]]; then
			rm -f "$key_path" "$pubkey_path"
		fi
		echo "Generating SSH key: $key_path"
		ssh-keygen -t ed25519 -f "$key_path" -N ""
	else
		echo "Using existing SSH key: $key_path"
	fi

	echo "Inspecting Raspberry Pi nodes"
	for node in "${nodes[@]}"; do
		local remote="${user}@${node}"
		echo "  -> $remote"
		local facts
		facts="$(collect_remote_facts "$remote")"
		IFS='|' read -r actual_hostname actual_ip actual_machine_id <<< "$facts"
		facts_hostnames+=("$actual_hostname")
		facts_ips+=("$actual_ip")
		facts_machine_ids+=("$actual_machine_id")
		echo "     hostname=${actual_hostname} ip=${actual_ip:-unknown}"
	done

	verify_unique_facts
	echo "Verified that each node resolves to a distinct Raspberry Pi."

	echo "Installing public key on Raspberry Pi nodes"
	for node in "${nodes[@]}"; do
		local remote="${user}@${node}"
		echo "  -> $remote"
		cat "$pubkey_path" | ssh "$remote" \
			"umask 077; mkdir -p ~/.ssh; touch ~/.ssh/authorized_keys; cat >> ~/.ssh/authorized_keys; chmod 700 ~/.ssh; chmod 600 ~/.ssh/authorized_keys"
	done

	if [[ $skip_config -eq 0 ]]; then
		local tmp_config
		tmp_config="$(mktemp)"
		trap 'rm -f "$tmp_config"' EXIT

		if [[ -f "$config_path" ]]; then
			cp "$config_path" "$tmp_config"
		else
			: > "$tmp_config"
		fi

		if [[ -s "$tmp_config" ]] && [[ "$(tail -c 1 "$tmp_config")" != $'\n' ]]; then
			printf '\n' >> "$tmp_config"
		fi

		for node in "${nodes[@]}"; do
			if ! grep -Eq "^Host[[:space:]]+$node([[:space:]]|\$)" "$tmp_config"; then
				cat >> "$tmp_config" <<EOF
Host $node
    User $user
    IdentityFile $key_path

EOF
			fi
		done

		mv "$tmp_config" "$config_path"
		chmod 600 "$config_path"
		fix_windows_ssh_acl "$config_path"
		echo "Updated SSH config: $config_path"
	fi

	echo
	echo "Done. Test with:"
	for node in "${nodes[@]}"; do
		echo "  ssh $node"
	done
}

mode="${1:-"--host-tools"}"
case "$mode" in
	--host-tools)
		setup_host_tools
		;;
	--ssh)
		shift
		setup_ssh "$@"
		;;
	--all)
		shift
		setup_host_tools
		setup_ssh "$@"
		;;
	--print-tool-bin)
		echo "$venv_bin_dir"
		;;
	--print-python)
		echo "$python_bin"
		;;
	--print-cargo-zigbuild)
		echo "$cargo_zigbuild_bin"
		;;
	*)
		echo "Unknown mode: $mode" >&2
		exit 1
		;;
esac
