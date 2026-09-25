//! The `stdf-convert` command (also run by the Python package).

use std::ffi::OsString;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use crate::{RecordReader, json};
use clap::{Parser, ValueEnum};
use walkdir::WalkDir;

/// Convert STDF V4 and V4-2007 files to JSON Lines, one JSON object per record.
///
/// Each PATH is an STDF file (optionally .gz, .bz2 or .zip compressed) or a folder, which is
/// searched recursively for *.stdf and *.std files and their compressed forms. Output files
/// are written next to each input unless --output-dir is given.
#[derive(Parser)]
#[command(name = "stdf-convert", version)]
struct Cli {
    /// STDF files or folders to convert
    #[arg(required = true, value_name = "PATH")]
    paths: Vec<PathBuf>,

    /// Output format
    #[arg(short, long = "format", value_enum, default_value = "json")]
    format: Format,

    /// Write outputs under this folder, mirroring the layout of the input folders
    #[arg(short, long, value_name = "DIR")]
    output_dir: Option<PathBuf>,

    /// Only write these record types (e.g. --records PIR,PTR,PRR)
    #[arg(long, value_delimiter = ',', value_name = "TYPES")]
    records: Option<Vec<String>>,

    /// Replace output files that already exist
    #[arg(long)]
    overwrite: bool,

    /// Only print warnings and errors (no progress lines)
    #[arg(short, long)]
    quiet: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    /// JSON Lines (.jsonl)
    Json,
}

/// Run the command with `args` (including the program name) and return its exit code.
pub fn run<I, T>(args: I) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(e) => {
            let _ = e.print();
            return e.exit_code();
        }
    };
    let inputs = find_inputs(&cli.paths);
    if inputs.is_empty() {
        eprintln!("no STDF files found");
        return 1;
    }
    let mut failed = 0;
    for (input, rel) in &inputs {
        if let Err(e) = convert(&cli, input, rel) {
            eprintln!("FAIL {}: {e}", input.display());
            failed += 1;
        }
    }
    if failed > 0 {
        eprintln!("{failed} of {} files failed", inputs.len());
        return 1;
    }
    0
}

const COMPRESSED: [&str; 3] = [".gz", ".bz2", ".zip"];
const STDF: [&str; 2] = [".stdf", ".std"];

/// The file name without its compression and STDF extensions: `a.stdf.gz` -> `a`.
fn stem(name: &str) -> (&str, bool) {
    let lower = name.to_ascii_lowercase();
    let mut end = name.len();
    if let Some(ext) = COMPRESSED.iter().find(|e| lower.ends_with(*e)) {
        end -= ext.len();
    }
    match STDF.iter().find(|e| lower[..end].ends_with(*e)) {
        Some(ext) => (&name[..end - ext.len()], true),
        None => (&name[..end], false),
    }
}

/// Where the JSON Lines output for `input` goes by default: `a.stdf.gz` -> `a.jsonl`.
pub fn output_path(input: &Path) -> PathBuf {
    let name = input.file_name().unwrap_or_default().to_string_lossy().into_owned();
    input.with_file_name(format!("{}.jsonl", stem(&name).0))
}

/// Every STDF file to convert, with its path relative to the argument it came from.
fn find_inputs(paths: &[PathBuf]) -> Vec<(PathBuf, PathBuf)> {
    let mut inputs = Vec::new();
    for path in paths {
        if path.is_dir() {
            for entry in WalkDir::new(path).sort_by_file_name().into_iter().filter_map(|e| e.ok()) {
                let name = entry.file_name().to_string_lossy();
                if entry.file_type().is_file() && stem(&name).1 && !name.starts_with("._") {
                    let p = entry.path();
                    inputs.push((p.to_path_buf(), p.strip_prefix(path).unwrap().to_path_buf()));
                }
            }
        } else if path.is_file() {
            inputs.push((path.clone(), PathBuf::from(path.file_name().unwrap())));
        } else {
            eprintln!("skip: {} (not a file or folder)", path.display());
        }
    }
    inputs
}

fn convert(cli: &Cli, input: &Path, rel: &Path) -> Result<(), String> {
    let base = match &cli.output_dir {
        Some(dir) => dir.join(rel),
        None => input.to_path_buf(),
    };
    let out = output_path(&base);
    if out.exists() && !cli.overwrite {
        eprintln!("SKIP {} (exists; use --overwrite)", out.display());
        return Ok(());
    }
    let filter: Option<Vec<&str>> = cli.records.as_ref().map(|r| r.iter().map(String::as_str).collect());
    let records = RecordReader::open(input, filter.as_deref()).map_err(|e| e.to_string())?;
    if let Some(parent) = out.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    // write to a temporary file so a failed conversion never leaves a partial output
    let tmp = out.with_extension("jsonl.tmp");
    let file = File::create(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let written = match cli.format {
        Format::Json => json::write_json_lines(records, file).map_err(|e| e.to_string()),
    };
    let n = match written.and_then(|n| fs::rename(&tmp, &out).map(|_| n).map_err(|e| e.to_string())) {
        Ok(n) => n,
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    };
    if !cli.quiet {
        println!("OK   {}  ({n} records)", out.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::stem;

    #[test]
    fn stems() {
        assert_eq!(stem("lot1.stdf"), ("lot1", true));
        assert_eq!(stem("lot1.STD.gz"), ("lot1", true));
        assert_eq!(stem("lot1.stdf.bz2"), ("lot1", true));
        assert_eq!(stem("lot1.zip"), ("lot1", false));
        assert_eq!(stem("notes.txt"), ("notes.txt", false));
    }
}
