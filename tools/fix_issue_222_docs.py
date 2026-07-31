from pathlib import Path


def replace_once(path: str, old: str, new: str) -> None:
    file_path = Path(path)
    text = file_path.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected one match, found {count}")
    file_path.write_text(text.replace(old, new, 1), encoding="utf-8")


replace_once(
    ".agents/commands/doc-sync.md",
    """- The documented quirks (`WarpCommand`'s legacy `gate_id` vs `target` form,
  the `gate_id`/`target_id` selection on `ApproachCommand`/`OrbitCommand`/
  `KeepAtRangeCommand`) still match `client_command_from_json`""",
    """- Navigation commands use required tagged targets: `WarpCommand` carries a
  `Gate` or `Body` target, while `ApproachCommand`/`OrbitCommand`/
  `KeepAtRangeCommand` carry a `Ship` or `Gate` target; no legacy
  `gate_id`/`target_id` fallback fields remain in `client_command_from_wire`""",
)

replace_once(
    "docs/adr/ADR-0015-approach-piloting.md",
    """ロックオンとは独立に、プレイヤーが**クリックで選択した船またはゲート**を対象にする。
クライアントは選択中の対象（船 ID / ゲート ID・排他）を保持し、A キーで
`ApproachCommand` を送る（船は `target_id`、ゲートは `gate_id` を JSON に含める）。
サーバーは所有権（`ship_owners`）を確認してから `ApproachComp` を付与する。""",
    """ロックオンとは独立に、プレイヤーが**クリックで選択した船またはゲート**を対象にする。
クライアントは選択中の対象を保持し、A キーで
`ClientMessage::Command(ClientCommandWire::ApproachCommand { target })` を
postcard エンコードした binary frame として送る。`target` は船なら
`NavigationTargetWire::Ship`、ゲートなら `NavigationTargetWire::Gate` である。
サーバーは所有権（`ship_owners`）を確認してから `ApproachComp` を付与する。""",
)

replace_once(
    "docs/adr/ADR-0041-dawn-wire-command-send.md",
    """ADR-0031/ADR-0035）や排他選択フィールド（`gate_id` xor `target_id`等）といった
ドメイン意味論""",
    """ADR-0031/ADR-0035）やタグ付きナビゲーション target の構築といった
ドメイン意味論""",
)

replace_once(
    "docs/adr/ADR-0041-dawn-wire-command-send.md",
    """この結果、今後「単純な」新規コマンドを追加する場合は""",
    """その後、Issue #222 で `gate_id` xor `target_id` の排他フィールドと
Warp の legacy `gate_id` fallback は削除され、必須のタグ付き target enum に
置き換えられた。専用メソッドを維持する現在の理由は、この target enum を
型安全に構築するためである。

この結果、今後「単純な」新規コマンドを追加する場合は""",
)

replace_once(
    "client/scripts/connection.gd",
    "## Commands with sentinel/exclusive-selection semantics (ADR-0031/ADR-0035)",
    "## Commands with sentinel/tagged-target semantics (ADR-0031/ADR-0035)",
)

print("updated Issue #222 documentation references")
