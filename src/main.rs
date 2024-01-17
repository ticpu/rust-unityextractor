use argparse::{ArgumentParser, Store, StoreTrue};
use flate2::read::GzDecoder;
use log::{info, error, LevelFilter};
use simple_logger::SimpleLogger;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use tar::Archive;
use tokio::fs;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command-line arguments
    let mut verbose = false;
    let mut input_path = String::new();
    {
        let mut parser = ArgumentParser::new();
        parser.set_description("Unity package extractor");
        parser
            .refer(&mut verbose)
            .add_option(&["-v"], StoreTrue, "Verbose mode");
        parser
            .refer(&mut input_path)
            .add_argument("input", Store, "*.unitypackage file")
            .required();
        parser.parse_args_or_exit();
    }

    if verbose {
        SimpleLogger::new()
            .with_level(LevelFilter::Info)
            .init()
            .unwrap();
    } else {
        SimpleLogger::new()
            .with_level(LevelFilter::Error)
            .init()
            .unwrap();
    }

    // Open the unitypackage file
    let file = File::open(&input_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    let mut assets: HashMap<PathBuf, Vec<u8>> = HashMap::new();

    // Iterate over each entry in the archive
    for entry_result in archive.entries()? {
        let mut entry = entry_result?;
        let path = entry.path()?.to_path_buf();

        // If the entry is an 'asset' file, read its content
        if path.ends_with("asset") {
            let mut asset_data = Vec::new();
            entry.read_to_end(&mut asset_data)?;
            assets.insert(path.clone(), asset_data);
        }
        // If the entry is a 'pathname' file, read its content and write the asset
        else if path.ends_with("pathname") {
            let mut pathname = String::new();
            entry.read_to_string(&mut pathname)?;

            // Sanitize the pathname
            let pathname = pathname.trim().replace('\\', "/");
            let target_path = PathBuf::from(&pathname);

            // Create directories for the target path
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent).await?;
            }

            // Write the asset data to the target path
            let asset_path = path.parent().unwrap().join("asset");
            if let Some(asset_data) = assets.remove(&asset_path) {
                if let Err(e) = fs::write(&target_path, &asset_data).await {
                    error!("Failed to extract file: {:?}, error: {:?}", &target_path, e)
                }
                else {
                    info!("Extracted: {}", pathname);
                }
            }
        }
    }

    Ok(())
}
