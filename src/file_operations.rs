use log::{debug, info, trace, warn};
use std::io::Read;
use std::path::Path;
use tokio::{fs, io};

use crate::archive_operations::AssetWriteError;
use crate::sanitize_path;

/// Handle file creation with proper error handling and directory creation
pub async fn create_file_with_content(
    content: Vec<u8>,
    target_path: &str,
    context_name: &str,
) -> Result<(), AssetWriteError> {
    let to_error = |error: io::Error| AssetWriteError {
        error,
        path: target_path.to_string(),
    };

    let sanitized_path = sanitize_path::sanitize_path(target_path).map_err(to_error)?;

    if target_path != sanitized_path {
        debug!("sanitizing path {target_path:?} => {sanitized_path:?}");
    }

    // Create parent directories
    if let Some(parent) = Path::new(&sanitized_path).parent() {
        fs::create_dir_all(parent).await.map_err(to_error)?;
    }

    info!("{context_name} to {sanitized_path:?}");
    let file = fs::File::create(&sanitized_path).await.map_err(to_error)?;
    let mut file_writer = io::BufWriter::new(file);

    use tokio::io::AsyncWriteExt;
    file_writer.write_all(&content).await.map_err(to_error)?;
    file_writer.flush().await.map_err(to_error)?;

    trace!("{context_name} written successfully");
    Ok(())
}

/// Handle file movement with proper error handling and directory creation
pub async fn move_file_to_target(
    source_path: &str,
    target_path: &str,
    context_name: &str,
) -> Result<(), AssetWriteError> {
    let to_error = |error: io::Error| AssetWriteError {
        error,
        path: target_path.to_string(),
    };

    let sanitized_path = sanitize_path::sanitize_path(target_path).map_err(to_error)?;

    if target_path != sanitized_path {
        debug!("sanitizing path {target_path:?} => {sanitized_path:?}");
    }

    // Create parent directories
    if let Some(parent) = Path::new(&sanitized_path).parent() {
        fs::create_dir_all(parent).await.map_err(to_error)?;
    }

    info!("{context_name} from {source_path} to {sanitized_path:?}");
    fs::rename(source_path, &sanitized_path)
        .await
        .map_err(to_error)?;

    trace!("{context_name} moved successfully");
    Ok(())
}

/// Handle file deletion with proper error handling
pub async fn delete_file(file_path: &str, context_name: &str) -> Result<(), AssetWriteError> {
    let to_error = |error: io::Error| AssetWriteError {
        error,
        path: file_path.to_string(),
    };

    warn!("{context_name}: {file_path}");
    fs::remove_file(file_path).await.map_err(to_error)?;

    trace!("{context_name} deleted successfully");
    Ok(())
}

/// Create directory structure
pub async fn create_directory_structure(
    target_path: &str,
    context_name: &str,
) -> Result<(), AssetWriteError> {
    let to_error = |error: io::Error| AssetWriteError {
        error,
        path: target_path.to_string(),
    };

    let sanitized_path = sanitize_path::sanitize_path(target_path).map_err(to_error)?;

    if target_path != sanitized_path {
        debug!("sanitizing path {target_path:?} => {sanitized_path:?}");
    }

    info!("{context_name}: {sanitized_path:?}");
    fs::create_dir_all(&sanitized_path)
        .await
        .map_err(to_error)?;

    trace!("{context_name} created successfully");
    Ok(())
}

/// Stream asset data directly to disk using chunked reads
pub fn stream_asset_to_pathname<R: Read>(
    mut reader: R,
    target_path: &str,
    context_name: &str,
    chunk_size: u64,
) -> Result<(), AssetWriteError> {
    let to_error = |error: io::Error| AssetWriteError {
        error,
        path: target_path.to_string(),
    };

    let sanitized_path = sanitize_path::sanitize_path(target_path).map_err(to_error)?;

    if target_path != sanitized_path {
        debug!("sanitizing path {target_path:?} => {sanitized_path:?}");
    }

    // Create parent directories
    if let Some(parent) = Path::new(&sanitized_path).parent() {
        std::fs::create_dir_all(parent).map_err(to_error)?;
    }

    info!("{context_name} streaming to {sanitized_path:?}");

    let mut file = std::fs::File::create(&sanitized_path).map_err(to_error)?;

    // Stream in chunks to avoid loading entire file in memory
    let mut buffer = vec![0; chunk_size as usize];
    loop {
        let bytes_read = reader.read(&mut buffer).map_err(to_error)?;
        if bytes_read == 0 {
            break;
        }

        use std::io::Write;
        file.write_all(&buffer[..bytes_read]).map_err(to_error)?;
    }

    use std::io::Write;
    file.flush().map_err(to_error)?;
    trace!("{context_name} streamed successfully");
    Ok(())
}

/// Stream orphaned asset directly to disk using chunked reads
pub fn stream_orphaned_asset<R: Read>(
    reader: R,
    orphan_name: &str,
    context_name: &str,
    chunk_size: u64,
) -> Result<(), AssetWriteError> {
    stream_asset_to_pathname(reader, orphan_name, context_name, chunk_size)
}
