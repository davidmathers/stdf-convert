//! The checked-in Arrow columns match the rust-stdf in the lock file.

use std::path::Path;

#[test]
fn generated_columns_are_up_to_date() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = stdf_convert_codegen::locate_rust_stdf(&workspace).unwrap();
    let want = stdf_convert_codegen::generate_from(&source).unwrap();
    let have = std::fs::read_to_string(workspace.join(stdf_convert_codegen::OUTPUT)).unwrap();
    assert!(
        have.replace("\r\n", "\n") == want,
        "{} is out of date: run `cargo run -p stdf-convert-codegen`",
        stdf_convert_codegen::OUTPUT
    );
}
