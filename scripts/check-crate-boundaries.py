"""Check Dawn's production crate dependency boundaries."""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path


def main() -> int:
    root = Path(__file__).resolve().parent.parent
    metadata = json.loads(
        subprocess.check_output(
            [
                "cargo",
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
                "--manifest-path",
                str(root / "Cargo.toml"),
            ],
            cwd=root,
            text=True,
        )
    )
    packages = {package["name"]: package for package in metadata["packages"]}
    errors: list[str] = []

    forbidden_packages = {
        "dawn-simulation",
        "dawn-sector-node",
        "dawn-wire",
        "dawn-event-store",
        "dawn-consensus",
        "dawn-peer-transport",
        "dawn-replication",
    }
    for package in sorted(forbidden_packages & packages.keys()):
        errors.append(f"obsolete package is still in the workspace: {package}")

    server = packages.get("dawn-server")
    if server is None:
        errors.append("dawn-server is missing from the workspace")
    else:
        binary_names = {
            target["name"]
            for target in server["targets"]
            if "bin" in target["kind"]
        }
        for required in {"simulate", "sector-node"} - binary_names:
            errors.append(f"dawn-server is missing required binary: {required}")

    for required in {"dawn-protocol", "dawn-storage", "dawn-distributed"}:
        if required not in packages:
            errors.append(f"final workspace boundary is missing: {required}")

    def normal_dependencies(package: dict) -> set[str]:
        return {
            dependency["name"]
            for dependency in package["dependencies"]
            if dependency["kind"] in (None, "normal")
            and dependency["name"] in packages
        }

    graph = {
        name: normal_dependencies(package)
        for name, package in packages.items()
    }

    forbidden_edges = {
        "dawn-core": graph.get("dawn-core", set()),
        "dawn-protocol": {"dawn-sector", "dawn-server", "dawn-actor"},
        "dawn-client-core": {"dawn-sector", "dawn-server", "dawn-actor"},
        "dawn-client-gdext": {
            "dawn-sector",
            "dawn-server",
            "dawn-actor",
            "dawn-simulation",
            "dawn-sector-node",
        },
        "dawn-market": {"dawn-sector", "dawn-server", "dawn-simulation"},
        "dawn-actor": {
            "dawn-sector",
            "dawn-server",
            "dawn-simulation",
            "dawn-sector-node",
        },
        "dawn-sector": {
            "dawn-server",
            "dawn-simulation",
            "dawn-sector-node",
            "dawn-client-core",
            "dawn-client-gdext",
        },
        "dawn-storage": {
            "dawn-sector",
            "dawn-server",
            "dawn-distributed",
            "dawn-client-core",
            "dawn-client-gdext",
        },
        "dawn-distributed": {
            "dawn-sector",
            "dawn-server",
            "dawn-actor",
            "dawn-client-core",
            "dawn-client-gdext",
            "dawn-protocol",
            "dawn-market",
        },
    }
    for package, forbidden in forbidden_edges.items():
        if package not in graph:
            continue
        for dependency in sorted(graph[package] & forbidden):
            errors.append(f"forbidden production dependency: {package} -> {dependency}")

    visiting: set[str] = set()
    visited: set[str] = set()

    def visit(package: str, path: list[str]) -> None:
        if package in visiting:
            cycle = " -> ".join(path + [package])
            errors.append(f"production dependency cycle: {cycle}")
            return
        if package in visited:
            return
        visiting.add(package)
        for dependency in sorted(graph.get(package, ())):
            visit(dependency, path + [package])
        visiting.remove(package)
        visited.add(package)

    for package in sorted(graph):
        visit(package, [])

    if errors:
        print("crate boundary check failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print(f"crate boundary check passed ({len(packages)} packages, acyclic production DAG)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
