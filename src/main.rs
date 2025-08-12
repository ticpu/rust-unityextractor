use crate::archive_operations::*;
use crate::memory_tracker::MemoryTracker;
use crate::thread_pool::ThreadPool;
use argparse::{ArgumentParser, IncrBy, Store};
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
    max_concurrent_writes: usize,
    max_buffer_memory: u64, // bytes
    stream_threshold: u64,  // bytes
}

fn parse_arguments() -> Config {
    let mut verbose = 0;
    let mut quiet = 0;
    let mut input_path = String::new();
    let mut max_concurrent_writes = 4; // Default to 4 writer threads
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
        parser.refer(&mut max_concurrent_writes).add_option(
            &["-j", "--jobs"],
            Store,
            "max concurrent file writes (default: 4, max: numcpu)",
        );
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
            &["--writer-threads"],
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
        .parse::<bytesize::ByteSize>()
        .map_err(|e| format!("Invalid memory size '{max_buffer_memory_str}': {e}"))
        .unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        })
        .as_u64();

    // Parse stream threshold using bytesize
    let stream_threshold = stream_threshold_str
        .parse::<bytesize::ByteSize>()
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
    max_concurrent_writes = writer_threads;

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
        max_concurrent_writes,
        max_buffer_memory,
        stream_threshold,
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
    let file = file?;

    // Create memory tracker and thread pool
    let memory_tracker = Arc::new(MemoryTracker::new(config.max_buffer_memory));
    let thread_pool = ThreadPool::new(config.max_concurrent_writes, memory_tracker.clone());

    info!(
        "starting {} writer threads with {}MB memory limit and {}MB stream threshold",
        config.max_concurrent_writes,
        config.max_buffer_memory / (1024 * 1024),
        config.stream_threshold / (1024 * 1024)
    );

    // Pass 1: Read compressed file sequentially
    // - Read pathname and metadata and save them in memory
    // - Read asset, route based on size: large files stream synchronously, small files queue
    info!("pass 1: processing archive entries");
    let decoder = GzDecoder::new(&file);
    let mut archive = tar::Archive::new(decoder);
    let result = process_archive_entries(&mut archive, &thread_pool, config.stream_threshold)?;

    // Create folder structures for pathnames that are marked as folders
    create_folder_structures(&result.context, &thread_pool).await?;

    // Pass 2: Handle orphaned assets
    // - Iterate the orphaned asset and move them where they belong, creating missing directories as needed
    // - Show warn!() for assets missing pathname and delete them
    if result.context.has_orphaned_work() {
        info!(
            "pass 2: processing {} orphaned assets",
            result.context.orphaned_count()
        );
        process_orphaned_assets(result.context, &thread_pool).await?;
    }

    // Shutdown thread pool and wait for all tasks to complete
    info!("waiting for all write tasks to complete");
    thread_pool.shutdown().await;
    info!("extraction complete");

    Ok(())
}
