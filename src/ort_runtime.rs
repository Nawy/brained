use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::Context;

/// Pinned to match the ONNX Runtime version `ort` 2.0.0-rc.12 (used transitively via
/// `fastembed`) actually targets — confirmed against `ort-sys`'s own prebuilt-binary manifest
/// (`build/download/dist.txt`, which fetches Microsoft's official `ms@1.24.2` build for every
/// platform/accelerator combination).
const ONNXRUNTIME_VERSION: &str = "1.24.2";

/// `ort`'s `load-dynamic` feature falls back to a bare `"onnxruntime.dll"`/`"libonnxruntime.so"`/
/// `"libonnxruntime.dylib"` lookup (the OS's default dynamic-library search order) when
/// `ORT_DYLIB_PATH` isn't set. On Windows this was confirmed to silently resolve to
/// `C:\Windows\System32\onnxruntime.dll` — an OS-bundled, DirectML-era build (found at
/// v1.10.220126, Jan 2022) that is API-incompatible with `ort` 2.x and hangs indefinitely during
/// environment/session creation instead of erroring (confirmed via live repro: 0% CPU, no
/// network activity, `Get-Process ... | Modules` showed `C:\WINDOWS\SYSTEM32\onnxruntime.dll`
/// loaded).
///
/// To avoid depending on either the OS's bundled runtime (Windows) or the OS having any runtime
/// on the default search path at all (Linux/macOS, where there's usually nothing to accidentally
/// find), `brd` downloads Microsoft's official prebuilt dynamic library for the pinned version
/// into its own cache directory (once, then reused) and points `ort` at it explicitly via
/// `ort::init_from` before any `Session`/`TextEmbedding` is constructed.
///
/// Verified end-to-end on Windows (real repro + real fix, confirmed via a live scan + a real MCP
/// `search_knowledge` call returning correct results). The Linux (x86_64) and macOS (aarch64)
/// paths below were implemented from Microsoft's actual published release assets for this same
/// version (asset names and internal archive layout confirmed by downloading and inspecting both
/// archives directly — see the per-platform comments), but have NOT been run on real Linux/macOS
/// hardware; this environment is Windows-only. If either turns out to be wrong in practice,
/// the error will be an explicit `anyhow` failure (bad URL / missing archive entry / extraction
/// error), not a silent hang — much easier to diagnose than the original bug. Other
/// platform/arch combinations (Linux aarch64, macOS x86_64 — Microsoft doesn't publish an
/// x86_64 macOS build for this ONNX Runtime version at all) fall through to a clear error
/// explaining that `ORT_DYLIB_PATH` must be set manually.
struct Target {
    /// URL to Microsoft's official release archive for this platform/version.
    url: String,
    /// Whether the archive is a zip (Windows) or a gzipped tar (Linux/macOS).
    archive_kind: ArchiveKind,
    /// (path inside the archive, file name to extract it as in the cache dir) — extracted in
    /// order; the first entry is the main dylib and is what `ensure_onnxruntime_dylib` returns
    /// the path to.
    entries: Vec<(String, &'static str)>,
}

enum ArchiveKind {
    Zip,
    TarGz,
}

fn target() -> anyhow::Result<Target> {
    let base = format!("https://github.com/microsoft/onnxruntime/releases/download/v{ONNXRUNTIME_VERSION}");

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        let dist = format!("onnxruntime-win-x64-{ONNXRUNTIME_VERSION}");
        return Ok(Target {
            url: format!("{base}/{dist}.zip"),
            archive_kind: ArchiveKind::Zip,
            entries: vec![
                (format!("{dist}/lib/onnxruntime.dll"), "onnxruntime.dll"),
                (format!("{dist}/lib/onnxruntime_providers_shared.dll"), "onnxruntime_providers_shared.dll"),
            ],
        });
    }

    // Confirmed by downloading `onnxruntime-linux-x64-1.24.2.tgz` and inspecting its contents:
    // `lib/libonnxruntime.so` is a symlink (-> `libonnxruntime.so.1` -> `libonnxruntime.so.1.24.2`,
    // the real ~22MB file), not a regular file — a plain tar-entry-by-name extraction of
    // `libonnxruntime.so` itself would only recreate an empty symlink pointing at a name that
    // doesn't exist in our flat cache dir. Extracting the real versioned file's *contents* and
    // writing them out under the plain `libonnxruntime.so` name sidesteps needing to recreate the
    // symlink chain (dlopen only needs a real file with the right bytes at the path we hand it).
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let dist = format!("onnxruntime-linux-x64-{ONNXRUNTIME_VERSION}");
        return Ok(Target {
            url: format!("{base}/{dist}.tgz"),
            archive_kind: ArchiveKind::TarGz,
            entries: vec![
                (format!("{dist}/lib/libonnxruntime.so.{ONNXRUNTIME_VERSION}"), "libonnxruntime.so"),
                (format!("{dist}/lib/libonnxruntime_providers_shared.so"), "libonnxruntime_providers_shared.so"),
            ],
        });
    }

    // Confirmed by downloading `onnxruntime-osx-arm64-1.24.2.tgz` and inspecting its contents:
    // unlike Linux, `lib/libonnxruntime.dylib` here is a real regular file (not a symlink), so it
    // can be extracted directly under its own name. No `libonnxruntime_providers_shared.dylib`
    // ships in this archive (not needed for CPU-only use on macOS). Microsoft does not publish an
    // x86_64 macOS build of onnxruntime for this version at all — only `osx-arm64` exists.
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let dist = format!("onnxruntime-osx-arm64-{ONNXRUNTIME_VERSION}");
        return Ok(Target {
            url: format!("{base}/{dist}.tgz"),
            archive_kind: ArchiveKind::TarGz,
            entries: vec![(format!("{dist}/lib/libonnxruntime.dylib"), "libonnxruntime.dylib")],
        });
    }

    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
    )))]
    anyhow::bail!(
        "brd doesn't know a prebuilt ONNX Runtime source for this platform/architecture. \
         Install ONNX Runtime {ONNXRUNTIME_VERSION} yourself and set the ORT_DYLIB_PATH \
         environment variable to point at it before running brd."
    );
}

/// Downloads (or reuses a cached copy of) a known-compatible ONNX Runtime dynamic library into
/// `<cache_dir>/onnxruntime-<version>/`, returning the path to the main dylib. Not unit-tested
/// directly (needs network on first run) — covered by the manual end-to-end smoke test.
pub fn ensure_onnxruntime_dylib(cache_dir: &Path) -> anyhow::Result<PathBuf> {
    let target = target()?;
    let dest_dir = cache_dir.join(format!("onnxruntime-{ONNXRUNTIME_VERSION}"));
    let main_dll_name = target.entries[0].1;
    let dll_path = dest_dir.join(main_dll_name);
    if dll_path.exists() {
        return Ok(dll_path);
    }
    std::fs::create_dir_all(&dest_dir).with_context(|| format!("creating {}", dest_dir.display()))?;

    eprintln!("brd: downloading ONNX Runtime {ONNXRUNTIME_VERSION} (one-time, ~70MB)...");
    let response = ureq::get(&target.url).call().with_context(|| format!("downloading {}", target.url))?;
    let mut archive_bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .read_to_end(&mut archive_bytes)
        .context("reading ONNX Runtime archive response body")?;

    match target.archive_kind {
        ArchiveKind::Zip => extract_from_zip(&archive_bytes, &target.entries, &dest_dir)?,
        ArchiveKind::TarGz => extract_from_tar_gz(&archive_bytes, &target.entries, &dest_dir)?,
    }
    eprintln!("brd: ONNX Runtime {ONNXRUNTIME_VERSION} ready");

    Ok(dll_path)
}

fn extract_from_zip(archive_bytes: &[u8], entries: &[(String, &'static str)], dest_dir: &Path) -> anyhow::Result<()> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(archive_bytes)).context("opening ONNX Runtime zip archive")?;
    for (entry_path, out_name) in entries {
        let mut entry = archive
            .by_name(entry_path)
            .with_context(|| format!("finding {entry_path} in the ONNX Runtime archive"))?;
        let out_path = dest_dir.join(out_name);
        let mut out_file = std::fs::File::create(&out_path).with_context(|| format!("creating {}", out_path.display()))?;
        std::io::copy(&mut entry, &mut out_file).with_context(|| format!("extracting {entry_path}"))?;
    }
    Ok(())
}

fn extract_from_tar_gz(archive_bytes: &[u8], entries: &[(String, &'static str)], dest_dir: &Path) -> anyhow::Result<()> {
    let decoder = flate2::read::GzDecoder::new(std::io::Cursor::new(archive_bytes));
    let mut archive = tar::Archive::new(decoder);
    let mut remaining: std::collections::HashMap<&str, &'static str> =
        entries.iter().map(|(path, name)| (path.as_str(), *name)).collect();

    for tar_entry in archive.entries().context("reading ONNX Runtime tar archive")? {
        let mut tar_entry = tar_entry.context("reading a tar entry from the ONNX Runtime archive")?;
        let entry_path = tar_entry.path().context("reading a tar entry's path")?.to_string_lossy().into_owned();
        let Some(out_name) = remaining.remove(entry_path.as_str()) else {
            continue;
        };
        let out_path = dest_dir.join(out_name);
        let mut out_file = std::fs::File::create(&out_path).with_context(|| format!("creating {}", out_path.display()))?;
        std::io::copy(&mut tar_entry, &mut out_file).with_context(|| format!("extracting {entry_path}"))?;
    }

    anyhow::ensure!(
        remaining.is_empty(),
        "ONNX Runtime archive was missing expected entries: {}",
        remaining.keys().cloned().collect::<Vec<_>>().join(", ")
    );
    Ok(())
}
