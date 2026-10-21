use crate::archive_operations::*;
use crate::memory_tracker::MemoryTracker;
use argparse::{ArgumentParser, IncrBy, Store};
use bytesize::ByteSize;
use flate2::read::GzDecoder;
use log::{debug, error, info, LevelFilter};
use simple_logger::SimpleLogger;
use std::error::Error;
use std::sync::Arc;

mod archive_operations;
mod file_operations;
mod memory_tracker;
mod sanitize_path;
mod thread_pool;

struct Config {
    input_path: String,
    log_level: LevelFilter,
    writer_threads: usize,
    max_buffer_memory: u64, // bytes
    stream_threshold: u64,  // bytes
}

fn parse_arguments() -> Config {
    let mut verbose = 0;
    let mut quiet = 0;
    let mut input_path = String::new();
    let mut max_buffer_memory_str = "1GB".to_string();
    let mut stream_threshold_str = "32MB".to_string();
    let mut writer_threads = 4;

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
        parser.refer(&mut max_buffer_memory_str).add_option(
            &["-m", "--max-memory"],
            Store,
            "max buffer memory (e.g. 512MB, 2GB, default: 1GB)",
        );
        parser.refer(&mut stream_threshold_str).add_option(
            &["--stream-threshold"],
            Store,
            "files larger than this size will be streamed (e.g. 32MB, 1GB, default: 32MB)",
        );
        parser.refer(&mut writer_threads).add_option(
            &["-j", "--jobs", "--writer-threads"],
            Store,
            "number of writer threads for small files (default: 4, max: numcpu)",
        );
        parser
            .refer(&mut input_path)
            .add_argument("input", Store, "*.unitypackage file")
            .required();
        parser.parse_args_or_exit();
    }

    // Parse memory limit using bytesize
    let max_buffer_memory = max_buffer_memory_str
        .parse::<ByteSize>()
        .map_err(|e| format!("Invalid memory size '{max_buffer_memory_str}': {e}"))
        .unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        })
        .as_u64();

    // Parse stream threshold using bytesize
    let stream_threshold = stream_threshold_str
        .parse::<ByteSize>()
        .map_err(|e| format!("Invalid stream threshold '{stream_threshold_str}': {e}"))
        .unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        })
        .as_u64();

    // Validate writer threads
    let max_cpus = num_cpus::get();
    if writer_threads > max_cpus {
        eprintln!("Writer threads ({writer_threads}) cannot exceed number of CPUs ({max_cpus})");
        std::process::exit(1);
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
        writer_threads,
        max_buffer_memory,
        stream_threshold,
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let config = parse_arguments();
    SimpleLogger::new().with_level(config.log_level).init()?;

    debug!("opening unitypackage file at {}", config.input_path);
    let file = std::fs::File::open(&config.input_path);

    if let Err(err) = file {
        error!("cannot open file at {}: {}", config.input_path, err);
        std::process::exit(2);
    }
    let file = file?;

    let memory_tracker = Arc::new(MemoryTracker::new(config.max_buffer_memory));

    info!(
        "starting {} writer threads with {} memory limit and {} stream threshold",
        config.writer_threads,
        ByteSize(config.max_buffer_memory).display().si(),
        ByteSize(config.stream_threshold).display().si()
    );

    let mut archive = tar::Archive::new(GzDecoder::new(&file));
    let extraction = extract_archive(
        &mut archive,
        config.writer_threads,
        memory_tracker,
        config.stream_threshold,
    )
    .await;
    let failures = match extraction {
        Ok(failures) => failures,
        Err(err) => {
            error!("cannot extract {}: {}", config.input_path, err);
            std::process::exit(1);
        }
    };
    if failures > 0 {
        error!("{failures} write operations failed, see the warnings above");
        std::process::exit(1);
    }
    info!("extraction complete");

    Ok(())
}
