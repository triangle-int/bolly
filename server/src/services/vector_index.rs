//! Versioned derived vector cache. Memory files remain the source of truth.
//!
//! A fixed envelope authenticates a bounded Postcard payload before decoding.
//! Handles share per-companion locks; publish new memory state only after the
//! atomic replacement commits, while searches retain the prior committed view.
//! Like the rest of the workspace, this cache assumes one server process owns it.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const FORMAT_VERSION: u32 = 1;
const MAGIC: &[u8; 8] = b"BOLLYVIX";
const HEADER_BYTES: usize = MAGIC.len() + 8 + 32;
const MAX_INDEX_FILE_BYTES: usize = 128 * 1024 * 1024;
const MAX_PAYLOAD_BYTES: usize = MAX_INDEX_FILE_BYTES - HEADER_BYTES;
const MAX_RECORDS: usize = 100_000;
const MAX_METADATA_BYTES: usize = 1_024;
const MAX_STRING_BYTES: usize = 64 * 1024;
const MAX_VECTOR_DIMENSIONS: usize = 16_384;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Record {
    #[serde(deserialize_with = "deserialize_bounded_string")]
    pub path: String,
    #[serde(deserialize_with = "deserialize_bounded_string")]
    pub source_type: String,
    pub chunk_index: u32,
    #[serde(deserialize_with = "deserialize_bounded_string")]
    pub content_preview: String,
    #[serde(deserialize_with = "deserialize_bounded_optional_string")]
    pub upload_id: Option<String>,
    #[serde(deserialize_with = "deserialize_bounded_vector")]
    pub vector: Vec<f32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Index {
    version: u32,
    #[serde(deserialize_with = "deserialize_bounded_metadata")]
    provider: String,
    #[serde(deserialize_with = "deserialize_bounded_metadata")]
    model: String,
    dimensions: u32,
    #[serde(deserialize_with = "deserialize_bounded_records")]
    records: Vec<Record>,
    backfilled: bool,
}

impl Index {
    fn empty(provider: &str, model: &str, dimensions: u32) -> Self {
        Self {
            version: FORMAT_VERSION,
            provider: provider.into(),
            model: model.into(),
            dimensions,
            records: Vec::new(),
            backfilled: false,
        }
    }

    fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate(&self.provider, &self.model, self.dimensions)?;
        let payload = postcard::to_allocvec(self).map_err(|error| error.to_string())?;
        if payload.len() > MAX_PAYLOAD_BYTES {
            return Err("vector index payload is too large".into());
        }
        let mut encoded = Vec::with_capacity(HEADER_BYTES + payload.len());
        encoded.extend_from_slice(MAGIC);
        encoded.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        encoded.extend_from_slice(&Sha256::digest(&payload));
        encoded.extend_from_slice(&payload);
        Ok(encoded)
    }

    fn decode(bytes: &[u8], provider: &str, model: &str, dimensions: u32) -> Result<Self, String> {
        if bytes.len() > MAX_INDEX_FILE_BYTES || bytes.len() < HEADER_BYTES {
            return Err("invalid vector index envelope size".into());
        }
        if &bytes[..MAGIC.len()] != MAGIC {
            return Err("invalid vector index magic".into());
        }
        let length_offset = MAGIC.len();
        let payload_len = u64::from_le_bytes(
            bytes[length_offset..length_offset + 8]
                .try_into()
                .map_err(|_| "invalid vector index length")?,
        );
        let payload_len = usize::try_from(payload_len)
            .map_err(|_| "vector index payload length does not fit this platform")?;
        if payload_len > MAX_PAYLOAD_BYTES
            || HEADER_BYTES.checked_add(payload_len) != Some(bytes.len())
        {
            return Err("invalid vector index payload length or trailing bytes".into());
        }
        let checksum_offset = length_offset + 8;
        let payload = &bytes[HEADER_BYTES..];
        if Sha256::digest(payload).as_slice() != &bytes[checksum_offset..HEADER_BYTES] {
            return Err("vector index checksum mismatch".into());
        }
        let (index, remainder): (Self, _) =
            postcard::take_from_bytes(payload).map_err(|error| error.to_string())?;
        if !remainder.is_empty() {
            return Err("vector index payload has trailing bytes".into());
        }
        index.validate(provider, model, dimensions)?;
        Ok(index)
    }

    fn validate(&self, provider: &str, model: &str, dimensions: u32) -> Result<(), String> {
        if self.version != FORMAT_VERSION
            || self.provider != provider
            || self.model != model
            || self.dimensions != dimensions
        {
            return Err("incompatible vector index metadata".into());
        }
        if self.provider.len() > MAX_METADATA_BYTES || self.model.len() > MAX_METADATA_BYTES {
            return Err("vector index metadata is too large".into());
        }
        if self.dimensions == 0 || self.dimensions as usize > MAX_VECTOR_DIMENSIONS {
            return Err("vector index dimensions are outside the supported range".into());
        }
        if self.records.len() > MAX_RECORDS {
            return Err("vector index contains too many records".into());
        }
        for record in &self.records {
            for value in [
                record.path.as_str(),
                record.source_type.as_str(),
                record.content_preview.as_str(),
            ] {
                if value.len() > MAX_STRING_BYTES {
                    return Err("vector index record string is too large".into());
                }
            }
            if record
                .upload_id
                .as_ref()
                .is_some_and(|value| value.len() > MAX_STRING_BYTES)
            {
                return Err("vector index record string is too large".into());
            }
            validate_vector(&record.vector, dimensions)?;
        }
        Ok(())
    }
}

fn deserialize_bounded_metadata<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_string_with_limit(deserializer, MAX_METADATA_BYTES)
}

fn deserialize_bounded_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    deserialize_string_with_limit(deserializer, MAX_STRING_BYTES)
}

fn deserialize_string_with_limit<'de, D>(deserializer: D, limit: usize) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct BoundedStringVisitor(usize);
    impl serde::de::Visitor<'_> for BoundedStringVisitor {
        type Value = String;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(formatter, "a string no longer than {} bytes", self.0)
        }

        fn visit_borrowed_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            self.visit_str(value)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            if value.len() > self.0 {
                return Err(E::custom("vector index string exceeds its size limit"));
            }
            Ok(value.to_owned())
        }

        fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            if value.len() > self.0 {
                return Err(E::custom("vector index string exceeds its size limit"));
            }
            Ok(value)
        }
    }
    deserializer.deserialize_str(BoundedStringVisitor(limit))
}

fn deserialize_bounded_optional_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct OptionalStringVisitor;
    impl<'de> serde::de::Visitor<'de> for OptionalStringVisitor {
        type Value = Option<String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("an optional bounded string")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            deserialize_bounded_string(deserializer).map(Some)
        }
    }
    deserializer.deserialize_option(OptionalStringVisitor)
}

fn deserialize_bounded_records<'de, D>(deserializer: D) -> Result<Vec<Record>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct RecordsVisitor;
    impl<'de> serde::de::Visitor<'de> for RecordsVisitor {
        type Value = Vec<Record>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a bounded vector index record list")
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            if sequence.size_hint().is_some_and(|size| size > MAX_RECORDS) {
                return Err(serde::de::Error::custom(
                    "vector index contains too many records",
                ));
            }
            let mut records =
                Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(MAX_RECORDS));
            while let Some(record) = sequence.next_element()? {
                if records.len() == MAX_RECORDS {
                    return Err(serde::de::Error::custom(
                        "vector index contains too many records",
                    ));
                }
                records.push(record);
            }
            Ok(records)
        }
    }
    deserializer.deserialize_seq(RecordsVisitor)
}

fn deserialize_bounded_vector<'de, D>(deserializer: D) -> Result<Vec<f32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct VectorVisitor;
    impl<'de> serde::de::Visitor<'de> for VectorVisitor {
        type Value = Vec<f32>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a bounded embedding vector")
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            if sequence
                .size_hint()
                .is_some_and(|size| size > MAX_VECTOR_DIMENSIONS)
            {
                return Err(serde::de::Error::custom("embedding vector is too large"));
            }
            let mut vector =
                Vec::with_capacity(sequence.size_hint().unwrap_or(0).min(MAX_VECTOR_DIMENSIONS));
            while let Some(value) = sequence.next_element()? {
                if vector.len() == MAX_VECTOR_DIMENSIONS {
                    return Err(serde::de::Error::custom("embedding vector is too large"));
                }
                vector.push(value);
            }
            Ok(vector)
        }
    }
    deserializer.deserialize_seq(VectorVisitor)
}

#[cfg(test)]
fn encode_unchecked(index: &Index) -> Vec<u8> {
    let payload = postcard::to_allocvec(index).unwrap();
    let mut encoded = Vec::with_capacity(HEADER_BYTES + payload.len());
    encoded.extend_from_slice(MAGIC);
    encoded.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    encoded.extend_from_slice(&Sha256::digest(&payload));
    encoded.extend_from_slice(&payload);
    encoded
}

fn validate_vector(vector: &[f32], dimensions: u32) -> Result<f64, String> {
    if vector.len() != dimensions as usize || vector.iter().any(|v| !v.is_finite()) {
        return Err("invalid vector dimensions or non-finite values".into());
    }
    let norm = vector
        .iter()
        .map(|&v| f64::from(v).powi(2))
        .sum::<f64>()
        .sqrt();
    if norm == 0. {
        return Err("zero vector has undefined cosine similarity".into());
    }
    Ok(norm)
}

impl Index {
    /// Descending cosine similarity; ties use ascending (path, source_type, chunk_index).
    /// Accumulate in f64 to avoid overflow for finite f32 vectors.
    fn search(&self, query: &[f32], limit: usize) -> Result<Vec<(&Record, f32)>, String> {
        let norm = validate_vector(query, self.dimensions)?;
        let mut ranked = self
            .records
            .iter()
            .map(|record| {
                let record_norm = validate_vector(&record.vector, self.dimensions)?;
                let dot = query
                    .iter()
                    .zip(&record.vector)
                    .map(|(&a, &b)| f64::from(a) * f64::from(b))
                    .sum::<f64>();
                Ok((record, (dot / (norm * record_norm)).clamp(-1., 1.)))
            })
            .collect::<Result<Vec<_>, String>>()?;
        ranked.sort_by(|(a, sa), (b, sb)| {
            sb.total_cmp(sa).then_with(|| {
                (&a.path, &a.source_type, a.chunk_index).cmp(&(
                    &b.path,
                    &b.source_type,
                    b.chunk_index,
                ))
            })
        });
        ranked.truncate(limit);
        Ok(ranked.into_iter().map(|(r, s)| (r, s as f32)).collect())
    }
}

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, RwLock, Weak},
};

struct SlugState {
    index: RwLock<Option<Index>>,
    write: Mutex<()>,
    mutation: Arc<tokio::sync::Mutex<()>>,
}

impl SlugState {
    fn new() -> Self {
        Self {
            index: RwLock::new(None),
            write: Mutex::new(()),
            mutation: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
}

type Indexes = Mutex<BTreeMap<String, Arc<SlugState>>>;
type RegistryKey = (PathBuf, String, String, u32);
static STORES: OnceLock<Mutex<BTreeMap<RegistryKey, Weak<Indexes>>>> = OnceLock::new();

pub(super) struct Store {
    root: PathBuf,
    provider: String,
    model: String,
    dimensions: u32,
    indexes: Arc<Indexes>,
    #[cfg(test)]
    fail_before_rename: std::sync::atomic::AtomicBool,
}

impl Store {
    pub fn new(workspace: &Path, provider: &str, model: &str, dimensions: u32) -> Self {
        let workspace = workspace.canonicalize().unwrap_or_else(|_| {
            std::path::absolute(workspace).unwrap_or_else(|_| workspace.into())
        });
        let root = workspace.join("vectors");
        let key = (
            root.clone(),
            provider.to_owned(),
            model.to_owned(),
            dimensions,
        );
        let mut registry = STORES
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        registry.retain(|_, value| value.strong_count() > 0);
        let indexes = registry
            .get(&key)
            .and_then(Weak::upgrade)
            .unwrap_or_else(|| {
                let indexes = Arc::new(Mutex::new(BTreeMap::new()));
                registry.insert(key, Arc::downgrade(&indexes));
                indexes
            });
        Self {
            root,
            provider: provider.into(),
            model: model.into(),
            dimensions,
            indexes,
            #[cfg(test)]
            fail_before_rename: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn slug_state(&self, slug: &str) -> Arc<SlugState> {
        let mut indexes = self
            .indexes
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        indexes
            .entry(slug.to_owned())
            .or_insert_with(|| Arc::new(SlugState::new()))
            .clone()
    }

    pub fn mutation_lock(&self, slug: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.slug_state(slug).mutation.clone()
    }

    // Hash the complete slug; no slug bytes ever become path components.
    fn index_path(&self, slug: &str) -> PathBuf {
        self.root.join(format!(
            "{}.bin",
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, slug.as_bytes())
        ))
    }

    fn load(&self, slug: &str) -> Result<Index, String> {
        use std::io::Read;

        let path = self.index_path(slug);
        let file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Index::empty(&self.provider, &self.model, self.dimensions));
            }
            Err(error) => return Err(error.to_string()),
        };
        let length = file.metadata().map_err(|error| error.to_string())?.len();
        let decoded = if length > MAX_INDEX_FILE_BYTES as u64 {
            Err("vector index file is too large".into())
        } else {
            let mut bytes = Vec::with_capacity(usize::try_from(length).unwrap_or(0));
            file.take((MAX_INDEX_FILE_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|error| error.to_string())?;
            if bytes.len() > MAX_INDEX_FILE_BYTES {
                Err("vector index file grew beyond the size limit".into())
            } else {
                Index::decode(&bytes, &self.provider, &self.model, self.dimensions)
            }
        };
        match decoded {
            Ok(index) => Ok(index),
            Err(error) => {
                log::warn!(
                    "[vector] rebuilding invalid cache {}: {error}",
                    path.display()
                );
                Ok(Index::empty(&self.provider, &self.model, self.dimensions))
            }
        }
    }

    fn persist(&self, slug: &str, index: &Index) -> Result<(), String> {
        std::fs::create_dir_all(&self.root).map_err(|error| error.to_string())?;
        use std::io::Write;
        let bytes = index.encode()?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(&self.root).map_err(|error| error.to_string())?;
        temporary
            .write_all(&bytes)
            .map_err(|error| error.to_string())?;
        temporary.flush().map_err(|error| error.to_string())?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|error| error.to_string())?;
        let temporary = temporary.into_temp_path();
        #[cfg(test)]
        if self
            .fail_before_rename
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            return Err("injected failure before rename".into());
        }
        // tempfile documents persist as an atomic replacement on supported platforms.
        // TempPath also removes the temporary file if replacement fails.
        temporary
            .persist(self.index_path(slug))
            .map_err(|error| error.error.to_string())?;
        // Replacement has committed. Directory-sync or legacy cleanup failure cannot roll it back.
        #[cfg(unix)]
        if let Err(error) = std::fs::File::open(&self.root).and_then(|dir| dir.sync_all()) {
            log::warn!("[vector] index saved but directory sync failed: {error}");
        }
        let workspace = self.root.parent().expect("vectors directory has a parent");
        let legacy = workspace.join("lancedb");
        if let Err(error) = std::fs::remove_dir_all(&legacy) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!("[vector] saved replacement index; old cache cleanup failed: {error}");
            }
        }
        for marker in [".vectors_backfilled_lancedb", ".vectors_backfilled"] {
            if let Err(error) = std::fs::remove_file(workspace.join(marker)) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!(
                        "[vector] saved replacement index; old marker cleanup failed: {error}"
                    );
                }
            }
        }
        Ok(())
    }

    fn with_index<T>(
        &self,
        slug: &str,
        operation: impl FnOnce(&Index) -> Result<T, String>,
    ) -> Result<T, String> {
        let state = self.slug_state(slug);
        {
            let current = state.index.read().map_err(|error| error.to_string())?;
            if let Some(index) = current.as_ref() {
                return operation(index);
            }
        }
        let _write = state.write.lock().map_err(|error| error.to_string())?;
        let mut current = state.index.write().map_err(|error| error.to_string())?;
        if current.is_none() {
            let loaded = self.load(slug)?;
            self.persist(slug, &loaded)?;
            *current = Some(loaded);
        }
        operation(current.as_ref().expect("index initialized"))
    }

    pub fn ensure(&self, slug: &str) -> Result<(), String> {
        self.with_index(slug, |_| Ok(()))
    }

    fn mutate(
        &self,
        slug: &str,
        change: impl FnOnce(&mut Index) -> Result<(), String>,
    ) -> Result<(), String> {
        let state = self.slug_state(slug);
        let _write = state.write.lock().map_err(|error| error.to_string())?;
        let mut next = state
            .index
            .read()
            .map_err(|error| error.to_string())?
            .clone()
            .map_or_else(|| self.load(slug), Ok)?;
        change(&mut next)?;
        next.validate(&self.provider, &self.model, self.dimensions)?;
        self.persist(slug, &next)?;
        *state.index.write().map_err(|error| error.to_string())? = Some(next);
        Ok(())
    }

    pub fn replace(&self, slug: &str, path: &str, records: Vec<Record>) -> Result<(), String> {
        for record in &records {
            validate_vector(&record.vector, self.dimensions)?;
        }
        self.mutate(slug, |index| {
            index.records.retain(|record| record.path != path);
            index.records.extend(records);
            sort_records(&mut index.records);
            Ok(())
        })
    }

    pub fn delete(&self, slug: &str, path: &str) -> Result<(), String> {
        self.replace(slug, path, Vec::new())
    }

    pub fn reset(&self, slug: &str) -> Result<(), String> {
        self.mutate(slug, |index| {
            *index = Index::empty(&self.provider, &self.model, self.dimensions);
            Ok(())
        })
    }

    pub fn needs_backfill(&self, slug: &str) -> Result<bool, String> {
        self.with_index(slug, |index| Ok(!index.backfilled))
    }

    #[cfg(test)]
    pub fn mark_backfilled(&self, slug: &str) -> Result<(), String> {
        self.mutate(slug, |index| {
            index.backfilled = true;
            Ok(())
        })
    }

    pub fn commit_backfill(&self, slug: &str, mut records: Vec<Record>) -> Result<(), String> {
        for record in &records {
            validate_vector(&record.vector, self.dimensions)?;
        }
        sort_records(&mut records);
        self.mutate(slug, |index| {
            index.records = records;
            index.backfilled = true;
            Ok(())
        })
    }

    pub fn search(
        &self,
        slug: &str,
        query: &[f32],
        limit: usize,
    ) -> Result<Vec<(Record, f32)>, String> {
        validate_vector(query, self.dimensions)?;
        self.with_index(slug, |index| {
            Ok(index
                .search(query, limit)?
                .into_iter()
                .map(|(record, score)| (record.clone(), score))
                .collect())
        })
    }

    pub fn list(&self, slug: &str, limit: usize) -> Result<Vec<Record>, String> {
        self.with_index(slug, |index| {
            Ok(index.records.iter().take(limit).cloned().collect())
        })
    }
}

fn sort_records(records: &mut [Record]) {
    records.sort_by(|a, b| {
        (&a.path, &a.source_type, a.chunk_index).cmp(&(&b.path, &b.source_type, b.chunk_index))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_roundtrip() {
        let index = Index::empty("google", "gemini-embedding-2-preview", 3);
        let bytes = index.encode().unwrap();
        let decoded = Index::decode(&bytes, "google", "gemini-embedding-2-preview", 3).unwrap();
        assert_eq!(decoded.version, 1);
        assert_eq!(decoded.provider, "google");
        assert_eq!(decoded.model, "gemini-embedding-2-preview");
        assert_eq!(decoded.dimensions, 3);
        assert!(decoded.records.is_empty());
    }

    #[test]
    fn envelope_rejects_checksum_corruption_truncation_and_trailing_bytes() {
        let index = Index::empty("google", "gemini-embedding-2-preview", 3);
        let good = index.encode().unwrap();

        let mut corrupt = good.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(Index::decode(&corrupt, "google", "gemini-embedding-2-preview", 3).is_err());
        assert!(
            Index::decode(
                &good[..good.len() - 1],
                "google",
                "gemini-embedding-2-preview",
                3
            )
            .is_err()
        );

        let mut trailing = good;
        trailing.push(0);
        assert!(Index::decode(&trailing, "google", "gemini-embedding-2-preview", 3).is_err());
    }

    #[test]
    fn envelope_rejects_oversized_metadata_records_and_strings() {
        let oversized_dimensions =
            Index::empty("provider", "model", (MAX_VECTOR_DIMENSIONS + 1) as u32);
        assert!(oversized_dimensions.encode().is_err());

        let mut index = Index::empty(&"p".repeat(MAX_METADATA_BYTES + 1), "model", 2);
        assert!(index.encode().is_err());
        assert!(
            Index::decode(&encode_unchecked(&index), &index.provider, &index.model, 2).is_err()
        );

        index = Index::empty("provider", "model", 2);
        index.records = (0..=MAX_RECORDS)
            .map(|i| Record {
                path: i.to_string(),
                source_type: "text_memory".into(),
                chunk_index: 0,
                content_preview: "x".into(),
                upload_id: None,
                vector: vec![1., 0.],
            })
            .collect();
        assert!(index.encode().is_err());
        assert!(Index::decode(&encode_unchecked(&index), "provider", "model", 2).is_err());

        let mut record =
            super::ranking_tests::record(&"x".repeat(MAX_STRING_BYTES + 1), vec![1., 0.]);
        index.records = vec![record.clone()];
        assert!(index.encode().is_err());
        assert!(Index::decode(&encode_unchecked(&index), "provider", "model", 2).is_err());
        record.path = "ok".into();
        record.content_preview = "x".repeat(MAX_STRING_BYTES + 1);
        index.records = vec![record];
        assert!(index.encode().is_err());
        assert!(Index::decode(&encode_unchecked(&index), "provider", "model", 2).is_err());
    }
}

#[cfg(test)]
mod ranking_tests {
    use super::*;
    pub(super) fn record(path: &str, vector: Vec<f32>) -> Record {
        Record {
            path: path.into(),
            source_type: "text_memory".into(),
            chunk_index: 0,
            content_preview: path.into(),
            upload_id: None,
            vector,
        }
    }
    #[test]
    fn cosine_ranking_and_stable_ties() {
        let mut index = Index::empty("test-provider", "test-model", 2);
        index.records = vec![
            record("z", vec![2., 0.]),
            record("negative", vec![-1., 0.]),
            record("a", vec![1., 0.]),
            record("orthogonal", vec![0., 1.]),
        ];
        let ranked = index.search(&[3., 0.], 3).unwrap();
        assert_eq!(
            ranked
                .iter()
                .map(|(r, _)| r.path.as_str())
                .collect::<Vec<_>>(),
            ["a", "z", "orthogonal"]
        );
        assert_eq!(
            ranked.iter().map(|(_, s)| *s).collect::<Vec<_>>(),
            [1., 1., 0.]
        );
        assert_eq!(index.search(&[3., 0.], 20).unwrap()[3].1, -1.);
        assert!(index.search(&[3., 0.], 0).unwrap().is_empty());
        assert!(index.search(&[1.], 1).is_err());
        assert!(index.search(&[f32::NAN, 0.], 1).is_err());
        assert!(index.search(&[f32::INFINITY, 0.], 1).is_err());
        assert!(index.search(&[0., 0.], 1).is_err());
        let mut later_chunk = record("a", vec![1., 0.]);
        later_chunk.chunk_index = 1;
        index.records.insert(0, later_chunk);
        assert_eq!(
            index
                .search(&[3., 0.], 2)
                .unwrap()
                .iter()
                .map(|(r, _)| r.chunk_index)
                .collect::<Vec<_>>(),
            [0, 1]
        );
        assert_eq!(index.search(&[f32::MAX, 0.], 1).unwrap()[0].1, 1.);
    }
}

#[cfg(test)]
mod store_tests {
    use super::ranking_tests::record;
    use super::*;

    struct Workspace(std::path::PathBuf);
    impl Workspace {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("vector-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn store(&self) -> Store {
            Store::new(&self.0, "test-provider", "test-model", 2)
        }
    }
    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn upsert_replaces_all_chunks_and_preserves_other_paths() {
        let ws = Workspace::new();
        let store = ws.store();
        store
            .replace("slug", "a", vec![record("a", vec![1., 0.])])
            .unwrap();
        store
            .replace("slug", "b", vec![record("b", vec![0., 1.])])
            .unwrap();
        let mut chunk = record("a", vec![0., 2.]);
        chunk.chunk_index = 1;
        store
            .replace("slug", "a", vec![record("a", vec![0., 1.]), chunk])
            .unwrap();
        assert_eq!(store.list("slug", 20).unwrap().len(), 3);
        store
            .replace("slug", "a", vec![record("a", vec![1., 1.])])
            .unwrap();
        assert_eq!(store.list("slug", 20).unwrap().len(), 2);
        assert!(
            store
                .replace("slug", "a", vec![record("a", vec![f32::NAN, 0.])])
                .is_err()
        );
        assert_eq!(store.list("slug", 20).unwrap().len(), 2);
    }
    #[test]
    fn delete_and_reset_are_isolated_and_idempotent() {
        let ws = Workspace::new();
        let store = ws.store();
        store
            .replace("one", "a", vec![record("a", vec![1., 0.])])
            .unwrap();
        store
            .replace("two", "a", vec![record("a", vec![1., 0.])])
            .unwrap();
        store.delete("one", "missing").unwrap();
        store.delete("one", "a").unwrap();
        assert!(store.list("one", 20).unwrap().is_empty());
        assert_eq!(store.list("two", 20).unwrap().len(), 1);
        store.reset("two").unwrap();
        store.reset("two").unwrap();
        assert!(store.list("two", 20).unwrap().is_empty());
    }

    #[test]
    fn persisted_records_reload_and_missing_index_is_created() {
        let ws = Workspace::new();
        let store = ws.store();
        store.ensure("slug").unwrap();
        assert!(store.index_path("slug").is_file());
        store
            .replace("slug", "a", vec![record("a", vec![1., 0.])])
            .unwrap();
        drop(store);
        let store = ws.store();
        assert_eq!(store.list("slug", 10).unwrap()[0].vector, [1., 0.]);
        store.delete("slug", "a").unwrap();
        drop(store);
        let store = ws.store();
        assert!(store.list("slug", 10).unwrap().is_empty());
        store
            .replace("slug", "b", vec![record("b", vec![0., 1.])])
            .unwrap();
        store.reset("slug").unwrap();
        drop(store);
        assert!(ws.store().list("slug", 10).unwrap().is_empty());
    }

    #[test]
    fn persistence_atomically_replaces_an_existing_index() {
        let ws = Workspace::new();
        let store = ws.store();
        store
            .replace("slug", "old", vec![record("old", vec![1., 0.])])
            .unwrap();
        store
            .replace("slug", "new", vec![record("new", vec![0., 1.])])
            .unwrap();
        drop(store);

        let records = ws.store().list("slug", 10).unwrap();
        assert_eq!(
            records.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            ["new", "old"]
        );
    }

    #[test]
    fn oversized_file_is_rebuilt_without_reading_its_payload() {
        let ws = Workspace::new();
        let store = ws.store();
        std::fs::create_dir_all(&store.root).unwrap();
        let file = std::fs::File::create(store.index_path("slug")).unwrap();
        file.set_len((MAX_INDEX_FILE_BYTES + 1) as u64).unwrap();
        store.ensure("slug").unwrap();
        assert!(store.list("slug", 1).unwrap().is_empty());
        assert!(
            std::fs::metadata(store.index_path("slug")).unwrap().len()
                <= MAX_INDEX_FILE_BYTES as u64
        );
    }

    #[test]
    fn corrupt_truncated_and_incompatible_indexes_are_replaced() {
        let good = Index::empty("test-provider", "test-model", 2);
        let mut cases = vec![vec![255; 10], good.encode().unwrap()[..2].to_vec()];
        let mut wrong = good.clone();
        wrong.version += 1;
        cases.push(encode_unchecked(&wrong));
        let mut wrong = good.clone();
        wrong.provider = "other-provider".into();
        cases.push(encode_unchecked(&wrong));
        let mut wrong = good.clone();
        wrong.model = "other".into();
        cases.push(encode_unchecked(&wrong));
        let mut wrong = good.clone();
        wrong.dimensions += 1;
        cases.push(encode_unchecked(&wrong));
        for vector in [vec![1.], vec![f32::NAN, 0.], vec![0., 0.]] {
            let mut wrong = good.clone();
            wrong.records.push(record("a", vector));
            cases.push(encode_unchecked(&wrong));
        }
        let mut trailing = good.encode().unwrap();
        trailing.push(0);
        cases.push(trailing);
        for bytes in cases {
            let ws = Workspace::new();
            let store = ws.store();
            std::fs::create_dir_all(&store.root).unwrap();
            std::fs::write(store.index_path("slug"), bytes).unwrap();
            store.ensure("slug").unwrap();
            assert!(store.list("slug", 10).unwrap().is_empty());
            Index::decode(
                &std::fs::read(store.index_path("slug")).unwrap(),
                "test-provider",
                "test-model",
                2,
            )
            .unwrap();
        }
    }

    #[test]
    fn slug_cannot_escape_index_directory() {
        let ws = Workspace::new();
        let store = ws.store();
        let slugs = [
            "../../escape",
            "/absolute",
            "a/b",
            "a\\b",
            "",
            ".",
            "..",
            "日本語",
            "normal",
        ];
        let mut paths = std::collections::BTreeSet::new();
        for slug in slugs {
            let path = store.index_path(slug);
            assert_eq!(path.parent(), Some(store.root.as_path()));
            assert!(paths.insert(path.clone()));
            store.ensure(slug).unwrap();
            assert!(path.is_file());
        }
        assert_eq!(std::fs::read_dir(&ws.0).unwrap().count(), 1);
    }

    #[test]
    fn failed_atomic_write_preserves_disk_and_memory() {
        let ws = Workspace::new();
        let store = ws.store();
        store
            .replace("slug", "a", vec![record("a", vec![1., 0.])])
            .unwrap();
        let before = std::fs::read(store.index_path("slug")).unwrap();
        store
            .fail_before_rename
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(
            store
                .replace("slug", "b", vec![record("b", vec![0., 1.])])
                .is_err()
        );
        assert_eq!(std::fs::read(store.index_path("slug")).unwrap(), before);
        assert_eq!(store.list("slug", 10).unwrap().len(), 1);
        assert_eq!(ws.store().list("slug", 10).unwrap().len(), 1);
        assert_eq!(std::fs::read_dir(&store.root).unwrap().count(), 1);
    }

    #[test]
    fn concurrent_handles_preserve_every_successful_write() {
        let ws = Workspace::new();
        let store = ws.store();
        store.ensure("slug").unwrap();
        let handles = (0..16).map(|_| ws.store()).collect::<Vec<_>>();
        let barrier = std::sync::Barrier::new(handles.len());
        std::thread::scope(|scope| {
            for (i, handle) in handles.iter().enumerate() {
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    let path = format!("path-{i}");
                    handle
                        .replace("slug", &path, vec![record(&path, vec![1., 0.])])
                        .unwrap();
                });
            }
        });
        assert_eq!(store.list("slug", 100).unwrap().len(), 16);
        drop(handles);
        drop(store);
        assert_eq!(ws.store().list("slug", 100).unwrap().len(), 16);
    }

    #[test]
    fn searches_use_last_committed_index_while_same_slug_write_is_waiting() {
        let ws = Workspace::new();
        let store = ws.store();
        store
            .replace("one", "a", vec![record("a", vec![1., 0.])])
            .unwrap();
        let one = store.slug_state("one");
        let _held = one.write.lock().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                tx.send(store.search("one", &[1., 0.], 1).unwrap()[0].0.path.clone())
                    .unwrap();
            });
            assert_eq!(
                rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap(),
                "a"
            );
        });
    }

    #[test]
    fn different_companions_do_not_share_the_persistence_lock() {
        let ws = Workspace::new();
        let store = ws.store();
        let one = store.slug_state("one");
        let _held = one.write.lock().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                store.ensure("two").unwrap();
                tx.send(()).unwrap();
            });
            rx.recv_timeout(std::time::Duration::from_secs(2)).unwrap();
        });
    }

    #[test]
    fn legacy_cleanup_only_after_successful_persistence() {
        let ws = Workspace::new();
        let legacy = ws.0.join("lancedb");
        let markers = [
            ws.0.join(".vectors_backfilled_lancedb"),
            ws.0.join(".vectors_backfilled"),
        ];
        std::fs::create_dir(&legacy).unwrap();
        std::fs::write(legacy.join("old-data"), b"untouched until commit").unwrap();
        for marker in &markers {
            std::fs::write(marker, b"legacy marker").unwrap();
        }
        let store = ws.store();
        assert!(legacy.exists());
        store
            .fail_before_rename
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(store.ensure("slug").is_err());
        assert!(legacy.join("old-data").is_file());
        assert!(markers.iter().all(|marker| marker.is_file()));
        assert!(!store.index_path("slug").exists());
        store
            .fail_before_rename
            .store(false, std::sync::atomic::Ordering::SeqCst);
        store.ensure("slug").unwrap();
        assert!(!legacy.exists());
        assert!(markers.iter().all(|marker| !marker.exists()));
        assert!(store.index_path("slug").is_file());
        // A non-directory simulates a cleanup error; saving the new index still succeeds.
        std::fs::write(&legacy, b"not a directory").unwrap();
        store
            .replace("slug", "a", vec![record("a", vec![1., 0.])])
            .unwrap();
        assert!(legacy.exists());
        drop(store);
        assert_eq!(ws.store().list("slug", 10).unwrap().len(), 1);
    }

    #[test]
    fn backfill_completion_is_per_index_and_invalidated_on_rebuild() {
        let ws = Workspace::new();
        let store = ws.store();
        assert!(store.needs_backfill("one").unwrap());
        store.mark_backfilled("one").unwrap();
        assert!(!store.needs_backfill("one").unwrap());
        assert!(store.needs_backfill("two").unwrap());
        drop(store);
        let store = ws.store();
        assert!(!store.needs_backfill("one").unwrap());
        store.reset("one").unwrap();
        assert!(store.needs_backfill("one").unwrap());
        store.mark_backfilled("one").unwrap();
        let path = store.index_path("one");
        drop(store);
        std::fs::write(&path, b"broken").unwrap();
        let store = ws.store();
        assert!(store.needs_backfill("one").unwrap());
        store.mark_backfilled("one").unwrap();
        drop(store);
        std::fs::remove_file(path).unwrap();
        assert!(ws.store().needs_backfill("one").unwrap());
    }

    #[test]
    fn candidate_commit_is_all_or_nothing_and_retry_drops_stale_records() {
        let ws = Workspace::new();
        let store = ws.store();
        store
            .replace("slug", "old", vec![record("old", vec![1., 0.])])
            .unwrap();

        assert!(
            store
                .commit_backfill("slug", vec![record("bad", vec![f32::NAN, 0.])])
                .is_err()
        );
        assert_eq!(store.list("slug", 10).unwrap()[0].path, "old");
        assert!(store.needs_backfill("slug").unwrap());

        store
            .commit_backfill("slug", vec![record("current", vec![0., 1.])])
            .unwrap();
        let records = store.list("slug", 10).unwrap();
        assert_eq!(
            records.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            ["current"]
        );
        assert!(!store.needs_backfill("slug").unwrap());
    }

    #[test]
    fn search_reloads_media_metadata() {
        let ws = Workspace::new();
        let store = ws.store();
        let mut media = record("upload", vec![1., 0.]);
        media.upload_id = Some("upload".into());
        media.source_type = "media_image".into();
        store.replace("slug", "upload", vec![media]).unwrap();
        drop(store);
        let store = ws.store();
        let results = store.search("slug", &[2., 0.], 1).unwrap();
        assert_eq!(results[0].0.upload_id.as_deref(), Some("upload"));
        assert_eq!(results[0].0.source_type, "media_image");
        assert_eq!(results[0].1, 1.);
        assert!(store.search("empty", &[f32::INFINITY, 0.], 0).is_err());
    }
}
