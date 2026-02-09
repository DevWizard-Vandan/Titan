//! Snapshot persistence for crash recovery.
//!
//! Implements deterministic snapshotting triggered every N orders.
//! Snapshots are written by a background thread to avoid blocking the hot path.

use std::fs::{self, File};
use std::io::{self, Write, Read};
use std::path::{Path, PathBuf};
use std::thread;

use crossbeam_channel::{Receiver, Sender, bounded};

/// Snapshot file magic bytes
const SNAPSHOT_MAGIC: [u8; 4] = [b'T', b'I', b'T', b'N'];

/// Snapshot header format
#[repr(C, packed)]
#[derive(Clone, Copy)]
pub struct SnapshotHeader {
    pub magic: [u8; 4],
    pub version: u32,
    pub timestamp_ns: u64,
    pub sequence_num: u64,
    pub bid_count: u32,
    pub ask_count: u32,
}

/// Message sent to snapshot thread
pub enum SnapshotMessage {
    /// Request a snapshot with the given data
    WriteSnapshot {
        sequence_num: u64,
        timestamp_ns: u64,
        data: Vec<u8>,
    },
    /// Shutdown the snapshot thread
    Shutdown,
}

/// Snapshot manager handle
pub struct SnapshotManager {
    sender: Sender<SnapshotMessage>,
    data_dir: PathBuf,
}

impl SnapshotManager {
    /// Create a new snapshot manager.
    /// Spawns a background thread for writing snapshots.
    pub fn new(data_dir: impl AsRef<Path>) -> io::Result<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();
        
        // Ensure data directory exists
        fs::create_dir_all(&data_dir)?;
        
        let (sender, receiver) = bounded::<SnapshotMessage>(4);
        let dir_clone = data_dir.clone();
        
        thread::Builder::new()
            .name("titan-snapshot".to_string())
            .spawn(move || {
                snapshot_writer_loop(receiver, dir_clone);
            })
            .expect("Failed to spawn snapshot thread");
        
        Ok(Self { sender, data_dir })
    }
    
    /// Request a snapshot to be written.
    /// This is non-blocking and returns immediately.
    pub fn request_snapshot(&self, sequence_num: u64, timestamp_ns: u64, data: Vec<u8>) {
        let msg = SnapshotMessage::WriteSnapshot {
            sequence_num,
            timestamp_ns,
            data,
        };
        
        // Use try_send to avoid blocking the hot path
        if self.sender.try_send(msg).is_err() {
            eprintln!("⚠️  Snapshot queue full, skipping snapshot at seq {}", sequence_num);
        }
    }
    
    /// Find and load the latest snapshot.
    pub fn load_latest(&self) -> io::Result<Option<(u64, Vec<u8>)>> {
        load_latest_snapshot(&self.data_dir)
    }
    
    /// Shutdown the snapshot manager.
    pub fn shutdown(&self) {
        let _ = self.sender.send(SnapshotMessage::Shutdown);
    }
}

/// Background thread loop for writing snapshots.
fn snapshot_writer_loop(receiver: Receiver<SnapshotMessage>, data_dir: PathBuf) {
    for msg in receiver {
        match msg {
            SnapshotMessage::WriteSnapshot { sequence_num, timestamp_ns, data } => {
                let filename = format!("snapshot_{:012}.bin", sequence_num);
                let path = data_dir.join(&filename);
                
                match write_snapshot_file(&path, sequence_num, timestamp_ns, &data) {
                    Ok(_) => {
                        println!("💾 Snapshot written: {} ({} bytes)", filename, data.len());
                        
                        // Cleanup old snapshots (keep last 5)
                        if let Err(e) = cleanup_old_snapshots(&data_dir, 5) {
                            eprintln!("⚠️  Failed to cleanup old snapshots: {}", e);
                        }
                    }
                    Err(e) => {
                        eprintln!("❌ Failed to write snapshot: {}", e);
                    }
                }
            }
            SnapshotMessage::Shutdown => {
                println!("📦 Snapshot thread shutting down");
                break;
            }
        }
    }
}

/// Write a snapshot file with header.
fn write_snapshot_file(
    path: &Path,
    sequence_num: u64,
    timestamp_ns: u64,
    data: &[u8],
) -> io::Result<()> {
    let mut file = File::create(path)?;
    
    // Write header
    file.write_all(&SNAPSHOT_MAGIC)?;
    file.write_all(&1u32.to_le_bytes())?;  // version
    file.write_all(&timestamp_ns.to_le_bytes())?;
    file.write_all(&sequence_num.to_le_bytes())?;
    
    // Write data
    file.write_all(data)?;
    file.sync_all()?;
    
    Ok(())
}

/// Load the latest snapshot from the data directory.
pub fn load_latest_snapshot(data_dir: &Path) -> io::Result<Option<(u64, Vec<u8>)>> {
    let mut snapshots: Vec<_> = fs::read_dir(data_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|s| s.starts_with("snapshot_") && s.ends_with(".bin"))
                .unwrap_or(false)
        })
        .collect();
    
    if snapshots.is_empty() {
        return Ok(None);
    }
    
    // Sort by filename (sequence number) descending
    snapshots.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    
    let latest = &snapshots[0];
    let mut file = File::open(latest.path())?;
    
    // Read and validate header
    let mut magic = [0u8; 4];
    file.read_exact(&mut magic)?;
    if magic != SNAPSHOT_MAGIC {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid snapshot magic",
        ));
    }
    
    let mut version = [0u8; 4];
    file.read_exact(&mut version)?;
    
    let mut timestamp = [0u8; 8];
    file.read_exact(&mut timestamp)?;
    
    let mut sequence = [0u8; 8];
    file.read_exact(&mut sequence)?;
    let sequence_num = u64::from_le_bytes(sequence);
    
    // Read remaining data
    let mut data = Vec::new();
    file.read_to_end(&mut data)?;
    
    Ok(Some((sequence_num, data)))
}

/// Remove old snapshots, keeping only the N most recent.
fn cleanup_old_snapshots(data_dir: &Path, keep: usize) -> io::Result<()> {
    let mut snapshots: Vec<_> = fs::read_dir(data_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|s| s.starts_with("snapshot_") && s.ends_with(".bin"))
                .unwrap_or(false)
        })
        .collect();
    
    if snapshots.len() <= keep {
        return Ok(());
    }
    
    // Sort by filename descending (newest first)
    snapshots.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    
    // Remove old ones
    for snapshot in snapshots.into_iter().skip(keep) {
        fs::remove_file(snapshot.path())?;
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    
    #[test]
    fn test_snapshot_roundtrip() {
        let temp_dir = env::temp_dir().join("titan_test_snapshots");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();
        
        let test_data = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let path = temp_dir.join("snapshot_000000000100.bin");
        
        write_snapshot_file(&path, 100, 12345678, &test_data).unwrap();
        
        let (seq, data) = load_latest_snapshot(&temp_dir).unwrap().unwrap();
        assert_eq!(seq, 100);
        assert_eq!(data, test_data);
        
        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
