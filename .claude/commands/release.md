Perform a release of rust-unityextractor.

Optional override: $ARGUMENTS (format: vX.Y.Z). If provided, use that version.

## Version determination

1. Find the last release tag (`git tag --sort=-v:refname | head -1`).
2. Examine commits since that tag to classify the release type:
   - **Patch**: only bug fixes, performance, dependency bumps, build changes, docs.
   - **Minor**: new features (`feat:`), new options, user-visible behavior changes.
   - **Major**: breaking CLI changes (removed or renamed options, a different output layout).
3. Bump the version accordingly. If **major**, stop and confirm before proceeding.

## Pre-release checks

Run in sequence, stop and report on any failure:

```sh
cargo fmt --all
cargo clippy --fix --allow-dirty --message-format=short
cargo test --release -- --quiet
./build.sh
make deb
```

`./build.sh` cross-compiles every release target into dist/ and fails if a Linux binary links a shared library. `make deb` packages the two Linux binaries; check them with `dpkg-deb -I` and `-c`. Smoke-test what it produced against a real package, each in its own scratch directory, and compare the extracted trees with the native build: `qemu-aarch64-static` runs the aarch64 binary, `WINEDEBUG=-all wine` the Windows one.

## Steps

1. Bump `version` in `Cargo.toml`, then `cargo check` so Cargo.lock follows.

2. Run the pre-release checks above.

3. Draft a changelog from `git log --oneline <last-tag>..HEAD`.

   **Rules:**
   - Group under: `New features:`, `Bug fixes:`, `Build:`, `Performance:` — omit empty sections.
   - Describe user-visible behavior, not implementation details. Name the input and what the user now sees: an example path or command reads better than a rule.
   - Merge related commits for the same change into one bullet.
   - No git hashes, no raw commit subjects, no co-author lines.

   Tag annotation format:
   ```
   vX.Y.Z

   Bug fixes:
   - what was fixed

   Build:
   - what changed
   ```

4. Stage, commit, push:

```sh
git add Cargo.toml Cargo.lock
git commit -F - <<'EOF'
release: vX.Y.Z
EOF
git push
```

5. Wait for CI to be green before tagging (`./watch-ci.sh`). It pins the workflow and the full commit SHA and passes `--exit-status`; never select a run with `--branch ... --limit 1`, which reads whatever ran last on the branch. Never tag on red or pending CI.

6. Create the signed, annotated tag and push it; the Build and Release workflow builds the binaries and leaves a **draft** release:

```sh
git tag -s vX.Y.Z -F - <<'EOF'
vX.Y.Z

<changelog>
EOF
git push origin vX.Y.Z
```

7. Wait for the tag's run (`./watch-ci.sh vX.Y.Z`).

8. Sign the assets and publish the release:

```sh
./sign-release.sh vX.Y.Z
```

   Signs every asset with `git config user.signingkey`, uploads the `.asc` files, then flips the release public as its last act. Assets can no longer be added once a release is published, so signing never happens after. `-n` signs and verifies without uploading or publishing.

9. Publish the Debian packages to apt.ticpu.net, from `~/GIT/apt-ticpu-net`:

```sh
cd ~/GIT/apt-ticpu-net && ./ingest.sh rust-unityextractor vX.Y.Z
```

   It downloads the release's `.deb` assets, verifies their signatures against the archive key, checks each package's version against the tag, includes them in every suite `projects.yaml` lists for this project, and publishes. `-n` inspects without touching the archive.

10. Report the tag and changelog.

## Important

- The tag is IMMUTABLE once pushed: never retag. Wrong? Make a new patch release.
- Release assets: `rust-unityextractor-vX.Y.Z-{x86_64-linux,aarch64-linux,x86_64-windows.exe}`, `rust-unityextractor_X.Y.Z_{amd64,arm64}.deb`, `SHA256SUMS`, and one `.asc` per asset. The binary names predate this process; keep them so download links keep their pattern.
- The deb version comes from the tag when HEAD carries one, else `X.Y.Z+<commits>-<sha>`. Before pushing the tag, run `make deb` on the tagged commit and check the file name reads `X.Y.Z`, not the `+` form: apt-ticpu-net refuses a version that does not match the tag.
- Cargo.lock is tracked: stage it with every change to it, not only on release commits.
