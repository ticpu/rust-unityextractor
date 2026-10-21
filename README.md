# Unity Package Extractor

Extracts the assets of a `.unitypackage` file to disk without Unity. A Unity package is a gzipped tar archive holding one directory per asset GUID; this tool reads it in a single pass and writes each asset under the path Unity would import it to.

## Install

Prebuilt binaries for Linux and Windows (x86_64) are attached to each [release](https://github.com/ticpu/rust-unityextractor/releases). To build from source:

```sh
cargo build --release
# binary at target/release/rust-unityextractor
```

## Usage

```sh
rust-unityextractor [options] <file.unitypackage>
```

Assets are extracted into the current working directory, under their package paths (usually `Assets/...`). Existing files with the same path are overwritten. On Windows you can also drag a package onto the executable.

| Option | Default | Meaning |
| --- | --- | --- |
| `-v` | | More logging: once for info, twice for debug, three times for trace |
| `-q` | | Less logging: hide warnings, show errors only |
| `--writer-threads N` | 4 | Worker threads writing small files, at most the CPU count |
| `-m`, `--max-memory SIZE` | 1GB | Memory allowed for buffered files waiting to be written |
| `--stream-threshold SIZE` | 32MB | Files at least this large are streamed straight to disk |

Sizes accept units such as `512MB` or `2GB`.

## What gets extracted

- Every asset is written to the path recorded in its `pathname` entry.
- Folder assets become directories, including empty ones.
- `.meta` files are read to detect folders and are not written out.
- Paths are sanitized: backslashes become separators, leading and trailing slashes, dots and whitespace are stripped, and a path with `..` in its directory part is refused with a warning.
- An asset whose `pathname` comes later in the archive is written to a temporary file in the working directory and moved once its path is known; one that never gets a path is deleted with a warning.

## Memory and speed

The archive is read sequentially. Files below the stream threshold are buffered and handed to the writer threads, as long as the buffered total stays under `--max-memory`; when it would not, the file is written directly instead of waiting. Files at or above the threshold are always streamed, so memory use stays bounded regardless of package size. Raising `--writer-threads` helps on packages with many small files on fast storage; lowering `--max-memory` caps memory on large packages at some cost in throughput.

## Exit status

0 on success, 2 when the package cannot be opened or an option is unknown, 1 on an invalid size or a write error. A single asset that fails to write is logged as a warning and does not stop the extraction.

## License

BSD 3-Clause, see [LICENSE](LICENSE).
