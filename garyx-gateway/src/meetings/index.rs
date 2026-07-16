use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crc32fast::Hasher;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::MeetingError;
use super::log::{INDEX_STRIDE, LogScan, SparseOffset};

const INDEX_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OffsetIndex {
    pub version: u32,
    pub log_epoch: i64,
    pub log_byte_len: u64,
    pub latest_seq: i64,
    pub offsets: Vec<SparseOffset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexPayload {
    version: u32,
    log_epoch: i64,
    log_byte_len: u64,
    latest_seq: i64,
    offsets: Vec<(i64, u64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexFile {
    version: u32,
    log_epoch: i64,
    log_byte_len: u64,
    latest_seq: i64,
    crc32: u32,
    offsets: Vec<(i64, u64)>,
}

impl OffsetIndex {
    pub(crate) fn from_scan(scan: &LogScan) -> Self {
        Self {
            version: INDEX_VERSION,
            log_epoch: scan.epoch,
            log_byte_len: scan.byte_len,
            latest_seq: scan.latest_seq,
            offsets: scan.offsets.clone(),
        }
    }

    fn payload(&self) -> IndexPayload {
        IndexPayload {
            version: self.version,
            log_epoch: self.log_epoch,
            log_byte_len: self.log_byte_len,
            latest_seq: self.latest_seq,
            offsets: self
                .offsets
                .iter()
                .map(|entry| (entry.seq, entry.offset))
                .collect(),
        }
    }

    fn encoded(&self) -> Result<Vec<u8>, MeetingError> {
        let payload = self.payload();
        let payload_bytes = serde_json::to_vec(&payload)?;
        let mut hasher = Hasher::new();
        hasher.update(&payload_bytes);
        serde_json::to_vec(&IndexFile {
            version: payload.version,
            log_epoch: payload.log_epoch,
            log_byte_len: payload.log_byte_len,
            latest_seq: payload.latest_seq,
            crc32: hasher.finalize(),
            offsets: payload.offsets,
        })
        .map_err(MeetingError::from)
    }
}

pub(crate) fn index_path(entity_dir: &Path) -> PathBuf {
    entity_dir.join("index.bin")
}

pub(crate) fn persist_index(entity_dir: &Path, index: &OffsetIndex) -> Result<(), MeetingError> {
    fs::create_dir_all(entity_dir)
        .map_err(|error| MeetingError::io("create meeting index directory", error))?;
    let final_path = index_path(entity_dir);
    let temp_path = entity_dir.join(format!(".index.{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = File::create(&temp_path)
            .map_err(|error| MeetingError::io("create meeting index temp file", error))?;
        file.write_all(&index.encoded()?)
            .map_err(|error| MeetingError::io("write meeting index", error))?;
        file.sync_data()
            .map_err(|error| MeetingError::io("fdatasync meeting index", error))?;
        fs::rename(&temp_path, &final_path)
            .map_err(|error| MeetingError::io("publish meeting index", error))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

pub(crate) fn load_index(
    entity_dir: &Path,
    expected_epoch: i64,
    expected_log_len: u64,
    expected_latest: i64,
) -> Result<Option<OffsetIndex>, MeetingError> {
    let actual_log_len = match fs::metadata(entity_dir.join("segments.jsonl")) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(MeetingError::io("inspect indexed meeting log", error)),
    };
    if actual_log_len != expected_log_len {
        return Ok(None);
    }
    let raw = match fs::read(index_path(entity_dir)) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(MeetingError::io("read meeting index", error)),
    };
    let file: IndexFile = match serde_json::from_slice(&raw) {
        Ok(file) => file,
        Err(_) => return Ok(None),
    };
    let payload = IndexPayload {
        version: file.version,
        log_epoch: file.log_epoch,
        log_byte_len: file.log_byte_len,
        latest_seq: file.latest_seq,
        offsets: file.offsets.clone(),
    };
    let encoded = serde_json::to_vec(&payload)?;
    let mut hasher = Hasher::new();
    hasher.update(&encoded);
    if file.version != INDEX_VERSION
        || file.log_epoch != expected_epoch
        || file.log_byte_len != expected_log_len
        || file.latest_seq != expected_latest
        || file.crc32 != hasher.finalize()
        || !valid_offsets(&file.offsets, expected_latest, expected_log_len)
    {
        return Ok(None);
    }
    Ok(Some(OffsetIndex {
        version: file.version,
        log_epoch: file.log_epoch,
        log_byte_len: file.log_byte_len,
        latest_seq: file.latest_seq,
        offsets: file
            .offsets
            .into_iter()
            .map(|(seq, offset)| SparseOffset { seq, offset })
            .collect(),
    }))
}

fn valid_offsets(offsets: &[(i64, u64)], latest_seq: i64, log_len: u64) -> bool {
    if latest_seq < 0
        || offsets.len() != usize::try_from(latest_seq / INDEX_STRIDE).unwrap_or(usize::MAX)
    {
        return false;
    }
    let mut previous_offset = 0u64;
    offsets.iter().enumerate().all(|(index, (seq, offset))| {
        let expected_seq = i64::try_from(index + 1)
            .ok()
            .and_then(|ordinal| ordinal.checked_mul(INDEX_STRIDE));
        let valid = Some(*seq) == expected_seq
            && *offset < log_len
            && (*offset > previous_offset || index == 0);
        previous_offset = *offset;
        valid
    })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    fn sample_index() -> OffsetIndex {
        OffsetIndex {
            version: INDEX_VERSION,
            log_epoch: 3,
            log_byte_len: 8_000,
            latest_seq: 128,
            offsets: vec![
                SparseOffset {
                    seq: 64,
                    offset: 3_000,
                },
                SparseOffset {
                    seq: 128,
                    offset: 7_500,
                },
            ],
        }
    }

    #[test]
    fn persisted_index_rejects_torn_stale_version_length_and_crc() {
        let temp = tempdir().expect("temp");
        let index = sample_index();
        fs::write(temp.path().join("segments.jsonl"), vec![0u8; 8_000]).expect("synthetic log");
        persist_index(temp.path(), &index).expect("persist");
        assert_eq!(
            load_index(temp.path(), 3, 8_000, 128)
                .expect("load")
                .expect("valid"),
            index
        );
        assert!(
            load_index(temp.path(), 4, 8_000, 128)
                .expect("epoch")
                .is_none()
        );
        assert!(
            load_index(temp.path(), 3, 8_001, 128)
                .expect("length")
                .is_none()
        );

        let path = index_path(temp.path());
        let mut incomplete = index.clone();
        incomplete.offsets.pop();
        persist_index(temp.path(), &incomplete).expect("incomplete index");
        assert!(
            load_index(temp.path(), 3, 8_000, 128)
                .expect("logical sparse validation")
                .is_none()
        );

        persist_index(temp.path(), &index).expect("restore before torn");
        let mut raw = fs::read(&path).expect("raw");
        raw.truncate(raw.len() / 2);
        fs::write(&path, raw).expect("torn");
        assert!(
            load_index(temp.path(), 3, 8_000, 128)
                .expect("torn load")
                .is_none()
        );

        persist_index(temp.path(), &index).expect("restore");
        let mut file: IndexFile =
            serde_json::from_slice(&fs::read(&path).expect("read")).expect("decode");
        file.version += 1;
        fs::write(&path, serde_json::to_vec(&file).expect("encode")).expect("version");
        assert!(
            load_index(temp.path(), 3, 8_000, 128)
                .expect("version load")
                .is_none()
        );

        persist_index(temp.path(), &index).expect("restore for crc");
        let mut file: IndexFile =
            serde_json::from_slice(&fs::read(&path).expect("read")).expect("decode");
        file.crc32 ^= 1;
        fs::write(&path, serde_json::to_vec(&file).expect("encode")).expect("crc");
        assert!(
            load_index(temp.path(), 3, 8_000, 128)
                .expect("crc load")
                .is_none()
        );
    }
}
