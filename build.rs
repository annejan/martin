//! Bundling pipeline (only runs for `--features bundle`): read `bundle.toml`, auto-collect the
//! `.ply`/PNG assets the baked-in show references, lz4-compress them into one archive embedded in
//! the binary, and emit the show config as Rust consts. At runtime `src/bundle.rs` self-extracts
//! the archive to a temp dir and plays the show. A normal build does nothing here.

use std::io::Write;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=bundle.toml");
    println!("cargo:rerun-if-changed=build.rs");
    // Only do the work for a bundled build (cargo sets CARGO_FEATURE_<NAME> per enabled feature).
    if std::env::var("CARGO_FEATURE_BUNDLE").is_err() {
        return;
    }

    let manifest = std::env::var("MARTIN_BUNDLE").unwrap_or_else(|_| "bundle.toml".to_string());
    println!("cargo:rerun-if-changed={manifest}");
    let toml = std::fs::read_to_string(&manifest)
        .unwrap_or_else(|e| panic!("bundle: cannot read {manifest}: {e}"));
    let cfg = parse_kv(&toml);

    let get = |k: &str| {
        cfg.iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.as_str())
    };
    let asset_dir = PathBuf::from(get("asset_dir").unwrap_or("assets"));
    let (kind, show_spec) = match (get("seq"), get("compose")) {
        (Some(s), _) => ("seq", s),
        (_, Some(c)) => ("compose", c),
        _ => panic!("bundle: bundle.toml needs a `seq = …` or `compose = …`"),
    };
    // The show spec is a file path (read its content) or an inline string — same rule martin uses.
    let show_src = read_or_inline(show_spec);

    // Auto-collect: every `splat:`/`image:`/`mesh:` filename the show references, + the logo.
    let mut names = referenced_assets(&show_src);
    let logo = get("logo").unwrap_or("").to_string();
    if !logo.is_empty() && !names.contains(&logo) {
        names.push(logo.clone());
    }

    // The archive: per entry [u32 name_len][name][u32 data_len][lz4 data]; prefixed by [u32 count].
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for name in &names {
        let path = asset_dir.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        // a `.mtl` sibling is optional — an .obj without one just renders with the flat fallback.
        if name.ends_with(".mtl") && !path.exists() {
            continue;
        }
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("bundle: missing asset {}: {e}", path.display()));
        files.push((name.clone(), bytes));
    }
    // The score: bake the named file (else the default assets/score.txt) so the bundle is self-contained.
    let score_path = get("score")
        .map(PathBuf::from)
        .unwrap_or_else(|| asset_dir.join("score.txt"));
    let score_name = "score.txt".to_string();
    if let Ok(bytes) = std::fs::read(&score_path) {
        println!("cargo:rerun-if-changed={}", score_path.display());
        files.push((score_name.clone(), bytes));
    }

    let root_ply = names
        .iter()
        .find(|n| n.ends_with(".ply"))
        .cloned()
        .unwrap_or_default();

    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    write_archive(&out.join("bundle.bin"), &files);

    let mut raw_total = 0usize;
    let mut comp_total = 0usize;
    let mut code = String::new();
    code.push_str(&format!("pub const SHOW_KIND: &str = {kind:?};\n"));
    code.push_str(&format!("pub const SHOW_SRC: &str = {show_src:?};\n"));
    code.push_str(&format!("pub const SCORE_NAME: &str = {score_name:?};\n"));
    code.push_str(&format!("pub const ROOT_PLY: &str = {root_ply:?};\n"));
    code.push_str(&format!("pub const LOGO: &str = {logo:?};\n"));
    code.push_str(&format!(
        "pub const MORPH_COUNT: &str = {:?};\n",
        get("morph_count").unwrap_or("")
    ));
    std::fs::write(out.join("bundle_config.rs"), code).expect("write bundle_config.rs");

    for (_, b) in &files {
        raw_total += b.len();
        comp_total += lz4_flex::compress_prepend_size(b).len();
    }
    println!(
        "cargo:warning=bundle: {} assets, {} KiB raw -> {} KiB compressed (show: {kind})",
        files.len(),
        raw_total / 1024,
        comp_total / 1024
    );
}

/// Minimal `key = value` parser (`#` comments, optional quotes) — no toml dependency needed.
fn parse_kv(s: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in s.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let v = v.trim().trim_matches('"').trim_matches('\'');
        out.push((k.trim().to_string(), v.to_string()));
    }
    out
}

/// A file path → its contents, else the spec used verbatim as an inline string.
fn read_or_inline(spec: &str) -> String {
    std::fs::read_to_string(spec).unwrap_or_else(|_| spec.to_string())
}

/// Every asset filename a show spec references — `splat:`/`image:`/`mesh:`/`glb:`/`gltf:`/`model:`
/// (the same token grammar martin parses), plus the sibling `.mtl` of any `.obj` (Wavefront
/// references its material by name from inside the file, so it must ship alongside).
fn referenced_assets(spec: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut push = |n: &str| {
        let n = n.trim();
        if n.is_empty() || names.contains(&n.to_string()) {
            return;
        }
        names.push(n.to_string());
        if let Some(stem) = n.strip_suffix(".obj") {
            names.push(format!("{stem}.mtl")); // ship the material beside the .obj
        }
    };
    for line in spec.split([';', '\n']) {
        let line = line.split('#').next().unwrap_or("");
        for tok in line.split_whitespace() {
            if let Some(p) = tok.strip_prefix("splat:") {
                p.split('+').for_each(&mut push);
            } else if let Some(p) = tok.strip_prefix("image:") {
                push(p);
            } else if let Some(p) = tok.strip_prefix("mesh:") {
                push(p);
            } else if let Some(p) = tok.strip_prefix("glb:") {
                push(p);
            } else if let Some(p) = tok.strip_prefix("gltf:") {
                push(p);
            } else if let Some(p) = tok.strip_prefix("model:") {
                push(p);
            }
        }
    }
    names
}

fn write_archive(path: &Path, files: &[(String, Vec<u8>)]) {
    let mut buf = Vec::new();
    buf.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for (name, bytes) in files {
        let comp = lz4_flex::compress_prepend_size(bytes);
        buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
        buf.extend_from_slice(name.as_bytes());
        buf.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        buf.extend_from_slice(&comp);
    }
    let mut f = std::fs::File::create(path).expect("write bundle.bin");
    f.write_all(&buf).expect("write bundle.bin");
}
