//! Writes the fuzz seed inputs to `crates/xpass-core/fuzz/seeds/<target>/`.
//!
//! `cargo run -p xpass-core --example fuzz_seeds --features fuzzing`
//!
//! The seeds are deterministic (fixed RNG seed); re-run after a format change.

use std::{fs, path::Path};

fn main() -> std::io::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz").join("seeds");
    for (target, name, data) in xpass_core::fuzzing::seed_inputs() {
        let dir = root.join(target);
        fs::create_dir_all(&dir)?;
        fs::write(dir.join(name), &data)?;
        println!("{target}/{name} ({} bytes)", data.len());
    }
    Ok(())
}
