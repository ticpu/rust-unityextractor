# Unity Package Extractor

Extracts the assets of a `.unitypackage` file to disk without Unity. A Unity package is a gzipped tar archive holding one directory per asset GUID; this tool reads it in a single pass and writes each asset under the path Unity would import it to.

## Install

Each [release](https://github.com/ticpu/rust-unityextractor/releases) has static Linux binaries for x86_64 and aarch64, which run on any distribution, and a Windows x86_64 exe. Every asset comes with a GPG signature, and `SHA256SUMS` lists their checksums.

On Debian and Ubuntu the package comes from the [apt.ticpu.net](https://github.com/ticpu/apt-ticpu-net) archive. It is built for amd64 and arm64, needs no other package, and the same `.deb` files are attached to each release:

```sh
curl -fsSLO https://apt.ticpu.net/ticpu-archive-keyring.deb
sudo dpkg -i ticpu-archive-keyring.deb
sudo apt-get update
sudo apt-get install rust-unityextractor
```

The keyring package installs the archive key and a sources entry for your release; the archive's README covers setting it up by hand and what else it carries.

On Arch Linux, the AUR has `rust-unityextractor`, built from source, and `rust-unityextractor-bin`, which installs the signed static binary.

To build from source, `cargo build --release` puts the binary in target/release/. `./build.sh` cross-compiles every release target into dist/ inside a container, with podman or docker.

## Usage

```sh
rust-unityextractor [options] <file.unitypackage>
```

Assets are extracted into the current working directory, under their package paths (usually `Assets/...`). Existing files with the same path are overwritten. On Windows you can also drag a package onto the executable.

| Option | Default | Meaning |
| --- | --- | --- |
| `-v` | | More logging: once for info, twice for debug, three times for trace |
| `-q` | | Less logging: hide warnings, show errors only |
| `-j`, `--jobs`, `--writer-threads N` | 4 | Worker threads writing small files, at most the CPU count |
| `-m`, `--max-memory SIZE` | 1GB | Memory allowed for buffered files waiting to be written |
| `--stream-threshold SIZE` | 32MB | Files at least this large are streamed straight to disk |

Sizes take a unit: MB and GB are powers of 1000, MiB and GiB powers of 1024.

## What gets extracted

- Every asset is written to the path recorded in its `pathname` entry.
- Folder assets become directories, including empty ones.
- `.meta` files are read to detect folders and are not written out.
- Paths are sanitized: backslashes become separators, surrounding whitespace and empty or `.` components are dropped, leading `..` components are removed, and a `..` anywhere else is refused with a warning. Dots inside names, such as `.gitignore`, are kept.
- An asset whose `pathname` comes later in the archive is written to a temporary file in the working directory and moved once its path is known; one that never gets a path is deleted with a warning.

## Memory and speed

The archive is read sequentially. Files below the stream threshold are buffered and handed to the writer threads, as long as the buffered total stays under `--max-memory`; when it would not, the file is written directly instead of waiting. Files at or above the threshold are always streamed, so memory use stays bounded regardless of package size. Raising `--writer-threads` helps on packages with many small files on fast storage; lowering `--max-memory` caps memory on large packages at some cost in throughput.

## Exit status

0 on success, 2 when the package cannot be opened or an option is unknown, 1 on an invalid size, a corrupt archive or a write error. Files written before a corrupt part of the archive stay on disk. An asset that fails to write is logged as a warning and the extraction goes on; the run then ends with the number of failures and exit code 1.

## License

BSD 3-Clause, see [LICENSE](LICENSE).
