//! `StateSnapshot` — current point-in-time snapshot implementation for a
//! `SimulationNode`.
//!
//! # Recovery contract status (ADR-0049 / #284)
//!
//! This file still implements the **pre-ADR-0049 snapshot/EventStore recovery
//! path**. Its `log_index`, postcard layout, and restore call sites describe the
//! current migration baseline; they are not the final exact-recovery contract.
//!
//! ADR-0049 has selected the target operational model:
//!
//! ```text
//! newest complete compatible versioned checkpoint
//!     + every contiguous committed authoritative RecoveryDelta after it
//! ```
//!
//! Eventless Ticks, `active_ship`, capacitor/lock/module counters, queued future
//! intent, and other authoritative values are covered by that recovery stream even
//! when no public `DomainEvent` exists. Public Event replay remains useful for its
//! supported projection/audit/legacy purposes, but is not the complete exact-state
//! reducer.
//!
//! # Current legacy restore procedure
//!
//! Until #271/#272/#284 replace this path, the implementation below still:
//!
//! 1. loads `StateSnapshot` from disk;
//! 2. opens the `FileEventStore` for the same Sector;
//! 3. calls `SimulationNode::restore_from_test(store, &snapshot, galaxy, &modules, &ship_types)`;
//! 4. restores the snapshot state and replays current EventStore tail behavior.
//!
//! Code that depends on this behavior must treat it as migration debt, not infer
//! the future journal/checkpoint API from it.
//!
//! # Publication guarantee retained by ADR-0049
//!
//! The crash-safe publication mechanics in this file remain a required property:
//! encode/validate replacement material, write it to a sibling file, flush+sync,
//! atomically publish it, sync the directory, and preserve/restore a readable
//! rollback copy on handled failure. #284 requires the future versioned checkpoint
//! implementation to retain or strengthen this guarantee.
//!
//! The current struct includes values such as position, capacitor, and tackle state.
//! Under ADR-0049 these are **authoritative recovery values**, not merely transient
//! values that may be recomputed arbitrarily after restart. The final checkpoint
//! schema and authoritative journal position are implemented by #271/#284; old
//! pre-release snapshot compatibility is not required.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use dawn_core::{
    fitting::FittingSnapshot, AbsolutePosition, NodeId, Position, SectorBounds, SectorId, ShipId,
    ShipTypeId, Tick, Velocity,
};
use serde::{Deserialize, Serialize};

const TEMP_FILE_CREATE_ATTEMPTS: usize = 128;
#[cfg(unix)]
const INITIAL_SNAPSHOT_MODE: u32 = 0o600;
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

/// State of a single Ship in the current legacy snapshot format.
///
/// Captures the ECS values the current snapshot path restores directly. ADR-0049
/// requires the eventual versioned checkpoint + RecoveryDelta tail to preserve all
/// authoritative values needed for exact recovery, whether or not a public Event
/// represents them.
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
/// #276 replaces this legacy receipt shape with the final durable Transit Saga.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletedIncomingTransit {
    pub ship_id: ShipId,
    pub from: SectorId,
    pub to: SectorId,
    pub request_tick: Tick,
}

// ── Node-level snapshot ───────────────────────────────────────────────────────

/// Complete state of a `SimulationNode` in the current snapshot format at a
/// specific legacy EventStore `log_index`.
///
/// This struct is the current implementation baseline, not the final ADR-0049
/// checkpoint manifest. #284/#271 replace the implicit binary-version contract
/// with an explicit versioned/fingerprinted checkpoint whose coverage is an
/// authoritative recovery-journal position rather than a public-event-only index.
///
/// # Current format compatibility (ADR-0017 legacy path)
///
/// The on-disk format is **version-locked to the binary**: postcard is not
/// self-describing, so fields are read positionally and a snapshot written by
/// a different field list fails to load with `DeserializeUnexpectedEnd`.
/// `#[serde(default)]` does **not** grant field-level back-compat here the way
/// it would for a self-describing format like JSON — it is a no-op on this
/// path. ADR-0049 explicitly does not require old pre-release snapshot
/// compatibility; the replacement format must instead reject incompatible data
/// clearly using explicit version/fingerprint metadata.
///
/// Which fields belong in this current struct is enforced from both sides:
/// `SimulationNode::take_snapshot` destructures the node exhaustively, and
/// `SimulationNode::apply_snapshot` destructures this struct exhaustively, so
/// adding a field to either is a compile error until it is handled. #284/#275
/// replace this broad manual boundary with the final authority inventory/state
/// owners.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateSnapshot {
    /// The node that produced this snapshot.
    pub node_id: NodeId,
    /// The Sector this node manages.
    pub sector_id: SectorId,
    /// Spatial bounds of the Sector.
    pub bounds: SectorBounds,
    /// Legacy EventStore coverage: events with index < `log_index` are covered
    /// by this current snapshot and events at/after it are replayed by the
    /// current restore path. ADR-0049 replaces this with an authoritative
    /// recovery-journal covered position in the versioned checkpoint manifest.
    pub log_index: u64,
    /// Logical tick at the time of the snapshot.
    pub tick: Tick,
    /// Next value for `SimulationNode::id_counter`.
    /// Must be restored to prevent EntityId reuse (INV-004).
    pub id_counter: u64,
    /// Next value for `SimulationNode::player_id_counter`.
    ///
    /// Must be restored so a freshly-admitted client cannot receive a
    /// `PlayerId` already handed out before restart.
    pub player_id_counter: u64,
    /// State of every Ship in the Sector at the snapshot instant.
    pub ships: Vec<ShipSnapshot>,
    /// Authoritative Ship -> Player ownership bindings, including Ships that
    /// arrived through Transit after their original Sector log was compacted.
    pub owners: BTreeMap<dawn_core::ShipId, dawn_core::PlayerId>,
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
        let protection = SnapshotProtection::capture(path)?;

        let mut temp = SnapshotTempFile::create(path, &protection)?;
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

        let previous = SnapshotBackup::capture(path, &protection)?;
        if previous.is_some() {
            sync_snapshot_directory(&directory, path)?;
        }

        before_publish(temp.path())?;
        let replacing_existing = previous.is_some();
        if let Err(error) = publish_snapshot(temp.path(), path, replacing_existing) {
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

#[derive(Debug, Clone)]
struct SnapshotProtection {
    #[cfg(unix)]
    mode: u32,
    #[cfg(windows)]
    source: Option<PathBuf>,
}

impl SnapshotProtection {
    fn capture(destination: &Path) -> io::Result<Self> {
        match fs::metadata(destination) {
            Ok(metadata) => {
                #[cfg(unix)]
                {
                    Ok(Self {
                        mode: metadata.permissions().mode() & 0o7777,
                    })
                }
                #[cfg(windows)]
                {
                    let _ = metadata;
                    Ok(Self {
                        source: Some(destination.to_path_buf()),
                    })
                }
                #[cfg(all(not(unix), not(windows)))]
                {
                    let _ = metadata;
                    Ok(Self {})
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                #[cfg(unix)]
                {
                    Ok(Self {
                        mode: INITIAL_SNAPSHOT_MODE,
                    })
                }
                #[cfg(windows)]
                {
                    Ok(Self { source: None })
                }
                #[cfg(all(not(unix), not(windows)))]
                {
                    Ok(Self {})
                }
            }
            Err(error) => Err(error),
        }
    }

    fn create_new(&self, path: &Path) -> io::Result<File> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        options.mode(self.mode);

        let file = options.open(path)?;
        let protection_result = self.apply_to_new_file(&file, path);
        if let Err(error) = protection_result {
            drop(file);
            let _ = fs::remove_file(path);
            return Err(error);
        }
        Ok(file)
    }

    fn apply_to_new_file(&self, file: &File, _path: &Path) -> io::Result<()> {
        #[cfg(unix)]
        {
            file.set_permissions(fs::Permissions::from_mode(self.mode))
        }
        #[cfg(windows)]
        {
            if let Some(source) = &self.source {
                copy_windows_dacl(source, _path)?;
            }
            let _ = file;
            Ok(())
        }
        #[cfg(all(not(unix), not(windows)))]
        {
            let _ = (file, _path);
            Ok(())
        }
    }
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
            // Windows replacement uses the operating system's atomic file
            // replacement primitive. Other platforms are rejected below.
            Ok(())
        }
    }
}

fn sync_snapshot_directory(directory: &SnapshotDirectory, _destination: &Path) -> io::Result<()> {
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
    fn create(destination: &Path, protection: &SnapshotProtection) -> io::Result<Self> {
        for _ in 0..TEMP_FILE_CREATE_ATTEMPTS {
            let sequence = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let suffix = format!(".{}.{}.tmp", process::id(), sequence);
            let path = snapshot_sibling(destination, &suffix)?;

            match protection.create_new(&path) {
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
    fn capture(destination: &Path, protection: &SnapshotProtection) -> io::Result<Option<Self>> {
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
        let backup = Self {
            path: backup_path,
            armed: true,
        };
        let mut backup_file = protection.create_new(backup.path())?;
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
        if let Err(error) = publish_snapshot(&self.path, destination, true) {
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
fn publish_snapshot(
    temp_path: &Path,
    destination: &Path,
    _replacing_existing: bool,
) -> io::Result<()> {
    fs::rename(temp_path, destination)
}

#[cfg(windows)]
fn publish_snapshot(
    temp_path: &Path,
    destination: &Path,
    replacing_existing: bool,
) -> io::Result<()> {
    use std::ptr;

    const MOVE_FILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
        fn ReplaceFileW(
            replaced_file_name: *const u16,
            replacement_file_name: *const u16,
            backup_file_name: *const u16,
            replace_flags: u32,
            exclude: *mut std::ffi::c_void,
            reserved: *mut std::ffi::c_void,
        ) -> i32;
    }

    let temp = wide_path(temp_path);
    let destination = wide_path(destination);
    let result = if replacing_existing {
        unsafe {
            ReplaceFileW(
                destination.as_ptr(),
                temp.as_ptr(),
                ptr::null(),
                0,
                ptr::null_mut(),
                ptr::null_mut(),
            )
        }
    } else {
        unsafe { MoveFileExW(temp.as_ptr(), destination.as_ptr(), MOVE_FILE_WRITE_THROUGH) }
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn copy_windows_dacl(source: &Path, destination: &Path) -> io::Result<()> {
    use std::ptr;

    const DACL_SECURITY_INFORMATION: u32 = 0x0000_0004;
    const ERROR_INSUFFICIENT_BUFFER: i32 = 122;

    #[link(name = "Advapi32")]
    unsafe extern "system" {
        fn GetFileSecurityW(
            file_name: *const u16,
            requested_information: u32,
            security_descriptor: *mut std::ffi::c_void,
            length: u32,
            length_needed: *mut u32,
        ) -> i32;
        fn SetFileSecurityW(
            file_name: *const u16,
            security_information: u32,
            security_descriptor: *mut std::ffi::c_void,
        ) -> i32;
    }

    let source = wide_path(source);
    let destination = wide_path(destination);
    let mut length_needed = 0;
    let first = unsafe {
        GetFileSecurityW(
            source.as_ptr(),
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            0,
            &mut length_needed,
        )
    };
    if first != 0 {
        return Err(io::Error::other(
            "GetFileSecurityW unexpectedly succeeded without a descriptor buffer",
        ));
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER) {
        return Err(error);
    }

    let mut descriptor = vec![0_u8; length_needed as usize];
    let fetched = unsafe {
        GetFileSecurityW(
            source.as_ptr(),
            DACL_SECURITY_INFORMATION,
            descriptor.as_mut_ptr().cast(),
            length_needed,
            &mut length_needed,
        )
    };
    if fetched == 0 {
        return Err(io::Error::last_os_error());
    }

    let applied = unsafe {
        SetFileSecurityW(
            destination.as_ptr(),
            DACL_SECURITY_INFORMATION,
            descriptor.as_mut_ptr().cast(),
        )
    };
    if applied == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(windows)]
fn wide_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(all(not(unix), not(windows)))]
fn publish_snapshot(
    _temp_path: &Path,
    _destination: &Path,
    _replacing_existing: bool,
) -> io::Result<()> {
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

    #[cfg(unix)]
    fn file_mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }

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
            owners: BTreeMap::new(),
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
    /// current format-compatibility note on `StateSnapshot` depends on. The
    /// ADR-0049 replacement instead requires an explicit version/fingerprint.
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
        #[cfg(unix)]
        assert_eq!(file_mode(&path), INITIAL_SNAPSHOT_MODE);
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

    #[cfg(unix)]
    #[test]
    fn replacement_preserves_existing_mode_on_all_publication_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snapshot.bin");

        let mut previous = sample_snapshot();
        previous.log_index = 7;
        previous.save(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();

        let mut replacement = sample_snapshot();
        replacement.log_index = 99;
        replacement
            .save_with_before_publish(&path, |temp_path| {
                assert_eq!(file_mode(temp_path), 0o600);
                let rollback_path = snapshot_sibling(&path, ".rollback").unwrap();
                assert_eq!(file_mode(&rollback_path), 0o600);
                Ok(())
            })
            .unwrap();

        assert_eq!(file_mode(&path), 0o600);
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
