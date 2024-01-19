use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use argparse::{ArgumentParser, IncrBy, Store};
use flate2::read::GzDecoder;
use log::{debug, error, info, trace, warn, LevelFilter};
use simple_logger::SimpleLogger;
use tokio::io::AsyncWriteExt;
use tokio::task::JoinHandle;
use tokio::{fs, io};

mod sanitize_path;

struct Config {
    input_path: String,
    log_level: LevelFilter,
}

struct AssetWriteError {
    error: io::Error,
    path: String,
}

impl fmt::Display for AssetWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.path, self.error)
    }
}

type AssetMap = HashMap<PathBuf, Vec<u8>>;
type FolderSet = HashSet<OsString>;
type ExtractTask = Vec<JoinHandle<Result<(), AssetWriteError>>>;

fn parse_arguments() -> Config {
    let mut verbose = 0;
    let mut quiet = 0;
    let mut input_path = String::new();

    {
        let mut parser = ArgumentParser::new();
        parser.set_description("Unity package extractor");
        parser.refer(&mut quiet).add_option(
            &["-q"],
            IncrBy(1),
            "decrease verbosity, hide warnings.",
        );
        parser
            .refer(&mut verbose)
            .add_option(&["-v"], IncrBy(1), "increase verbosity; up to 3.");
        parser
            .refer(&mut input_path)
            .add_argument("input", Store, "*.unitypackage file")
            .required();
        parser.parse_args_or_exit();
    }

    let log_level = match verbose - quiet {
        ..=-1 => LevelFilter::Error,
        0 => LevelFilter::Warn,
        1 => LevelFilter::Info,
        2 => LevelFilter::Debug,
        3.. => LevelFilter::Trace,
    };

    Config {
        input_path,
        log_level,
    }
}

fn read_asset_to_memory<R: Read>(
    assets: &mut AssetMap,
    mut entry: tar::Entry<'_, R>,
    path: PathBuf,
) -> Result<(), io::Error> {
    debug!("reading asset to memory {:?}", path);
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

fn check_for_folders<R: Read>(
    folders: &mut FolderSet,
    mut entry: tar::Entry<'_, R>,
    path: PathBuf,
) -> Result<(), io::Error> {
    debug!("reading asset to memory {:?}", path);
    let mut metadata = String::new();
    entry.read_to_string(&mut metadata)?;
    if metadata.contains("folderAsset: yes\n") {
        folders.insert(path.into_os_string());
    }
    Ok(())
}

fn read_destination_path_and_write<R: Read>(
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
        debug!("sanitizing path {:?} => {:?}", path_name, target_path);
    }

    if let Some(parent) = Path::new(&target_path).parent() {
        fs::create_dir_all(parent).await.map_err(to_asset_error)?;
    }

    info!("extracting {} to {:?}", asset_hash, target_path);
    let file = fs::File::create(&target_path)
        .await
        .map_err(to_asset_error)?;
    let mut file_writer = io::BufWriter::new(file);
    file_writer
        .write_all(&asset_data)
        .await
        .map_err(to_asset_error)?;
    file_writer.flush().await.map_err(to_asset_error)?;
    trace!("{} is written to disk", asset_hash);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = parse_arguments();
    SimpleLogger::new().with_level(config.log_level).init()?;
    debug!("opening unitypackage file at {}", &config.input_path);
    let file = std::fs::File::open(&config.input_path);

    if let Err(err) = file {
        error!("cannot open file at {}: {}", config.input_path, err);
        std::process::exit(2);
    }

    let decoder = GzDecoder::new(file?);
    let mut archive = tar::Archive::new(decoder);
    let mut assets: AssetMap = HashMap::new();
    let mut folders: FolderSet = HashSet::new();
    let mut tasks: ExtractTask = Vec::new();

    debug!("iterating archive's entries");
    for entry_result in archive.entries()? {
        let entry = match entry_result {
            Ok(file) => file,
            Err(e) => {
                warn!("error reading entry from archive: {}", e);
                continue;
            }
        };

        let path = match entry.path() {
            Ok(p) => p.to_path_buf(),
            Err(e) => {
                warn!("errors reading path from entry: {}", e);
                continue;
            }
        };

        if path.ends_with("asset") {
            read_asset_to_memory(&mut assets, entry, path)?;
        } else if path.ends_with("asset.meta") {
            check_for_folders(&mut folders, entry, path)?;
        } else if path.ends_with("pathname") {
            read_destination_path_and_write(&mut assets, &folders, &mut tasks, entry, path)?;
        } else if path.ends_with("/") {
            trace!("skipping folder {}", path.display());
        } else {
            trace!("skipping entry with name {}", path.display())
        }
    }

    debug!("end of archive");
    for task in tasks {
        match task.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                warn!("failed to write asset: {}", e);
            }
            Err(e) => {
                warn!("an extraction task has failed: {}", e);
            }
        }
    }
    info!("done");

    Ok(())
}
