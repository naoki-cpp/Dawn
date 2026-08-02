//! `StateSnapshot` — point-in-time capture of a `SimulationNode`'s ECS state.
//!
//! # Recovery procedure (INV-002)
//!
//! 1. Load `StateSnapshot` from disk.
//! 2. Open the `FileEventStore` for the same Sector.
//! 3. Call `SimulationNode::restore_from(store, &snapshot, galaxy, &modules, &ship_types)`
//!    with the same module / ship-type definitions the node was configured with.
//!    - The snapshot reconstructs the ECS World up to `log_index`
//!      (position, velocity, hull layers, capacitor, fitting).
//!    - Events at `log_index` and beyond are replayed on top.
//! 4. The restored node is equivalent to the node at shutdown.
//!
//! # Snapshot is the authoritative durable checkpoint (ADR-0017 / INV-002)
//!
//! The Event Log stays append-only (INV-001) and is the history / propagation /
//! snapshot-source. But the snapshot — not genesis replay — is what operational
//! recovery and failover (ADR-0014) rely on: load the latest snapshot, then
//! catch up the tail of events.
//!
//! Publication is write-new-then-replace: encode and validate the postcard
//! payload, write it to a uniquely named sibling file, flush and sync that file,
//! then replace the authoritative path. Before replacement, the persistence
//! layer opens the parent directory and durably preserves a readable rollback
//! copy of any existing snapshot. Unix uses same-directory `rename` plus parent-
//! directory sync; Windows uses `MoveFileExW` with replace-existing and write-
//! through flags. A handled failure at any publication stage restores the prior
//! authoritative snapshot (or restores absence for a failed first publication).
//!
//! Derived / transient state (position, capacitor, lock countdowns) is persisted
//! in the snapshot. It is a per-tick pure function (position = velocity integral,
//! cap = recharge) and is NOT event-sourced, so it cannot be rebuilt from events
//! alone — it is restored from the snapshot and recomputed as the sim runs forward.
//!
//! Genesis (index 0) reconstruction is off-path (audit / disaster only): apply
//! events to rebuild authoritative state, then let transient state recompute. No
//! operational path depends on it.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

use dawn_core::{
    fitting::FittingSnapshot, AbsolutePosition, NodeId, Position, SectorBounds, SectorId, ShipId,
    ShipTypeId, Tick, Velocity,
};
use serde::{Deserialize, Serialize};

const TEMP_FILE_CREATE_ATTEMPTS: usize = 128;
static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
thread_local! {
    static DIRECTORY_SYNC_FAILURE: std::cell::RefCell<Option<(PathBuf, usize)>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn inject_directory_sync_failure(destination: impl AsRef<Path>, call_number: usize) {
    assert!(call_number > 0, "directory sync call numbers are one-based");
    DIRECTORY_SYNC_FAILURE.with(|failure| {
        *failure.borrow_mut() = Some((destination.as_ref().to_path_buf(), call_number));
    });
}

// ── Ship-level snapshot ───────────────────────────────────────────────────────

/// State of a single Ship at the time of the snapshot.
///
/// Captures everything needed to reconstruct the Ship's ECS components
/// (`PositionComp`, `VelocityComp`, `ShipStatsComp`, `HullComp`,
/// `CapacitorComp`, `FittingComp`) without replaying events from the
/// beginning of the log (INV-002).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShipSnapshot {
    pub ship_id: ShipId,
    pub ship_type_id: ShipTypeId,
    /// Authoritative Sector-frame position (ADR-0044). `None` for a
    /// transient/in-memory state that has not acquired a Sector-frame
    /// projection yet.
    pub absolute_position: Option<AbsolutePosition>,
    /// Anchor-relative offset retained as the local simulation representation.
    pub position: Position,
    /// Coordinate anchor the `position` offset is relative to (ADR-0029).
    pub anchor: dawn_core::AnchorId,
    pub velocity: Velocity,
    /// `HullComp` at the time of the snapshot (Shield / Armor / Hull layers).
    pub current_shield: f32,
    pub current_armor: f32,
    pub current_hull: f32,
    pub is_destroyed: bool,
    /// `CapacitorComp.current`, if the ship has a capacitor.
    pub capacitor: Option<f32>,
    /// Fitted modules (High/Mid/Low/Rig) and their on/off state.
    pub fitting: FittingSnapshot,
    /// Ships currently tackling this ship (ADR-0024). Persisted so tackle
    /// state is not lost on restart (which would allow escape).
    pub tackled_by: Vec<dawn_core::ShipId>,
    /// Unfitted / unassembled items the pilot owns (ADR-0034).
    pub inventory: std::collections::BTreeMap<dawn_core::ItemId, u64>,
}

/// Durable destination-side receipt for an imported Sector Transit.
///
/// Ship presence is not a valid deduplication marker because the imported Ship
/// may be destroyed or transit onward before an old Commit retry arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletedIncomingTransit {
    pub ship_id: ShipId,
    pub from: SectorId,
    pub to: SectorId,
    pub request_tick: Tick,
}

// ── Node-level snapshot ───────────────────────────────────────────────────────

/// Complete state of a `SimulationNode` at a specific `log_index`.
///
/// Stores enough information to reconstruct the ECS World without replaying
/// events from the beginning of time.
///
/// # Format compatibility (ADR-0017)
///
/// The on-disk format is **version-locked to the binary**: postcard is not
/// self-describing, so fields are read positionally and a snapshot written by
/// a different field list fails to load with `DeserializeUnexpectedEnd`.
/// `#[serde(default)]` does **not** grant field-level back-compat here the way
/// it would for a self-describing format like JSON — it is a no-op on this
/// path, so do not add it to imply a compatibility guarantee that does not
/// exist. Changing this struct means operators regenerate snapshots.
///
/// Which fields belong here is enforced from both sides:
/// `SimulationNode::take_snapshot` destructures the node exhaustively, and
/// `SimulationNode::apply_snapshot` destructures this struct exhaustively, so
/// adding a field to either is a compile error until it is handled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    /// The node that produced this snapshot.
    pub node_id: NodeId,
    /// The Sector this node manages.
    pub sector_id: SectorId,
    /// Spatial bounds of the Sector.
    pub bounds: SectorBounds,
    /// All events with index < `log_index` are covered by this snapshot.
    /// Events at `log_index` and beyond must be replayed.
    pub log_index: u64,
    /// Logical tick at the time of the snapshot.
    pub tick: Tick,
    /// Next value for `SimulationNode::id_counter`.
    /// Must be restored to prevent EntityId reuse (INV-004).
    pub id_counter: u64,
    /// Next value for `SimulationNode::player_id_counter`.
    ///
    /// Must be restored for the same reason as `id_counter`: ownership is not
    /// carried in the snapshot (a returning client re-asserts it via
    /// `adopt_player_ship`, ADR-0007 §2-A resume), so a counter that restarted
    /// at zero would hand a freshly-admitted client a `PlayerId` that a
    /// restored, still-owned ship already belongs to.
    pub player_id_counter: u64,
    /// State of every Ship in the Sector at the snapshot instant.
    pub ships: Vec<ShipSnapshot>,
    /// Destination-side receipts that survive checkpoint compaction.
    #[serde(default)]
    pub completed_incoming_transits: Vec<CompletedIncomingTransit>,
    /// Current docked station per ship.
    pub docked_ships: BTreeMap<dawn_core::ShipId, dawn_core::StationId>,
    /// Current docked station context per player.
    pub docked_players: BTreeMap<dawn_core::PlayerId, dawn_core::StationId>,
}

impl StateSnapshot {
    /// Encode, validate, durably write, and atomically publish this snapshot.
    ///
    /// The temporary file and rollback copy are siblings of `path`, so every
    /// replacement stays on the same filesystem. A handled error restores the
    /// previously readable authoritative snapshot before this function returns.
    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        self.save_with_before_publish(path.as_ref(), |_| Ok(()))
    }

    fn save_with_before_publish<F>(&self, path: &Path, before_publish: F) -> io::Result<()>
    where
        F: FnOnce(&Path) -> io::Result<()>,
    {
        let bytes = postcard::to_stdvec(self).map_err(|e| io::Error::other(e.to_string()))?;
        Self::decode(&bytes, "encoded snapshot")?;

        // Open the parent before replacement. On Unix this preserves a usable
        // directory handle even if a path lookup would fail after the rename.
        let directory = SnapshotDirectory::open(path)?;

        let mut temp = SnapshotTempFile::create(path)?;
        {
            let file = temp.file_mut();
            file.write_all(&bytes)?;
            file.flush()?;
            file.sync_all()?;
            file.seek(SeekFrom::Start(0))?;

            let mut persisted = Vec::with_capacity(bytes.len());
            file.read_to_end(&mut persisted)?;
            if persisted != bytes {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "temporary snapshot bytes differ from the encoded snapshot",
                ));
            }
            Self::decode(&persisted, "temporary snapshot")?;
        }
        temp.close();

        let previous = SnapshotBackup::capture(path)?;
        if previous.is_some() {
            sync_snapshot_directory(&directory, path)?;
        }

        before_publish(temp.path())?;
        if let Err(error) = publish_snapshot(temp.path(), path) {
            let rollback = rollback_publication(previous, path, &directory);
            return Err(publication_error(error, rollback));
        }
        temp.disarm();

        if let Err(error) = sync_snapshot_directory(&directory, path) {
            let rollback = rollback_publication(previous, path, &directory);
            return Err(publication_error(error, rollback));
        }

        if let Some(backup) = previous {
            let _ = backup.cleanup();
            // The authoritative replacement is already durable. Cleanup is
            // best-effort and uses one fixed rollback name, so it cannot grow
            // an unbounded set of artifacts if removal itself is unavailable.
            let _ = directory.sync();
        }
        Ok(())
    }

    /// Read from `path` and deserialise.
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        Self::decode(&bytes, "current snapshot")
    }

    fn decode(bytes: &[u8], context: &str) -> io::Result<Self> {
        postcard::from_bytes(bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{context}: {e}")))
    }
}

fn snapshot_parent(destination: &Path) -> &Path {
    destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn snapshot_sibling(destination: &Path, suffix: &str) -> io::Result<PathBuf> {
    let file_name = destination.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "snapshot destination must include a file name",
        )
    })?;
    let mut sibling_name = OsString::from(".");
    sibling_name.push(file_name);
    sibling_name.push(suffix);
    Ok(snapshot_parent(destination).join(sibling_name))
}

struct SnapshotDirectory {
    #[cfg(unix)]
    handle: File,
}

impl SnapshotDirectory {
    fn open(destination: &Path) -> io::Result<Self> {
        #[cfg(unix)]
        {
            let handle = File::open(snapshot_parent(destination))?;
            Ok(Self { handle })
        }
        #[cfg(not(unix))]
        {
            let _ = destination;
            Ok(Self {})
        }
    }

    fn sync(&self) -> io::Result<()> {
        #[cfg(unix)]
        {
            self.handle.sync_all()
        }
        #[cfg(not(unix))]
        {
            // Windows publication itself requests write-through. Other
            // platforms are rejected by `publish_snapshot`.
            Ok(())
        }
    }
}

fn sync_snapshot_directory(
    directory: &SnapshotDirectory,
    _destination: &Path,
) -> io::Result<()> {
    #[cfg(test)]
    if should_fail_directory_sync(_destination) {
        return Err(io::Error::other(
            "injected failure while syncing the snapshot directory",
        ));
    }
    directory.sync()
}

#[cfg(test)]
fn should_fail_directory_sync(destination: &Path) -> bool {
    DIRECTORY_SYNC_FAILURE.with(|failure| {
        let mut failure = failure.borrow_mut();
        let should_fail = match failure.as_mut() {
            Some((configured_path, remaining)) if configured_path == destination => {
                if *remaining == 1 {
                    true
                } else {
                    *remaining -= 1;
                    false
                }
            }
            _ => false,
        };
        if should_fail {
            *failure = None;
        }
        should_fail
    })
}

struct SnapshotTempFile {
    path: PathBuf,
    file: Option<File>,
}

impl SnapshotTempFile {
    fn create(destination: &Path) -> io::Result<Self> {
        for _ in 0..TEMP_FILE_CREATE_ATTEMPTS {
            let sequence = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let suffix = format!(".{}.{}.tmp", process::id(), sequence);
            let path = snapshot_sibling(destination, &suffix)?;

            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(file) => {
                    return Ok(Self {
                        path,
                        file: Some(file),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }

        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not allocate a unique temporary snapshot file",
        ))
    }

    fn file_mut(&mut self) -> &mut File {
        self.file
            .as_mut()
            .expect("temporary snapshot file is open while it is written")
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn close(&mut self) {
        self.file.take();
    }

    fn disarm(&mut self) {
        self.file.take();
        self.path = PathBuf::new();
    }
}

impl Drop for SnapshotTempFile {
    fn drop(&mut self) {
        self.file.take();
        if !self.path.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

struct SnapshotBackup {
    path: PathBuf,
    armed: bool,
}

impl SnapshotBackup {
    fn capture(destination: &Path) -> io::Result<Option<Self>> {
        let backup_path = snapshot_sibling(destination, ".rollback")?;
        match fs::metadata(&backup_path) {
            Ok(_) => {
                if StateSnapshot::load(destination).is_err() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        format!(
                            "rollback snapshot {} exists while the authoritative snapshot is unreadable",
                            backup_path.display()
                        ),
                    ));
                }
                fs::remove_file(&backup_path)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }

        let mut source = match File::open(destination) {
            Ok(source) => source,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let mut backup_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&backup_path)?;
        let backup = Self {
            path: backup_path,
            armed: true,
        };
        io::copy(&mut source, &mut backup_file)?;
        backup_file.flush()?;
        backup_file.sync_all()?;
        drop(backup_file);
        StateSnapshot::load(backup.path())?;
        Ok(Some(backup))
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn restore(mut self, destination: &Path) -> io::Result<()> {
        if let Err(error) = publish_snapshot(&self.path, destination) {
            self.armed = false;
            return Err(io::Error::new(
                error.kind(),
                format!(
                    "could not restore the previous snapshot; rollback retained at {}: {error}",
                    self.path.display()
                ),
            ));
        }
        self.armed = false;
        Ok(())
    }

    fn cleanup(mut self) -> io::Result<()> {
        match fs::remove_file(&self.path) {
            Ok(()) => {
                self.armed = false;
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.armed = false;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }
}

impl Drop for SnapshotBackup {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn rollback_publication(
    previous: Option<SnapshotBackup>,
    destination: &Path,
    directory: &SnapshotDirectory,
) -> io::Result<()> {
    match previous {
        Some(backup) => backup.restore(destination)?,
        None => match fs::remove_file(destination) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        },
    }
    sync_snapshot_directory(directory, destination)
}

fn publication_error(publication: io::Error, rollback: io::Result<()>) -> io::Error {
    match rollback {
        Ok(()) => publication,
        Err(rollback_error) => io::Error::other(format!(
            "snapshot publication failed: {publication}; rollback failed: {rollback_error}"
        )),
    }
}

#[cfg(unix)]
fn publish_snapshot(temp_path: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(temp_path, destination)
}

#[cfg(windows)]
fn publish_snapshot(temp_path: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVE_FILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVE_FILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    let existing: Vec<u16> = temp_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let new: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let result = unsafe {
        MoveFileExW(
            existing.as_ptr(),
            new.as_ptr(),
            MOVE_FILE_REPLACE_EXISTING | MOVE_FILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(all(not(unix), not(windows)))]
fn publish_snapshot(_temp_path: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "atomic snapshot publication is not implemented on this platform",
    ))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use dawn_core::Position;

    fn sample_snapshot() -> StateSnapshot {
        StateSnapshot {
            node_id: NodeId(0),
            sector_id: dawn_core::SectorId(0),
            bounds: SectorBounds::centered(SectorBounds::DEFAULT_HALF),
            log_index: 42,
            tick: Tick(10),
            id_counter: 5,
            player_id_counter: 3,
            ships: vec![ShipSnapshot {
                ship_id: ShipId::new(NodeId(0), 0),
                ship_type_id: ShipTypeId(1),
                absolute_position: Some(AbsolutePosition::new(100.0, 200.0, 300.0)),
                position: Position::new(100.0, 200.0, 300.0),
                anchor: dawn_core::AnchorId(0),
                velocity: dawn_core::Velocity::new(1.0, 0.0, 0.0),
                current_shield: 50.0,
                current_armor: 60.0,
                current_hull: 70.0,
                is_destroyed: false,
                capacitor: Some(250.0),
                fitting: FittingSnapshot::empty(),
                tackled_by: vec![],
                inventory: std::collections::BTreeMap::from([(
                    dawn_core::ItemId::Module(dawn_core::ModuleId(7)),
                    1,
                )]),
            }],
            completed_incoming_transits: Vec::new(),
            docked_ships: BTreeMap::from([(ShipId::new(NodeId(0), 0), dawn_core::StationId(0))]),
            docked_players: BTreeMap::from([(dawn_core::PlayerId(9), dawn_core::StationId(0))]),
        }
    }

    fn assert_no_publication_artifacts(dir: &Path) {
        let leftovers: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .filter(|name| {
                let name = name.to_string_lossy();
                name.ends_with(".tmp") || name.ends_with(".rollback")
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "snapshot publication artifacts were not cleaned up: {leftovers:?}"
        );
    }

    /// Re-encoding what postcard decoded must reproduce the original bytes.
    ///
    /// Asserted on the encoded form rather than field by field on purpose: a
    /// field-by-field list is itself hand-maintained, so it goes stale in
    /// exactly the way this whole seam exists to prevent. `sample_snapshot`
    /// gives every field a non-default value, and its struct literal stops
    /// compiling when `StateSnapshot` grows one.
    #[test]
    fn snapshot_round_trips_through_postcard_without_data_loss() {
        let original = sample_snapshot();
        let bytes = postcard::to_stdvec(&original).unwrap();
        let restored: StateSnapshot = postcard::from_bytes(&bytes).unwrap();

        assert_eq!(postcard::to_stdvec(&restored).unwrap(), bytes);
    }

    /// postcard is not self-describing: struct fields are read positionally,
    /// so a buffer written from a different field list cannot be decoded and
    /// `#[serde(default)]` does not rescue it. This pins the behaviour the
    /// format-compatibility note on `StateSnapshot` depends on — if it ever
    /// stops holding, that note (and ADR-0017) needs revisiting.
    #[test]
    fn a_snapshot_written_with_fewer_fields_fails_to_load() {
        #[derive(Serialize)]
        struct Truncated {
            node_id: NodeId,
        }

        let bytes = postcard::to_stdvec(&Truncated { node_id: NodeId(0) }).unwrap();
        assert!(postcard::from_bytes::<StateSnapshot>(&bytes).is_err());
    }

    #[test]
    fn first_publication_survives_save_and_load_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.bin");

        let original = sample_snapshot();
        original.save(&path).unwrap();

        let restored = StateSnapshot::load(&path).unwrap();
        assert_eq!(restored.log_index, original.log_index);
        assert_eq!(restored.tick, original.tick);
        assert_eq!(restored.id_counter, original.id_counter);
        assert_no_publication_artifacts(dir.path());
    }

    #[test]
    fn publication_replaces_an_existing_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.bin");

        let mut previous = sample_snapshot();
        previous.log_index = 7;
        previous.tick = Tick(3);
        previous.save(&path).unwrap();

        let mut replacement = sample_snapshot();
        replacement.log_index = 99;
        replacement.tick = Tick(25);
        replacement.save(&path).unwrap();

        let restored = StateSnapshot::load(&path).unwrap();
        assert_eq!(restored.log_index, replacement.log_index);
        assert_eq!(restored.tick, replacement.tick);
        assert_no_publication_artifacts(dir.path());
    }

    #[test]
    fn failure_before_replacement_preserves_the_previous_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.bin");

        let mut previous = sample_snapshot();
        previous.log_index = 7;
        previous.save(&path).unwrap();
        let previous_bytes = fs::read(&path).unwrap();

        let mut replacement = sample_snapshot();
        replacement.log_index = 99;
        let error = replacement
            .save_with_before_publish(&path, |_| {
                Err(io::Error::other("injected failure before replacement"))
            })
            .unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(fs::read(&path).unwrap(), previous_bytes);
        assert_eq!(StateSnapshot::load(&path).unwrap().log_index, 7);
        assert_no_publication_artifacts(dir.path());
    }

    #[test]
    fn directory_sync_failure_after_replacement_restores_the_previous_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.bin");

        let mut previous = sample_snapshot();
        previous.log_index = 7;
        previous.tick = Tick(3);
        previous.save(&path).unwrap();
        let previous_bytes = fs::read(&path).unwrap();

        let mut replacement = sample_snapshot();
        replacement.log_index = 99;
        replacement.tick = Tick(25);
        inject_directory_sync_failure(&path, 2);
        let error = replacement.save(&path).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(fs::read(&path).unwrap(), previous_bytes);
        assert_eq!(StateSnapshot::load(&path).unwrap().log_index, 7);
        assert_no_publication_artifacts(dir.path());
    }

    #[test]
    fn directory_sync_failure_during_first_publication_restores_absence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.bin");

        inject_directory_sync_failure(&path, 1);
        let error = sample_snapshot().save(&path).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert!(!path.exists());
        assert_no_publication_artifacts(dir.path());
    }
}
