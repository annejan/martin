// SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
// SPDX-License-Identifier: MIT
//
//! `splatgen` — a standalone CLI that writes any procedural shape to martin's sh0 binary `.ply`, so
//! you can author new abstract content without a python/numpy step or a real capture. It shares the
//! exact generator `build.rs` uses to auto-synthesize the demo splats (`build/gen_splats.rs`), so a
//! `splatgen <shape>` cloud is identical to the auto-generated one and morphs cleanly with the rest.
//!
//!   cargo run --release --bin splatgen -- list                 # the shapes it knows
//!   cargo run --release --bin splatgen -- lsystem assets/tree.ply
//!   cargo run --release --bin splatgen -- torus                # → torus.ply
//!
//! Then reference the `.ply` from a show (`splat:tree.ply`, `mesh:`/`glb:` are for real geometry).

#[path = "../../build/gen_splats.rs"]
mod splats;

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(String::as_str);
    match cmd {
        Some("list") | Some("--list") | Some("-l") => {
            for s in splats::SHAPES {
                println!("{s}");
            }
            ExitCode::SUCCESS
        }
        Some(shape) if !shape.starts_with('-') => match splats::gen_shape(shape) {
            Some((pos, rgb)) => {
                let out = args
                    .get(2)
                    .cloned()
                    .unwrap_or_else(|| format!("{shape}.ply"));
                splats::write_ply(Path::new(&out), shape, &pos, &rgb);
                println!("splatgen: wrote {out} ({} splats: {shape})", pos.len());
                ExitCode::SUCCESS
            }
            None => {
                eprintln!(
                    "splatgen: unknown shape '{shape}'. known: {}",
                    splats::SHAPES.join(", ")
                );
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("usage: splatgen <shape> [out.ply]   |   splatgen list");
            eprintln!("shapes: {}", splats::SHAPES.join(", "));
            ExitCode::FAILURE
        }
    }
}
