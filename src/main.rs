use crate::archive_operations::*;
use argparse::{ArgumentParser, IncrBy, Store};
use flate2::read::GzDecoder;
use log::{debug, error, info, warn, LevelFilter};
use simple_logger::SimpleLogger;
use std::collections::{HashMap, HashSet};
use std::error::Error;

mod archive_operations;
mod sanitize_path;

struct Config {
    input_path: String,
    log_level: LevelFilter,
}

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
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

    process_archive_entries(&mut archive, &mut assets, &mut folders, &mut tasks)?;

    debug!("end of archive");
    for task in tasks {
        match task.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                warn!("failed to write asset: {e}");
            }
            Err(e) => {
                warn!("an extraction task has failed: {e}");
            }
        }
    }
    info!("done");

    Ok(())
}
