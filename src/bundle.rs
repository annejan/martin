//! Bundled single-binary mode (`--features bundle`): the embedded asset archive (built by
//! `build.rs`) is self-extracted to a temp dir at startup, and the baked-in show is applied by
//! pre-seeding the same `MARTIN_*` env vars martin already reads — so the whole normal load path
//! runs unchanged, and any env var the user sets still overrides the bundled default.

use std::path::PathBuf;

// Emitted by build.rs: SHOW_KIND, SHOW_SRC, SCORE_NAME, ROOT_PLY, LOGO, MORPH_COUNT.
include!(concat!(env!("OUT_DIR"), "/bundle_config.rs"));

// The lz4 archive built by build.rs: [u32 count]{ [u32 name_len][name][u32 data_len][lz4 data] }.
static ARCHIVE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/bundle.bin"));

/// Self-extract the embedded assets to a temp dir, then pre-seed the `MARTIN_*` env so the baked-in
/// show plays with no external files. Call once at the very start of `main`.
pub fn apply() {
    let dir = match extract() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("bundle: extract failed: {e}");
            return;
        }
    };

    // Only set what the user hasn't overridden — a bundled binary is still tweakable via env.
    let set = |k: &str, v: &str| {
        if std::env::var_os(k).is_none() {
            // SAFETY: self-extract runs at the very start of main(), single-threaded, pre-app.
            unsafe { std::env::set_var(k, v) };
        }
    };
    match SHOW_KIND {
        "show" => set("MARTIN_SHOW", SHOW_SRC), // unified .show — show::apply accepts inline text
        "compose" => set("MARTIN_COMPOSE", SHOW_SRC),
        _ => set("MARTIN_SEQ", SHOW_SRC),
    }
    // MARTIN_PLY's parent dir is martin's asset root → point it at any extracted file in the dir.
    let anchor = if ROOT_PLY.is_empty() {
        "_root"
    } else {
        ROOT_PLY
    };
    if let Some(p) = dir.join(anchor).to_str() {
        set("MARTIN_PLY", p);
    }
    if !SCORE_NAME.is_empty() && dir.join(SCORE_NAME).exists() {
        if let Some(p) = dir.join(SCORE_NAME).to_str() {
            set("MARTIN_SCORE", p);
        }
    }
    if !LOGO.is_empty() {
        set("MARTIN_LOGO", LOGO); // a filename relative to the asset root (the temp dir)
        set("MARTIN_LOADER", "1"); // a bundled show wants the loader screen
    }
    if !MORPH_COUNT.is_empty() {
        set("MARTIN_MORPH_COUNT", MORPH_COUNT);
    }
    // Pre-rendered music baked in → play it directly (no ~30s live synth render → no silent start).
    if !MUSIC_NAME.is_empty() && dir.join(MUSIC_NAME).exists() {
        if let Some(p) = dir.join(MUSIC_NAME).to_str() {
            set("MARTIN_MUSIC", p);
        }
    }
    println!(
        "bundle: extracted to {} — playing baked-in show",
        dir.display()
    );
}

/// Decompress every archive entry into a temp dir keyed by the archive's size, so relaunches of the
/// same binary reuse the extraction instead of rewriting it (and don't pile up per-run copies).
fn extract() -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!("martin-bundle-{}", ARCHIVE.len()));
    // a previous run already unpacked this exact archive here → reuse it.
    if dir.join(".ready").exists() {
        return Ok(dir);
    }
    std::fs::create_dir_all(&dir)?;

    let mut p = 0usize;
    let rd_u32 = |b: &[u8], p: &mut usize| -> u32 {
        let v = u32::from_le_bytes([b[*p], b[*p + 1], b[*p + 2], b[*p + 3]]);
        *p += 4;
        v
    };
    let count = rd_u32(ARCHIVE, &mut p);
    for _ in 0..count {
        let name_len = rd_u32(ARCHIVE, &mut p) as usize;
        let name = std::str::from_utf8(&ARCHIVE[p..p + name_len])
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
            .to_string();
        p += name_len;
        let data_len = rd_u32(ARCHIVE, &mut p) as usize;
        let comp = &ARCHIVE[p..p + data_len];
        p += data_len;
        let raw = lz4_flex::decompress_size_prepended(comp)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(dir.join(&name), raw)?;
    }
    std::fs::write(dir.join(".ready"), [])?; // mark complete so relaunches skip the unpack
    Ok(dir)
}
