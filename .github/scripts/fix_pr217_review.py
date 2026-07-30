from pathlib import Path


def replace_if_needed(path: Path, old: str, new: str) -> None:
    text = path.read_text(encoding="utf-8")
    old_count = text.count(old)
    new_count = text.count(new)
    if old_count == 1 and new_count == 0:
        path.write_text(text.replace(old, new, 1), encoding="utf-8")
        return
    if old_count == 0 and new_count == 1:
        return
    raise SystemExit(
        f"{path}: expected exactly one old or new occurrence, "
        f"found old={old_count}, new={new_count}: {old!r}"
    )


transit = Path("crates/dawn-sector/src/node/transit.rs")
for old, new in [
    (
        "    pub(crate) fn propose_transit(&mut self, cmd: TransitCommand) -> Result<(), DawnError> {",
        "    fn propose_transit(&mut self, cmd: TransitCommand) -> Result<(), DawnError> {",
    ),
    ("    pub(crate) fn append_jump_events(", "    fn append_jump_events("),
    (
        "    pub(crate) fn export_transit(&self, ship_id: ShipId) -> Option<ShipSnapshot> {",
        "    fn export_transit(&self, ship_id: ShipId) -> Option<ShipSnapshot> {",
    ),
    ("    pub(crate) fn import_transit(", "    fn import_transit("),
    (
        "    pub(crate) fn replay_sector_transit_requested(",
        "    pub(super) fn replay_sector_transit_requested(",
    ),
    (
        "    pub(crate) fn replay_sector_transit_aborted(",
        "    pub(super) fn replay_sector_transit_aborted(",
    ),
    (
        "    pub(crate) fn replay_sector_transit_completed(",
        "    pub(super) fn replay_sector_transit_completed(",
    ),
]:
    replace_if_needed(transit, old, new)

replace_if_needed(
    transit,
    "    /// Re-anchor a Ship that just arrived in this Sector via Sector Transit\n"
    "    /// to the nearest body anchor to `entry_pos_abs`, appending the\n"
    "    /// authoritative `AnchorRebased` event (ADR-0029). No-op if the Ship or\n"
    "    /// an anchor candidate in this Sector can't be found.\n",
    "    /// Re-anchor a Ship that just arrived in this Sector via Sector Transit\n"
    "    /// to the nearest body anchor to `entry_pos_abs`, returning the\n"
    "    /// authoritative `AnchorRebased` event for the caller to append\n"
    "    /// (ADR-0029). Returns `None` if the Ship or an anchor candidate in this\n"
    "    /// Sector can't be found.\n",
)

server = Path("docs/architecture/architecture-review/server.md")
replace_if_needed(
    server,
    "| 永続化 | B+ | snapshot seamは改善。post-snapshot tail replayとの同値性を#197で固定する |",
    "| 永続化 | A− | snapshot seamとpost-snapshot tail replayの同値性を#197で固定済み |",
)
