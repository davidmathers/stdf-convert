//! `cargo run -p stdf-convert-codegen`: regenerate stdf-convert's Arrow columns.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let result = stdf_convert_codegen::locate_rust_stdf(&workspace)
        .and_then(|source| stdf_convert_codegen::generate_from(&source))
        .and_then(|code| {
            let path = workspace.join(stdf_convert_codegen::OUTPUT);
            std::fs::write(&path, code).map_err(|e| format!("{}: {e}", path.display()))
        });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("stdf-convert-codegen: {e}");
            ExitCode::FAILURE
        }
    }
}
