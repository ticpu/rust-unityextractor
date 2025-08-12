# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Unity Package Extractor is a Rust-based command-line tool that extracts assets from `.unitypackage` files. The tool parses Unity package archives (which are gzipped tar files) and extracts all assets to the working directory using async I/O operations.

## Commands

### Build and Run
- `cargo build` - Build the project
- `cargo run -- <path-to-unitypackage>` - Run the extractor on a Unity package file
- `cargo test` - Run unit tests
- `cargo clippy` - Run Rust linter (clippy)

### Logging Levels
The tool supports verbose logging:
- `-v` - Info level logging
- `-vv` - Debug level logging  
- `-vvv` - Trace level logging
- `-q` - Hide warnings (quiet mode)

### Concurrency and Memory Control
- `-j/--jobs N` - Max concurrent file writes (default: 4, max: numcpu)
- `--writer-threads N` - Number of writer threads for small files (default: 4, max: numcpu)
- `-m/--max-memory SIZE` - Max buffer memory (e.g. 512MB, 2GB, default: 1GB)
- `--stream-threshold SIZE` - Files larger than this size will be streamed (e.g. 32MB, 1GB, default: 32MB)

## Architecture

### Core Components

**Main Entry Point (`main.rs`)**
- Argument parsing with configurable verbosity levels, concurrency, and streaming controls
- File opening and gzip decompression setup
- Fixed thread pool creation with memory-bounded queue
- Size-based routing: large files stream synchronously, small files use thread pool

**Archive Processing (`archive_operations.rs`)**
- `process_archive_entries()` - Size-based routing and extraction system
- `ExtractionContext` - Centralized state management for pathnames, metadata, orphaned assets, and stream threshold
- `ExtractionResult` - Contains extraction context (no longer holds async tasks)
- Entry type detection (asset files, metadata, pathnames, folders)
- Size-based extraction: files >= stream threshold are streamed synchronously
- Small files are queued for thread pool processing or fallback to synchronous streaming

**Memory Tracking (`memory_tracker.rs`)**
- `MemoryTracker` - Simplified memory tracking for queue buffer management
- Atomic tracking of allocated buffer memory for queued tasks
- Memory limit enforcement for thread pool queue admission control
- No system memory monitoring (user configures buffer limits directly)

**Thread Pool (`thread_pool.rs`)**
- `ThreadPool` - Fixed number of async worker threads for small file processing
- Memory-bounded queue using `MemoryTracker` for admission control
- Graceful fallback: when queue is full, files are processed synchronously
- Automatic memory tracking with reserve/release for queued tasks

**Path Sanitization (`sanitize_path.rs`)**
- Security-focused path cleaning to prevent directory traversal attacks
- Handles Windows/Unix path separators
- Validates against `..` directory traversal in paths

**File Operations (`file_operations.rs`)**
- Async file writing functions for buffered data
- Synchronous streaming functions for large files using chunked I/O
- `stream_asset_to_pathname()` - Direct reader-to-file streaming with configurable chunk size
- `stream_orphaned_asset()` - Streaming for orphaned assets

### Data Flow

**Pass 1: Size-Based Archive Processing**
1. `pathname` files are read and stored in memory immediately
2. `asset.meta` files are checked for folder markers and stored
3. `asset` files are processed based on size:
   - **Large files (>= stream threshold)**: Stream directly to disk synchronously using chunked I/O
   - **Small files (< stream threshold)**: 
     - Try to queue for async thread pool processing
     - If queue full (memory limit reached): fallback to synchronous streaming
   - **Folder creation**: Queued for thread pool or processed synchronously if queue full
4. **No async task accumulation**: All processing happens immediately during archive reading

**Pass 2: Orphaned Asset Processing**
1. Process orphaned assets using same size-based routing
2. Move orphaned assets to proper locations using stored pathnames (queued or synchronous)
3. Warn and delete assets missing pathnames (queued or synchronous)

### Key Data Structures

- `ExtractionContext` - Centralized state management with stream threshold
- `ExtractionResult` - Contains context only (no async tasks)
- `PathNameMap` - HashMap storing pathnames for assets
- `FolderSet` - HashSet tracking folder entries
- `OrphanedAssets` - Vector tracking orphaned asset files in root
- `WriteTask` - Boxed async closure for thread pool tasks
- `ThreadPool` - Fixed worker threads with memory-bounded queue

## Important Patterns

### Error Handling
- Non-fatal errors (individual entry failures) are logged as warnings and processing continues
- Fatal errors (file I/O, archive corruption) terminate the program
- Custom `AssetWriteError` type for async write failures

### Async Design and Concurrency Control
- **Fixed thread pool**: Limited number of worker threads (default: 4, max: numcpu)
- **Memory-bounded queue**: Tasks only queued if memory limit allows
- **Size-based routing**: Large files bypass queue and stream synchronously
- **Graceful fallback**: Queue full triggers synchronous processing
- **No task accumulation**: All work completes during archive processing
- Uses tokio for async runtime and file I/O

### Asset Processing
- **Size-based processing**: Large files (>= stream threshold) streamed synchronously
- **Memory-aware queueing**: Small files queued if memory available, otherwise streamed
- **Immediate processing**: No task accumulation, work completes during archive reading
- **Chunked streaming**: Large files read/written in configurable chunks (default: stream threshold size)
- Assets with pathnames are extracted directly to target locations
- Assets without pathnames are extracted to root as orphaned files
- Orphaned assets are moved to proper locations in Pass 2
- Missing pathname assets are warned about and deleted

### Security Considerations
- Path sanitization prevents directory traversal attacks
- Paths containing `..` in directory components are rejected
- Leading/trailing whitespace and path separators are stripped

### Resource Management
- **Memory safety**: Configurable buffer limits prevent memory exhaustion
- **Stream threshold**: Large files bypass memory entirely via direct streaming
- **Fixed concurrency**: Controlled number of worker threads prevents resource overwhelming
- **Queue admission control**: Memory-bounded queue prevents unbounded buffering
- **Bytesize parsing**: Human-friendly memory specifications (e.g., "512MB", "2GB", "32MB")
- **Chunked I/O**: Large files processed in chunks to avoid loading entire files in memory

## Development Guidelines

This file documents architecture and patterns, not changelogs or issue tracking.

### Code Style
- When possible, use format!("{variable_name}") directly
- When you need to test compilation, always use `cargo check`
- Use named constants in tests instead of magic numbers
- Keep test data sizes small for fast execution
- dead_code is not allowed