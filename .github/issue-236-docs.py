from pathlib import Path


def exact(path, old, new, expected=1):
    p = Path(path)
    text = p.read_text()
    count = text.count(old)
    if count != expected:
        raise SystemExit(f'{path}: expected {expected}, found {count}: {old[:100]!r}')
    p.write_text(text.replace(old, new))


path = 'docs/adr/ADR-0014-raft-consensus.md'
exact(
    path,
    '''    ShipSnapshotを含むCommitをproposal
''',
    '''    TransitHandoffStateを含むCommitをproposal
''',
)
exact(
    path,
    '''- 再起動後は同じShip snapshotを使ってCommitを再proposalする
''',
    '''- 再起動後はfrozen Shipから同じ`TransitHandoffState`を再生成してCommitを再proposalする
''',
)
old_payload = '''```rust
pub struct SectorTransitCompleted {
    pub ship_id: ShipId,
    pub from: SectorId,
    pub to: SectorId,
    pub request_tick: Tick,
    pub entry_pos: AbsolutePosition,
    pub velocity: Velocity,
    pub tick: Tick,
    pub ship_state: TransitShipState,
}

pub struct TransitShipState {
    pub ship_type_id: ShipTypeId,
    pub current_shield: f32,
    pub current_armor: f32,
    pub current_hull: f32,
    pub is_destroyed: bool,
    pub capacitor: Option<f32>,
    pub fitting: FittingSnapshot,
    pub inventory: BTreeMap<ItemId, u64>,
}
```

`position`・`anchor`は`entry_pos`とdestination側rebaseがauthority、`velocity`はevent本体、
`tackled_by`はSector-localなので`TransitShipState`へ重複させない。
`request_tick`はsource-localなattempt identityであり、Request → Commit → Completed → Ackの
全経路で変更せず伝播する。同じShipが同じ経路を複数回通っても別attemptとして照合する。
'''
new_payload = '''```rust
pub struct TransitHandoffState {
    pub ship_id: ShipId,
    pub ship_type_id: ShipTypeId,
    pub velocity: Velocity,
    pub current_shield: f32,
    pub current_armor: f32,
    pub current_hull: f32,
    pub is_destroyed: bool,
    pub capacitor: Option<f32>,
    pub fitting: FittingSnapshot,
    pub inventory: BTreeMap<ItemId, u64>,
}

pub struct SectorTransitCompleted {
    pub handoff: TransitHandoffState,
    pub from: SectorId,
    pub to: SectorId,
    pub request_tick: Tick,
    pub entry_pos: AbsolutePosition,
    pub tick: Tick,
}
```

同じ`TransitHandoffState`をRaft Commitと`SectorTransitCompleted`が共有する。
永続化用`ShipSnapshot`はsnapshot/restore境界だけに留まり、consensus payloadへ流用しない。
`position`・`anchor`は`entry_pos`とdestination側rebaseがauthorityであり、`tackled_by`も
Sector-localなのでhandoffへ含めない。AckはShip stateを返さず、
`ship_id + from + to + request_tick`だけでattemptを照合する。
`request_tick`はsource-localなattempt identityであり、Request → Commit → Completed → Ackの
全経路で変更せず伝播する。同じShipが同じ経路を複数回通っても別attemptとして照合する。
'''
exact(path, old_payload, new_payload)
exact(
    path,
    '''- `SectorTransitCompleted` on destination: `ship_state`からmaterializeし、`entry_pos`へre-anchorする
''',
    '''- `SectorTransitCompleted` on destination: `handoff`をlive importと同じ直接mappingでmaterializeし、`entry_pos`へre-anchorする
''',
)

exact(
    'docs/adr/ADR-0017-snapshot-compaction.md',
    '| Sector Transit（ADR-0014） | ShipSnapshot を Raft で転送 | 不要（replay ですらない） |',
    '| Sector Transit（ADR-0014） | Transit専用`TransitHandoffState`をRaftで転送し、Completed tailにも保存 | snapshot + tailで復旧 |',
)

path = 'docs/architecture/event-catalog.md'
old_completed = '''### `SectorTransitCompleted`

Self-contained completion event. `ship_state` carries the destination replay state without depending on the in-memory Raft actor surviving restart.

The destination appends this event when Commit materialization succeeds, then proposes Ack. The source appends it only after Ack, when it removes the frozen recovery copy. Thus a crash can temporarily retain two ECS copies, but never zero durable copies; only the destination copy is active after Commit.

| Field | Type | Required | Description |
|---|---|---|---|
| `ship_id` | `ShipId` | ✓ | Ship that transited |
| `from` | `SectorId` | ✓ | previous active Sector |
| `to` | `SectorId` | ✓ | new active Sector |
| `entry_pos` | `AbsolutePosition` | ✓ | authoritative entry coordinates in the destination Sector frame |
| `velocity` | `Velocity` | ✓ | velocity on entry |
| `tick` | `Tick` | ✓ | local completion Tick |
| `ship_state` | `TransitShipState` | ✓ | type / HP / capacitor / fitting / inventory used by destination Replay |

**Replay:** on `from`, remove the Ship. On `to`, rebuild a `ShipSnapshot` from `ship_state` + `entry_pos`, materialize it, and redo anchor rebase directly. The live `AnchorRebased` event precedes Completed and may replay before the destination Ship exists.
'''
new_completed = '''### `SectorTransitCompleted`

Self-contained completion event. `handoff` is the same canonical
`TransitHandoffState` carried by the Raft Commit, so destination replay does not
depend on an in-memory Raft actor or on persistence `ShipSnapshot` surviving the
protocol boundary.

The destination appends this event when Commit materialization succeeds, then proposes a minimal identity-only Ack. The source appends it only after Ack, when it removes the frozen recovery copy. Thus a crash can temporarily retain two ECS copies, but never zero durable copies; only the destination copy is active after Commit.

| Field | Type | Required | Description |
|---|---|---|---|
| `handoff` | `TransitHandoffState` | ✓ | Ship identity, type, velocity, HP, capacitor, fitting, and inventory |
| `from` | `SectorId` | ✓ | previous active Sector |
| `to` | `SectorId` | ✓ | new active Sector |
| `request_tick` | `Tick` | ✓ | source-local attempt identity |
| `entry_pos` | `AbsolutePosition` | ✓ | authoritative entry coordinates in the destination Sector frame |
| `tick` | `Tick` | ✓ | local completion Tick |

**Replay:** on `from`, remove `handoff.ship_id`. On `to`, feed `handoff` through the same direct destination-ECS mapping used by live Commit import, then redo anchor rebase from `entry_pos`. No fake `ShipSnapshot`, placeholder source anchor, or source position is reconstructed. The live `AnchorRebased` event precedes Completed and may replay before the destination Ship exists.
'''
exact(path, old_completed, new_completed)

exact(
    'docs/architecture/security-review.md',
    '''ギャップだと判明した。ノード間転送（Sector Transit）で運ばれる`ShipSnapshot`
（`crates/dawn-sector/src/persistence/snapshot.rs`）は`player_id`/所有権情報を一切含んでいない。
''',
    '''ギャップだと判明した。ノード間転送（Sector Transit）で運ばれる
`TransitHandoffState`は`player_id`/所有権情報を一切含んでいない。
永続化用`ShipSnapshot`も同様にownership authorityではない。
''',
)
exact(
    'docs/architecture/security-review.md',
    '''1. **狭い修正**: `ShipSnapshot`にトランジット元ノードの正規`owners`から`player_id`を
''',
    '''1. **狭い修正**: `TransitHandoffState`にトランジット元ノードの正規`owners`から`player_id`を
''',
)
