FROM docker.io/library/debian:bookworm AS build

RUN --mount=type=cache,target=/var/cache/apt,id=apt-cache-bookworm \
    --mount=type=cache,target=/var/lib/apt/lists,id=apt-lists-bookworm \
    apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    build-essential \
    gcc-mingw-w64-x86-64

ENV CARGO_HOME=/usr/local/cargo RUSTUP_HOME=/usr/local/rustup
ENV PATH="/usr/local/cargo/bin:${PATH}"

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path

# musl targets link static-pie through rust-lld against the libc rustup ships,
# so neither arch needs a cross gcc and the binary runs on any Linux.
RUN rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-pc-windows-gnu

ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C target-feature=+crt-static -C linker=rust-lld" \
    CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_RUSTFLAGS="-C target-feature=+crt-static -C linker=rust-lld" \
    CARGO_PROFILE_RELEASE_STRIP=true \
    CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc \
    CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src ./src

# A dependency pulling in a C library would silently reintroduce a libc floor,
# so the static linking is asserted rather than trusted.
RUN --mount=type=cache,target=/usr/local/cargo/registry,id=cargo-registry \
    --mount=type=cache,target=/usr/local/cargo/git,id=cargo-git \
    --mount=type=cache,target=/app/target,id=cargo-target-unityextractor \
    set -eu; \
    for target in x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-pc-windows-gnu; do \
        cargo build --release --locked --target "$target"; \
    done; \
    mkdir /out; \
    cp target/x86_64-unknown-linux-musl/release/rust-unityextractor /out/rust-unityextractor.x86_64-linux; \
    cp target/aarch64-unknown-linux-musl/release/rust-unityextractor /out/rust-unityextractor.aarch64-linux; \
    cp target/x86_64-pc-windows-gnu/release/rust-unityextractor.exe /out/rust-unityextractor.x86_64-windows.exe; \
    for arch in x86_64 aarch64; do \
        if readelf -d "/out/rust-unityextractor.$arch-linux" | grep -q NEEDED; then \
            echo "ERROR: rust-unityextractor.$arch-linux links a shared library" >&2; \
            readelf -d "/out/rust-unityextractor.$arch-linux" | grep NEEDED >&2; \
            exit 1; \
        fi; \
    done

FROM scratch
COPY --from=build /out/ /
