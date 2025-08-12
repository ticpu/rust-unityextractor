use log::{debug, trace, warn};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::file_operations;
use crate::file_operations::{stream_asset_to_pathname, stream_orphaned_asset};
use crate::thread_pool::{ThreadPool, WriteTask};

const KB: u64 = 1024;

pub struct AssetWriteError {
    pub error: io::Error,
    pub path: String,
}

impl fmt::Display for AssetWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.path, self.error)
    }
}

pub type FolderSet = HashSet<OsString>;
pub type OrphanedAssets = Vec<PathBuf>;
pub type PathNameMap = HashMap<PathBuf, String>;

pub struct ExtractionContext {
    folders: FolderSet,
    pathnames: PathNameMap,
    orphaned_assets: OrphanedAssets,
    stream_threshold: u64,
}

impl ExtractionContext {
    pub fn new(stream_threshold: u64) -> Self {
        Self {
            folders: HashSet::new(),
            pathnames: HashMap::new(),
            orphaned_assets: Vec::new(),
            stream_threshold,
        }
    }

    pub fn has_orphaned_work(&self) -> bool {
        !self.orphaned_assets.is_empty()
    }

    // Folder management
    pub fn insert_folder(&mut self, path: OsString) {
        self.folders.insert(path);
    }

    pub fn is_folder(&self, meta_path: &Path) -> bool {
        self.folders.contains(&meta_path.as_os_str().to_os_string())
    }

    // Pathname management
    pub fn insert_pathname(&mut self, asset_path: PathBuf, pathname: String) {
        self.pathnames.insert(asset_path, pathname);
    }

    pub fn get_pathname(&self, asset_path: &PathBuf) -> Option<&String> {
        self.pathnames.get(asset_path)
    }

    // Orphaned asset management
    pub fn add_orphaned_asset(&mut self, asset_path: PathBuf) {
        self.orphaned_assets.push(asset_path);
    }

    pub fn take_orphaned_data(self) -> (OrphanedAssets, PathNameMap) {
        (self.orphaned_assets, self.pathnames)
    }

    // Debug info
    pub fn orphaned_count(&self) -> usize {
        self.orphaned_assets.len()
    }
}

pub struct ExtractionResult {
    pub context: ExtractionContext,
}

// Pass 1: Read pathnames/metadata first, then process assets
pub fn process_archive_entries<R: Read>(
    archive: &mut tar::Archive<R>,
    thread_pool: &ThreadPool,
    stream_threshold: u64,
) -> Result<ExtractionResult, io::Error> {
    let mut context = ExtractionContext::new(stream_threshold);

    for entry_result in archive.entries()? {
        let entry = match entry_result {
            Ok(file) => file,
            Err(e) => {
                warn!("error reading entry from archive: {e}");
                continue;
            }
        };

        let path = match entry.path() {
            Ok(p) => p.to_path_buf(),
            Err(e) => {
                warn!("errors reading path from entry: {e}");
                continue;
            }
        };

        if path.ends_with("pathname") {
            read_pathname(&mut context, entry, path)?;
        } else if path.ends_with("asset.meta") {
            read_metadata(&mut context, entry, path)?;
        } else if path.ends_with("asset") {
            read_asset(&mut context, thread_pool, entry, path)?;
        } else if path.ends_with("/") {
            trace!("skipping folder {}", path.display());
        } else {
            trace!("skipping entry with name {}", path.display())
        }
    }

    Ok(ExtractionResult { context })
}

fn read_pathname<R: Read>(
    context: &mut ExtractionContext,
    mut entry: tar::Entry<'_, R>,
    path: PathBuf,
) -> Result<(), io::Error> {
    let mut path_name = String::new();
    entry.read_to_string(&mut path_name)?;

    let asset_path = path.parent().unwrap().join("asset");

    debug!("storing pathname: {}", path_name.escape_default());
    context.insert_pathname(asset_path, path_name);
    Ok(())
}

fn read_metadata<R: Read>(
    context: &mut ExtractionContext,
    mut entry: tar::Entry<'_, R>,
    path: PathBuf,
) -> Result<(), io::Error> {
    debug!("reading metadata for {path:?}");
    let mut metadata = String::new();
    entry.read_to_string(&mut metadata)?;
    if metadata.contains("folderAsset: yes\n") {
        context.insert_folder(path.into_os_string());
    }
    Ok(())
}

fn read_asset<R: Read>(
    context: &mut ExtractionContext,
    thread_pool: &ThreadPool,
    mut entry: tar::Entry<'_, R>,
    path: PathBuf,
) -> Result<(), io::Error> {
    let asset_size = entry.header().size()?;
    debug!("processing asset {path:?}, size: {asset_size} bytes");

    let meta_path = path.parent().unwrap().join("asset.meta");

    // Check if we have a pathname for this asset
    if let Some(pathname) = context.get_pathname(&path) {
        debug!(
            "extracting asset to proper location: {}",
            pathname.escape_default()
        );

        let entry_hash = path.to_string_lossy().to_string();
        let pathname_clone = pathname.clone();

        // Route based on size: large files stream synchronously, small files queue
        if asset_size >= context.stream_threshold {
            debug!("Streaming large asset ({asset_size} bytes) synchronously");
            let context_name = format!("extracting {}", extract_asset_hash(&entry_hash));
            stream_asset_to_pathname(
                entry,
                &pathname_clone,
                &context_name,
                context.stream_threshold,
            )
            .map_err(|e| io::Error::other(e.to_string()))?;
        } else {
            // Try to queue small file
            let mut asset_data = Vec::new();
            entry.read_to_end(&mut asset_data)?;

            let asset_data_for_task = asset_data.clone();
            let task: WriteTask = Box::new(move || {
                Box::pin(async move {
                    write_asset_to_pathname(asset_data_for_task, entry_hash, pathname_clone).await
                })
            });

            if !thread_pool.try_queue_task(asset_size, task) {
                // Fallback to synchronous streaming if queue is full
                debug!("Queue full, streaming small asset ({asset_size} bytes) synchronously");
                let context_name =
                    format!("extracting {}", extract_asset_hash(&path.to_string_lossy()));
                let pathname_for_fallback = pathname.clone();
                stream_asset_to_pathname(
                    io::Cursor::new(asset_data),
                    &pathname_for_fallback,
                    &context_name,
                    context.stream_threshold,
                )
                .map_err(|e| io::Error::other(e.to_string()))?;
            }
        }
    } else if context.is_folder(&meta_path) {
        debug!("skipping folder asset content for {path:?}");
    } else {
        // Extract asset to root directory as orphaned file
        let orphan_name = extract_guid_from_path(&path);
        debug!("extracting orphaned asset to root: {orphan_name}");
        let orphan_path = PathBuf::from(&orphan_name);
        context.add_orphaned_asset(orphan_path.clone());

        // Route orphaned assets similar to regular assets
        if asset_size >= context.stream_threshold {
            debug!("Streaming large orphaned asset ({asset_size} bytes) synchronously");
            stream_orphaned_asset(
                entry,
                &orphan_name,
                "writing orphaned asset to root",
                context.stream_threshold,
            )
            .map_err(|e| io::Error::other(e.to_string()))?;
        } else {
            // Try to queue small orphaned file
            let mut asset_data = Vec::new();
            entry.read_to_end(&mut asset_data)?;

            let asset_data_for_task = asset_data.clone();
            let orphan_name_for_task = orphan_name.clone();
            let task: WriteTask = Box::new(move || {
                Box::pin(async move {
                    write_orphaned_asset(asset_data_for_task, orphan_name_for_task).await
                })
            });

            if !thread_pool.try_queue_task(asset_size, task) {
                // Fallback to synchronous streaming if queue is full
                debug!(
                    "Queue full, streaming small orphaned asset ({asset_size} bytes) synchronously"
                );
                stream_orphaned_asset(
                    io::Cursor::new(asset_data),
                    &orphan_name,
                    "writing orphaned asset to root",
                    context.stream_threshold,
                )
                .map_err(|e| io::Error::other(e.to_string()))?;
            }
        }
    }
    Ok(())
}

// Pass 2: Process orphaned assets - now handled during archive processing
pub async fn process_orphaned_assets(
    context: ExtractionContext,
    thread_pool: &ThreadPool,
) -> Result<(), io::Error> {
    let (orphaned_assets, pathnames) = context.take_orphaned_data();

    if orphaned_assets.is_empty() {
        debug!("No orphaned assets to process");
        return Ok(());
    }

    debug!(
        "Pass 2: processing {} orphaned assets",
        orphaned_assets.len()
    );

    for orphan_path in orphaned_assets {
        let guid = orphan_path.to_string_lossy().to_string();
        let asset_path = PathBuf::from(&guid).join("asset");

        if let Some(pathname) = pathnames.get(&asset_path) {
            debug!(
                "moving orphaned asset {} to {}",
                guid,
                pathname.escape_default()
            );
            let pathname_clone = pathname.clone();
            let guid_clone = guid.clone();

            let task: WriteTask = Box::new(move || {
                Box::pin(async move { move_orphaned_asset(guid_clone, pathname_clone).await })
            });

            // For orphaned assets, we don't know the size, so assume small and queue
            // If queue is full, handle synchronously
            if !thread_pool.try_queue_task(KB, task) {
                // Assume 1KB for orphaned moves
                debug!("Queue full, moving orphaned asset {guid} synchronously");
                move_orphaned_asset(guid, pathname.clone())
                    .await
                    .map_err(|e| io::Error::other(e.to_string()))?;
            }
        } else {
            warn!("No pathname found for orphaned asset {guid}, deleting");
            let guid_clone = guid.clone();

            let task: WriteTask =
                Box::new(move || Box::pin(async move { delete_orphaned_asset(guid_clone).await }));

            if !thread_pool.try_queue_task(1, task) {
                // Minimal size for deletion
                debug!("Queue full, deleting orphaned asset {guid} synchronously");
                delete_orphaned_asset(guid)
                    .await
                    .map_err(|e| io::Error::other(e.to_string()))?;
            }
        }
    }

    Ok(())
}

// Helper functions for creating folder structures (when pathname exists but no asset)
pub async fn create_folder_structures(
    context: &ExtractionContext,
    thread_pool: &ThreadPool,
) -> Result<(), io::Error> {
    for (asset_path, pathname) in &context.pathnames {
        let meta_path = asset_path.parent().unwrap().join("asset.meta");
        if context.is_folder(&meta_path) {
            let pathname_clone = pathname.clone();
            let task: WriteTask = Box::new(move || {
                Box::pin(async move { create_folder_structure(pathname_clone).await })
            });

            if !thread_pool.try_queue_task(1, task) {
                // Minimal size for folder creation
                debug!("Queue full, creating folder structure synchronously");
                create_folder_structure(pathname.clone())
                    .await
                    .map_err(|e| io::Error::other(e.to_string()))?;
            }
        }
    }

    Ok(())
}

fn extract_guid_from_path(path: &Path) -> String {
    path.parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string()
}

fn extract_asset_hash(entry_hash: &str) -> &str {
    match entry_hash.find('/') {
        Some(idx) => &entry_hash[..idx],
        None => entry_hash,
    }
}

async fn create_folder_structure(path_name: String) -> Result<(), AssetWriteError> {
    file_operations::create_directory_structure(&path_name, "creating folder structure").await
}

async fn write_asset_to_pathname(
    asset_data: Vec<u8>,
    entry_hash: String,
    path_name: String,
) -> Result<(), AssetWriteError> {
    let asset_hash: &str;
    match entry_hash.find('/') {
        Some(idx) => {
            (asset_hash, _) = entry_hash.split_at(idx);
        }
        None => {
            asset_hash = &entry_hash;
        }
    }

    let context_name = format!("extracting {asset_hash}");
    file_operations::create_file_with_content(asset_data, &path_name, &context_name).await
}

async fn write_orphaned_asset(
    asset_data: Vec<u8>,
    orphan_name: String,
) -> Result<(), AssetWriteError> {
    let context_name = "writing orphaned asset to root".to_string();
    file_operations::create_file_with_content(asset_data, &orphan_name, &context_name).await
}

async fn move_orphaned_asset(
    orphan_name: String,
    target_pathname: String,
) -> Result<(), AssetWriteError> {
    let context_name = format!("moving orphaned asset {orphan_name}");
    file_operations::move_file_to_target(&orphan_name, &target_pathname, &context_name).await
}

async fn delete_orphaned_asset(orphan_name: String) -> Result<(), AssetWriteError> {
    let context_name = "deleting orphaned asset without pathname".to_string();
    file_operations::delete_file(&orphan_name, &context_name).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::bufread::GzDecoder;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Cursor;
    use tar::Builder;

    const MB: u64 = 1024 * KB;

    /// Test utility for building Unity package archives with various asset types
    pub struct TestUnityPackageBuilder {
        entries: Vec<TestEntry>,
    }

    #[derive(Clone)]
    enum TestEntry {
        Asset {
            guid: String,
            data: Vec<u8>,
        },
        FolderAsset {
            guid: String,
        },
        RegularAsset {
            guid: String,
            data: Vec<u8>,
            meta_content: Option<String>,
        },
        Pathname {
            guid: String,
            path: String,
        },
    }

    impl TestUnityPackageBuilder {
        pub fn new() -> Self {
            Self {
                entries: Vec::new(),
            }
        }

        /// Add a folder asset (only .meta file with folderAsset: yes, no asset file)
        pub fn add_folder_asset(mut self, guid: &str, pathname: &str) -> Self {
            self.entries.push(TestEntry::FolderAsset {
                guid: guid.to_string(),
            });
            self.entries.push(TestEntry::Pathname {
                guid: guid.to_string(),
                path: pathname.to_string(),
            });
            self
        }

        /// Add a regular asset with data
        pub fn add_asset(self, guid: &str, pathname: &str, asset_data: &[u8]) -> Self {
            self.add_asset_with_meta(guid, pathname, asset_data, None)
        }

        /// Add a regular asset with custom metadata
        pub fn add_asset_with_meta(
            mut self,
            guid: &str,
            pathname: &str,
            asset_data: &[u8],
            meta_content: Option<&str>,
        ) -> Self {
            self.entries.push(TestEntry::RegularAsset {
                guid: guid.to_string(),
                data: asset_data.to_vec(),
                meta_content: meta_content.map(|s| s.to_string()),
            });
            self.entries.push(TestEntry::Pathname {
                guid: guid.to_string(),
                path: pathname.to_string(),
            });
            self
        }

        /// Add only an asset file (no pathname) - useful for testing orphaned assets
        pub fn add_orphaned_asset(mut self, guid: &str, asset_data: &[u8]) -> Self {
            self.entries.push(TestEntry::Asset {
                guid: guid.to_string(),
                data: asset_data.to_vec(),
            });
            self
        }

        /// Add only a pathname file (no asset) - useful for testing orphaned pathnames  
        pub fn add_orphaned_pathname(mut self, guid: &str, pathname: &str) -> Self {
            self.entries.push(TestEntry::Pathname {
                guid: guid.to_string(),
                path: pathname.to_string(),
            });
            self
        }

        /// Build the Unity package as a gzipped tar archive
        pub fn build(self) -> Vec<u8> {
            let mut gz_buffer = Vec::new();
            {
                let gz_encoder = GzEncoder::new(&mut gz_buffer, Compression::default());
                let mut tar_builder = Builder::new(gz_encoder);

                for entry in &self.entries {
                    match entry {
                        TestEntry::Asset { guid, data } => {
                            add_tar_entry(&mut tar_builder, &format!("{guid}/asset"), data);
                        }
                        TestEntry::FolderAsset { guid } => {
                            let meta_content = format!(
                                r#"fileFormatVersion: 2
guid: {guid}
folderAsset: yes
DefaultImporter:
  externalObjects: {{}}
  userData: 
  assetBundleName: 
  assetBundleVariant:
"#
                            );
                            add_tar_entry(
                                &mut tar_builder,
                                &format!("{guid}/asset.meta"),
                                meta_content.as_bytes(),
                            );
                        }
                        TestEntry::RegularAsset {
                            guid,
                            data,
                            meta_content,
                        } => {
                            // Add asset file
                            add_tar_entry(&mut tar_builder, &format!("{guid}/asset"), data);

                            // Add metadata file
                            let default_meta = format!(
                                r#"fileFormatVersion: 2
guid: {guid}
TextureImporter:
  userData: 
"#
                            );
                            let meta = meta_content
                                .as_ref()
                                .map(|s| s.as_str())
                                .unwrap_or(&default_meta);
                            add_tar_entry(
                                &mut tar_builder,
                                &format!("{guid}/asset.meta"),
                                meta.as_bytes(),
                            );
                        }
                        TestEntry::Pathname { guid, path } => {
                            add_tar_entry(
                                &mut tar_builder,
                                &format!("{guid}/pathname"),
                                path.as_bytes(),
                            );
                        }
                    }
                }

                tar_builder.finish().unwrap();
            }
            gz_buffer
        }
    }

    const TEST_ASSET_DATA: &[u8] = b"test asset content";
    const TEST_PATHNAME: &str = "Assets/TestFile.txt";
    const TEST_GUID: &str = "guid123";

    fn add_tar_entry<W: io::Write>(builder: &mut Builder<W>, path: &str, data: &[u8]) {
        let mut header = tar::Header::new_gnu();
        header.set_path(path).unwrap();
        header.set_size(data.len() as u64);
        header.set_cksum();
        builder.append(&header, Cursor::new(data)).unwrap();
    }

    #[tokio::test]
    async fn test_extraction_asset_first() {
        use crate::memory_tracker::MemoryTracker;
        use crate::thread_pool::ThreadPool;
        use std::sync::Arc;

        let package_data = TestUnityPackageBuilder::new()
            .add_asset(TEST_GUID, TEST_PATHNAME, TEST_ASSET_DATA)
            .build();
        let cursor = Cursor::new(package_data);
        let decoder = GzDecoder::new(cursor);
        let mut archive = tar::Archive::new(decoder);

        let memory_tracker = Arc::new(MemoryTracker::new(MB)); // 1MB
        let thread_pool = ThreadPool::new(2, memory_tracker);
        let result = process_archive_entries(&mut archive, &thread_pool, 32 * MB).unwrap();

        // When asset comes before pathname, it will be orphaned initially
        assert!(result.context.has_orphaned_work()); // Asset becomes orphaned

        thread_pool.shutdown().await;
    }

    #[tokio::test]
    async fn test_extraction_pathname_first() {
        use crate::memory_tracker::MemoryTracker;
        use crate::thread_pool::ThreadPool;
        use std::sync::Arc;

        let package_data = TestUnityPackageBuilder::new()
            .add_orphaned_pathname(TEST_GUID, TEST_PATHNAME)
            .add_orphaned_asset(TEST_GUID, TEST_ASSET_DATA)
            .build();
        let cursor = Cursor::new(package_data);
        let decoder = GzDecoder::new(cursor);
        let mut archive = tar::Archive::new(decoder);

        let memory_tracker = Arc::new(MemoryTracker::new(MB)); // 1MB
        let thread_pool = ThreadPool::new(2, memory_tracker);
        let result = process_archive_entries(&mut archive, &thread_pool, 32 * MB).unwrap();

        // New system can extract immediately when asset comes after pathname
        assert!(!result.context.has_orphaned_work());

        thread_pool.shutdown().await;
    }

    #[tokio::test]
    async fn test_orphaned_asset_creation() {
        use crate::memory_tracker::MemoryTracker;
        use crate::thread_pool::ThreadPool;
        use std::sync::Arc;

        let package_data = TestUnityPackageBuilder::new()
            .add_orphaned_asset(TEST_GUID, TEST_ASSET_DATA)
            .build();
        let cursor = Cursor::new(package_data);
        let decoder = GzDecoder::new(cursor);
        let mut archive = tar::Archive::new(decoder);

        let memory_tracker = Arc::new(MemoryTracker::new(MB)); // 1MB
        let thread_pool = ThreadPool::new(2, memory_tracker);
        let result = process_archive_entries(&mut archive, &thread_pool, 32 * MB).unwrap();

        // Should create orphaned asset in root
        assert!(result.context.has_orphaned_work());
        assert_eq!(result.context.orphaned_count(), 1);

        thread_pool.shutdown().await;
    }

    #[tokio::test]
    async fn test_folder_asset_creation() {
        use crate::memory_tracker::MemoryTracker;
        use crate::thread_pool::ThreadPool;
        use std::sync::Arc;

        let package_data = TestUnityPackageBuilder::new()
            .add_folder_asset("folder123", "Assets/Scripts/")
            .build();
        let cursor = Cursor::new(package_data);
        let decoder = GzDecoder::new(cursor);
        let mut archive = tar::Archive::new(decoder);

        let memory_tracker = Arc::new(MemoryTracker::new(MB)); // 1MB
        let thread_pool = ThreadPool::new(2, memory_tracker.clone());
        let result = process_archive_entries(&mut archive, &thread_pool, 32 * MB).unwrap();
        create_folder_structures(&result.context, &thread_pool)
            .await
            .unwrap();

        // Should create folder structure and no orphaned assets
        assert!(!result.context.has_orphaned_work());

        thread_pool.shutdown().await;
    }

    #[tokio::test]
    async fn test_mixed_package() {
        use crate::memory_tracker::MemoryTracker;
        use crate::thread_pool::ThreadPool;
        use std::sync::Arc;

        let package_data = TestUnityPackageBuilder::new()
            .add_folder_asset("folder1", "Assets/Scripts/")
            .add_asset("asset1", "Assets/TestFile.txt", TEST_ASSET_DATA)
            .add_orphaned_asset("orphan1", b"orphaned data")
            .build();

        let cursor = Cursor::new(package_data);
        let decoder = GzDecoder::new(cursor);
        let mut archive = tar::Archive::new(decoder);

        let memory_tracker = Arc::new(MemoryTracker::new(MB)); // 1MB
        let thread_pool = ThreadPool::new(2, memory_tracker);
        let result = process_archive_entries(&mut archive, &thread_pool, 32 * MB).unwrap();

        // Should have 2 orphaned assets (asset1 and orphan1) since asset1 comes before its pathname
        assert!(result.context.has_orphaned_work());
        assert_eq!(result.context.orphaned_count(), 2);

        thread_pool.shutdown().await;
    }
}
