use log::{debug, info, trace, warn};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use tokio::task::JoinHandle;
use tokio::{fs, io};

use crate::sanitize_path;

pub fn process_archive_entries<R: Read>(
    archive: &mut tar::Archive<R>,
    assets: &mut AssetMap,
    folders: &mut FolderSet,
    tasks: &mut ExtractTask,
) -> Result<(), io::Error> {
    debug!("iterating archive's entries");
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

        if path.ends_with("asset") {
            read_asset_to_memory(assets, entry, path)?;
        } else if path.ends_with("asset.meta") {
            check_for_folders(folders, entry, path)?;
        } else if path.ends_with("pathname") {
            read_destination_path_and_write(assets, folders, tasks, entry, path)?;
        } else if path.ends_with("/") {
            trace!("skipping folder {}", path.display());
        } else {
            trace!("skipping entry with name {}", path.display())
        }
    }
    Ok(())
}

pub struct AssetWriteError {
    error: io::Error,
    path: String,
}

impl fmt::Display for AssetWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.path, self.error)
    }
}

pub type AssetMap = HashMap<PathBuf, Vec<u8>>;
pub type FolderSet = HashSet<OsString>;
pub type ExtractTask = Vec<JoinHandle<Result<(), AssetWriteError>>>;

pub fn read_asset_to_memory<R: Read>(
    assets: &mut AssetMap,
    mut entry: tar::Entry<'_, R>,
    path: PathBuf,
) -> Result<(), io::Error> {
    debug!("reading asset to memory {path:?}");
    let mut asset_data = Vec::new();
    entry.read_to_end(&mut asset_data)?;
    trace!(
        "saving {:?} with {} bytes to memory",
        path,
        asset_data.len(),
    );
    assets.insert(path, asset_data);
    Ok(())
}

pub fn check_for_folders<R: Read>(
    folders: &mut FolderSet,
    mut entry: tar::Entry<'_, R>,
    path: PathBuf,
) -> Result<(), io::Error> {
    debug!("reading asset to memory {path:?}");
    let mut metadata = String::new();
    entry.read_to_string(&mut metadata)?;
    if metadata.contains("folderAsset: yes\n") {
        folders.insert(path.into_os_string());
    }
    Ok(())
}

pub fn read_destination_path_and_write<R: Read>(
    assets: &mut AssetMap,
    folders: &FolderSet,
    tasks: &mut ExtractTask,
    mut entry: tar::Entry<'_, R>,
    path: PathBuf,
) -> Result<(), io::Error> {
    let mut path_name = String::new();
    entry.read_to_string(&mut path_name)?;

    let asset_path = path.parent().unwrap().join("asset");
    if let Some(asset_data) = assets.remove(&asset_path) {
        tasks.push(tokio::spawn(async move {
            write_asset_to_pathname(asset_data, path.to_string_lossy().to_string(), path_name).await
        }));
    } else {
        let path_string = path.into_os_string();
        if folders.contains(&path_string) {
            warn!("no asset data found for {}", path_name.escape_default());
        }
    }
    Ok(())
}

async fn write_asset_to_pathname(
    asset_data: Vec<u8>,
    entry_hash: String,
    path_name: String,
) -> Result<(), AssetWriteError> {
    let to_asset_error = |error: io::Error| AssetWriteError {
        error,
        path: path_name.clone(),
    };
    let target_path = sanitize_path::sanitize_path(&path_name).map_err(to_asset_error)?;
    let asset_hash: &str;

    match entry_hash.find('/') {
        Some(idx) => {
            (asset_hash, _) = entry_hash.split_at(idx);
        }
        None => {
            asset_hash = &entry_hash;
        }
    }

    if path_name != target_path {
        debug!("sanitizing path {path_name:?} => {target_path:?}");
    }

    if let Some(parent) = Path::new(&target_path).parent() {
        fs::create_dir_all(parent).await.map_err(to_asset_error)?;
    }

    info!("extracting {asset_hash} to {target_path:?}");
    let file = fs::File::create(&target_path)
        .await
        .map_err(to_asset_error)?;
    let mut file_writer = io::BufWriter::new(file);
    file_writer
        .write_all(&asset_data)
        .await
        .map_err(to_asset_error)?;
    file_writer.flush().await.map_err(to_asset_error)?;
    trace!("{asset_hash} is written to disk");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::bufread::GzDecoder;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Cursor;
    use tar::{Builder, Header};

    fn create_test_unitypackage(asset_first: bool) -> Vec<u8> {
        let mut gz_buffer = Vec::new();
        {
            let gz_encoder = GzEncoder::new(&mut gz_buffer, Compression::default());
            let mut tar_builder = Builder::new(gz_encoder);

            // Asset data
            let asset_data = b"test asset content";
            let pathname_data = b"Assets/TestFile.txt";

            if asset_first {
                // Well-ordered: asset before pathname
                add_tar_entry(&mut tar_builder, "guid123/asset", asset_data);
                add_tar_entry(&mut tar_builder, "guid123/pathname", pathname_data);
            } else {
                // Problematic: pathname before asset
                add_tar_entry(&mut tar_builder, "guid123/pathname", pathname_data);
                add_tar_entry(&mut tar_builder, "guid123/asset", asset_data);
            }

            tar_builder.finish().unwrap();
        }
        gz_buffer
    }

    fn add_tar_entry<W: std::io::Write>(builder: &mut Builder<W>, path: &str, data: &[u8]) {
        let mut header = Header::new_gnu();
        header.set_path(path).unwrap();
        header.set_size(data.len() as u64);
        header.set_cksum();
        builder.append(&header, Cursor::new(data)).unwrap();
    }

    #[tokio::test]
    async fn test_extraction_asset_first() {
        // Test current behavior - should work
        let package_data = create_test_unitypackage(true);
        let cursor = Cursor::new(package_data);
        let decoder = GzDecoder::new(cursor);
        let mut archive = tar::Archive::new(decoder);

        let mut assets: AssetMap = HashMap::new();
        let mut folders: FolderSet = HashSet::new();
        let mut tasks: ExtractTask = Vec::new();

        process_archive_entries(&mut archive, &mut assets, &mut folders, &mut tasks).unwrap();

        // Should have 1 task queued
        assert_eq!(tasks.len(), 1);
        // Asset should be removed from map (consumed)
        assert!(assets.is_empty());
    }

    #[tokio::test]
    async fn test_extraction_pathname_first() {
        // Test problematic case - currently fails
        let package_data = create_test_unitypackage(false);
        let cursor = Cursor::new(package_data);
        let decoder = GzDecoder::new(cursor);
        let mut archive = tar::Archive::new(decoder);

        let mut assets: AssetMap = HashMap::new();
        let mut folders: FolderSet = HashSet::new();
        let mut tasks: ExtractTask = Vec::new();

        process_archive_entries(&mut archive, &mut assets, &mut folders, &mut tasks).unwrap();

        // Currently: no tasks queued because asset wasn't available when pathname was processed
        assert_eq!(tasks.len(), 0);
        // Asset is still in map (not consumed)
        assert_eq!(assets.len(), 1);
    }

    #[tokio::test]
    async fn test_extraction_pathname_first_should_work() {
        // This test shows what SHOULD happen - it will fail until we fix the issue
        let package_data = create_test_unitypackage(false);
        let cursor = Cursor::new(package_data);
        let decoder = GzDecoder::new(cursor);
        let mut archive = tar::Archive::new(decoder);

        let mut assets: AssetMap = HashMap::new();
        let mut folders: FolderSet = HashSet::new();
        let mut tasks: ExtractTask = Vec::new();

        process_archive_entries(&mut archive, &mut assets, &mut folders, &mut tasks).unwrap();

        // EXPECTED behavior: should have 1 task queued regardless of order
        assert_eq!(
            tasks.len(),
            1,
            "Should extract asset even when pathname comes first"
        );
        // Asset should be consumed
        assert!(
            assets.is_empty(),
            "Asset should be consumed after processing"
        );
    }
}
