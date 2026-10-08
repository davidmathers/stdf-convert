//! Generates stdf-convert's Arrow columns (`crates/stdf-convert/src/arrow/generated.rs`) from
//! rust-stdf's record definitions, so the columns can't drift from the parser.
//!
//! rust-stdf declares every record field with an STDF type alias (`test_num: U4`,
//! `lo_limit: Option<R4>`, `rtn_stat: KxN1`, ...) in `src/records/*.rs`. The alias names are
//! the STDF type codes, which the compiled types lose (`Bn` and `KxU1` are both `Vec<u8>`), so
//! they are read from the source. Each field becomes a column of type `col::<code>`, or
//! `col::Opt<col::<code>>` if optional; the `col` module maps each code to an Arrow type.
//!
//! The record types come from the `RECORDS` table in rust-stdf-derive, which generates the
//! `StdfRecord` enum, plus the two variants that enum adds for reserved and unknown records.
//! A record rust-stdf reads incompletely ([`OVERRIDES`]) gets its columns from stdf-convert's
//! own struct for it instead, declared with the same type aliases.
//! The generated code matches on every `StdfRecord` variant, so if rust-stdf adds one, it
//! fails to compile until the generator knows about it.

use std::collections::HashSet;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use quote::ToTokens;
use syn::ext::IdentExt;
use syn::{Expr, Fields, GenericArgument, Item, Lit, PathArguments, Type};

/// Where the generated file goes, relative to the workspace root.
pub const OUTPUT: &str = "crates/stdf-convert/src/arrow/generated.rs";

/// The stdf-convert source defining the [`OVERRIDES`] structs, relative to the workspace root.
pub const OVERRIDES_SOURCE: &str = "crates/stdf-convert/src/vur.rs";

/// Records whose columns come from a stdf-convert struct rather than rust-stdf's: (record name,
/// struct). The struct has a `fn of(&Record) -> Cow<Self>` giving a record's values. VUR: until
/// https://github.com/noonchen/rust-stdf/issues/33 is fixed.
const OVERRIDES: &[(&str, &str)] = &[("VUR", "Vur")];

/// `StdfRecord` variants after the `RECORDS` table: (variant, struct, record name as in
/// `stdf_convert::RECORD_TYPES`).
const EXTRA_RECORDS: &[(&str, &str, &str)] =
    &[("ReservedRec", "ReservedRec", "RESERVED"), ("UnknownRec", "ReservedRec", "UNKNOWN")];

/// Fields declared with a plain Rust type that doesn't say which STDF type it holds:
/// (struct, field, STDF code).
const FIELD_TYPES: &[(&str, &str, &str)] = &[("ReservedRec", "raw_data", "Bn"), ("Dn", "bit_data", "Bn")];

/// STDF codes for plain Rust types that have only one meaning.
const PRIMITIVES: &[(&str, &str)] = &[
    ("u8", "U1"),
    ("u16", "U2"),
    ("u32", "U4"),
    ("u64", "U8"),
    ("i8", "I1"),
    ("i16", "I2"),
    ("i32", "I4"),
    ("f32", "R4"),
    ("f64", "R8"),
];

/// STDF codes whose values are bytes: `bytes` in Python, hex in JSON.
const BYTE_CODES: &[&str] = &["B1", "Bn"];

/// The rust-stdf sources the columns are generated from.
pub struct Source {
    pub version: String,
    /// `src/records/*.rs` and `src/stdf_codec/primitives.rs`.
    pub files: Vec<PathBuf>,
    /// rust-stdf-derive's `src/lib.rs`, which holds the `RECORDS` table.
    pub derive: PathBuf,
    /// [`OVERRIDES_SOURCE`].
    pub overrides: PathBuf,
}

/// Find the rust-stdf and rust-stdf-derive that `workspace`'s lock file resolves to.
pub fn locate_rust_stdf(workspace: &Path) -> Result<Source, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let out = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--locked", "--manifest-path"])
        .arg(workspace.join("Cargo.toml"))
        .output()
        .map_err(|e| format!("could not run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let metadata: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: {e}"))?;
    let packages = metadata["packages"].as_array().ok_or("cargo metadata: no packages")?;
    let package = |name: &str| {
        let mut found = packages.iter().filter(|p| p["name"] == name);
        let (Some(package), None) = (found.next(), found.next()) else {
            return Err(format!("expected exactly one {name} package in the lock file"));
        };
        let manifest = package["manifest_path"].as_str().ok_or(format!("{name} has no manifest_path"))?;
        let version = package["version"].as_str().ok_or(format!("{name} has no version"))?;
        Ok((Path::new(manifest).with_file_name("src"), version.to_string()))
    };
    let (src, version) = package("rust-stdf")?;
    let (derive_src, _) = package("rust-stdf-derive")?;
    let records = src.join("records");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&records)
        .map_err(|e| format!("{}: {e}", records.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    files.sort();
    files.push(src.join("stdf_codec").join("primitives.rs"));
    Ok(Source {
        version,
        files,
        derive: derive_src.join("lib.rs"),
        overrides: workspace.join(OVERRIDES_SOURCE),
    })
}

/// Generate the module from `source`.
pub fn generate_from(source: &Source) -> Result<String, String> {
    let read = |p: &PathBuf| std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()));
    let files = source.files.iter().map(read).collect::<Result<Vec<_>, _>>()?;
    generate(&files, &read(&source.derive)?, &read(&source.overrides)?, &source.version)
}

struct Record {
    /// `PTR`, `RESERVED`, ...; for types inside records, the type (`Dn`).
    name: String,
    /// The `StdfRecord` variant.
    variant: String,
    /// The struct's path in stdf-convert: rust-stdf's, or for [`OVERRIDES`], stdf-convert's.
    rust_path: String,
    /// Whether the struct is stdf-convert's.
    overridden: bool,
    fields: Vec<Field>,
}

struct Field {
    /// The Rust field, as written (`r#type` stays raw).
    ident: String,
    /// The column name, as rust-stdf's serde names the field.
    name: String,
    code: String,
    optional: bool,
}

/// A `V1` variant: a GDR value's type.
struct Value {
    /// `U1`, `Cn`, `B0`, ...
    name: String,
    /// The STDF type of the variant's value; `None` for `B0` (padding) and `Invalid`.
    code: Option<String>,
}

/// Generate the module from the text of rust-stdf `version`'s record and type definitions,
/// of rust-stdf-derive's `src/lib.rs`, and of [`OVERRIDES_SOURCE`].
pub fn generate(files: &[String], derive: &str, overrides: &str, version: &str) -> Result<String, String> {
    let mut items = Vec::new();
    for text in files {
        items.extend(syn::parse_file(text).map_err(|e| format!("rust-stdf source: {e}"))?.items);
    }
    items.extend(syn::parse_file(overrides).map_err(|e| format!("{OVERRIDES_SOURCE}: {e}"))?.items);

    // Type names fields can be declared with: aliases, enums and structs other than records.
    let mut codes = HashSet::new();
    for item in &items {
        match item {
            Item::Type(t) => {
                codes.insert(t.ident.to_string());
            }
            Item::Enum(e) => {
                codes.insert(e.ident.to_string());
            }
            Item::Struct(s) => {
                codes.insert(s.ident.to_string());
            }
            _ => {}
        }
    }
    let find_struct = |name: &str| {
        items
            .iter()
            .find_map(|i| match i {
                Item::Struct(s) if s.ident == name => Some(s),
                _ => None,
            })
            .ok_or_else(|| format!("rust-stdf has no struct {name}"))
    };
    let columns_of = |rust_type: &str| -> Result<Vec<Field>, String> {
        let s = find_struct(rust_type)?;
        let Fields::Named(named) = &s.fields else {
            return Err(format!("{rust_type} should have named fields"));
        };
        let uppercase = s
            .attrs
            .iter()
            .any(|a| a.to_token_stream().to_string().replace(' ', "").contains("rename_all=\"UPPERCASE\""));
        let mut fields = Vec::new();
        for f in &named.named {
            let ident = f.ident.as_ref().expect("named field");
            let field_name = ident.unraw().to_string();
            let (code, optional) = field_code(rust_type, &field_name, &f.ty, &codes)
                .ok_or_else(|| format!("{rust_type}.{field_name}: no STDF type for this field"))?;
            let name = if uppercase { field_name.to_uppercase() } else { field_name };
            fields.push(Field { ident: ident.to_string(), name, code, optional });
        }
        Ok(fields)
    };

    let mut records = Vec::new();
    let table = record_table(derive)?;
    let extra = EXTRA_RECORDS.iter().map(|(v, t, n)| (v.to_string(), t.to_string(), n.to_string()));
    for (variant, rust_type, name) in table.into_iter().map(|n| (n.clone(), n.clone(), n)).chain(extra) {
        let (rust_type, rust_path, overridden) = match OVERRIDES.iter().find(|(n, _)| *n == name) {
            Some((_, own)) => (own.to_string(), format!("crate::{own}"), true),
            None => (rust_type.clone(), format!("rust_stdf::{rust_type}"), false),
        };
        let fields = columns_of(&rust_type)?;
        records.push(Record { name, variant, rust_path, overridden, fields });
    }
    if let Some((name, _)) = OVERRIDES.iter().find(|(n, _)| !records.iter().any(|r| r.name == *n)) {
        return Err(format!("override for {name}, which is not a rust-stdf record"));
    }

    let mut inner = Vec::new();
    // GDR values (`V1`): the variant, and a value column for each variant that holds one.
    let v1 = items
        .iter()
        .find_map(|i| match i {
            Item::Enum(e) if e.ident == "V1" => Some(e),
            _ => None,
        })
        .ok_or("rust-stdf has no enum V1")?;
    let mut values = Vec::new();
    for variant in &v1.variants {
        let name = variant.ident.to_string();
        let code = match &variant.fields {
            Fields::Unit => None,
            fields => Some(
                single_type_name(fields)
                    .filter(|t| codes.contains(t))
                    .ok_or_else(|| format!("V1::{name} should hold one STDF type"))?,
            ),
        };
        values.push(Value { name, code });
    }

    // Struct types used inside records or GDR values (`Dn`) get columns of their own too.
    let used =
        records.iter().flat_map(|r| &r.fields).map(|f| &f.code).chain(values.iter().flat_map(|v| &v.code));
    for code in used {
        if !inner.iter().any(|r: &Record| &r.name == code) && find_struct(code).is_ok() {
            let fields = columns_of(code)?;
            inner.push(Record {
                name: code.clone(),
                variant: String::new(),
                rust_path: format!("rust_stdf::{code}"),
                overridden: false,
                fields,
            });
        }
    }

    let mut byte_fields: Vec<String> = Vec::new();
    for field in records.iter().chain(&inner).flat_map(|r| &r.fields) {
        if BYTE_CODES.contains(&field.code.as_str()) && !byte_fields.contains(&field.name) {
            byte_fields.push(field.name.clone());
        }
    }
    // `V1::Bn(Bn)` holds bytes, named by its variant.
    for v in &values {
        if v.code.as_deref().is_some_and(|c| BYTE_CODES.contains(&c)) {
            byte_fields.push(v.name.clone());
        }
    }

    Ok(render(&records, &inner, &values, &byte_fields, version))
}

/// The record names in rust-stdf-derive's `RECORDS` table: `[("FAR", 0, 10), ...]`.
fn record_table(derive: &str) -> Result<Vec<String>, String> {
    let file = syn::parse_file(derive).map_err(|e| format!("rust-stdf-derive: {e}"))?;
    let table = file
        .items
        .iter()
        .find_map(|i| match i {
            Item::Const(c) if c.ident == "RECORDS" => Some(&*c.expr),
            _ => None,
        })
        .ok_or("rust-stdf-derive has no RECORDS table")?;
    let Expr::Reference(r) = table else { return Err("RECORDS should be &[...]".into()) };
    let Expr::Array(array) = &*r.expr else { return Err("RECORDS should be &[...]".into()) };
    array
        .elems
        .iter()
        .map(|e| match e {
            Expr::Tuple(t) => match t.elems.first() {
                Some(Expr::Lit(l)) => match &l.lit {
                    Lit::Str(s) => Ok(s.value()),
                    _ => Err("RECORDS entries should start with a name".to_string()),
                },
                _ => Err("RECORDS entries should start with a name".to_string()),
            },
            _ => Err("RECORDS entries should be tuples".to_string()),
        })
        .collect()
}

/// The type name held by a one-field tuple variant, e.g. `PTR` in `PTR(PTR)`.
fn single_type_name(fields: &Fields) -> Option<String> {
    let Fields::Unnamed(u) = fields else { return None };
    if u.unnamed.len() != 1 {
        return None;
    }
    path_ident(&u.unnamed[0].ty)
}

fn path_ident(ty: &Type) -> Option<String> {
    let Type::Path(p) = ty else { return None };
    if p.qself.is_some() || p.path.segments.len() != 1 {
        return None;
    }
    let segment = &p.path.segments[0];
    matches!(segment.arguments, PathArguments::None).then(|| segment.ident.to_string())
}

/// The STDF code of a field's type, and whether it is optional.
fn field_code(rust_type: &str, field: &str, ty: &Type, codes: &HashSet<String>) -> Option<(String, bool)> {
    if let Type::Path(p) = ty
        && p.path.segments.len() == 1
        && p.path.segments[0].ident == "Option"
        && let PathArguments::AngleBracketed(args) = &p.path.segments[0].arguments
        && let [GenericArgument::Type(inner)] = args.args.iter().collect::<Vec<_>>()[..]
    {
        return field_code(rust_type, field, inner, codes)
            .filter(|(_, optional)| !optional)
            .map(|(code, _)| (code, true));
    }
    if let Some((_, _, code)) = FIELD_TYPES.iter().find(|(t, f, _)| *t == rust_type && *f == field) {
        return Some((code.to_string(), false));
    }
    let name = path_ident(ty)?;
    if codes.contains(&name) {
        return Some((name, false));
    }
    PRIMITIVES.iter().find(|(rust, _)| *rust == name).map(|(_, code)| (code.to_string(), false))
}

fn render_columns(w: &mut String, r: &Record, what: &str) {
    let _ = writeln!(w, "/// Columns of {what} (`{}`).", r.rust_path);
    let _ = writeln!(w, "#[derive(Default)]");
    let _ = writeln!(w, "pub(crate) struct {}Columns {{", r.name);
    for f in &r.fields {
        let _ = writeln!(w, "    {}: {},", f.ident, column_type(f));
    }
    let _ = writeln!(w, "}}\n");
    let _ = writeln!(w, "impl {}Columns {{", r.name);
    let _ = writeln!(w, "    pub(crate) fn fields() -> Vec<Field> {{");
    let _ = writeln!(w, "        vec![");
    for f in &r.fields {
        let _ = writeln!(w, "            <{}>::field(\"{}\"),", column_type(f), f.name);
    }
    let _ = writeln!(w, "        ]");
    let _ = writeln!(w, "    }}\n");
    let _ = writeln!(w, "    pub(crate) fn append(&mut self, r: &{}) {{", r.rust_path);
    if r.fields.is_empty() {
        let _ = writeln!(w, "        let _ = r;");
    }
    for f in &r.fields {
        let _ = writeln!(w, "        self.{0}.append(&r.{0});", f.ident);
    }
    let _ = writeln!(w, "    }}\n");
    let _ = writeln!(w, "    pub(crate) fn finish(&mut self) -> Vec<ArrayRef> {{");
    let _ = writeln!(w, "        vec![");
    for f in &r.fields {
        let _ = writeln!(w, "            self.{}.finish(),", f.ident);
    }
    let _ = writeln!(w, "        ]");
    let _ = writeln!(w, "    }}");
    let _ = writeln!(w, "}}\n");
}

fn column_type(field: &Field) -> String {
    if field.optional { format!("col::Opt<col::{}>", field.code) } else { format!("col::{}", field.code) }
}

fn render(
    records: &[Record],
    inner: &[Record],
    values: &[Value],
    byte_fields: &[String],
    version: &str,
) -> String {
    let mut out = String::new();
    let w = &mut out;
    let _ = writeln!(
        w,
        "// @generated by stdf-convert-codegen from rust-stdf {version} (src/records, src/stdf_codec),\n\
         // rust-stdf-derive (RECORDS) and {OVERRIDES_SOURCE}.\n\
         // Do not edit: run `cargo run -p stdf-convert-codegen` to regenerate.\n\
         \n\
         #![allow(clippy::upper_case_acronyms)]\n\
         \n\
         use arrow_array::ArrayRef;\n\
         use arrow_schema::Field;\n\
         use rust_stdf::{{StdfRecord, V1}};\n\
         \n\
         use super::col::{{self, Column}};\n\
         \n\
         /// The rust-stdf version the columns were generated from.\n\
         pub const RUST_STDF_VERSION: &str = \"{version}\";\n"
    );

    let _ = writeln!(w, "/// Every record type name, in `StdfRecord` order.");
    let _ = writeln!(w, "pub const RECORD_TYPES: &[&str] = &[");
    for r in records {
        let _ = writeln!(w, "    \"{}\",", r.name);
    }
    let _ = writeln!(w, "];\n");

    let _ = writeln!(
        w,
        "/// Fields whose values are bytes (STDF `B1` and `Bn`), including `Dn` bit data and the"
    );
    let _ = writeln!(w, "/// `Bn` values inside `GDR.GEN_DATA`.");
    let _ = writeln!(w, "pub const BYTE_FIELDS: &[&str] = &[");
    for f in byte_fields {
        let _ = writeln!(w, "    \"{f}\",");
    }
    let _ = writeln!(w, "];\n");

    for r in records {
        render_columns(w, r, &format!("`{}` records", r.name));
    }
    for r in inner {
        render_columns(w, r, &format!("`{}` values", r.name));
    }

    let _ = writeln!(w, "/// The position of `record`'s type in [`RECORD_TYPES`].");
    let _ = writeln!(w, "pub(crate) fn record_index(record: &StdfRecord) -> usize {{");
    let _ = writeln!(w, "    match record {{");
    for (i, r) in records.iter().enumerate() {
        let _ = writeln!(w, "        StdfRecord::{}(_) => {i},", r.variant);
    }
    let _ = writeln!(w, "    }}");
    let _ = writeln!(w, "}}\n");

    let _ =
        writeln!(w, "/// Columns of the values in `GDR.GEN_DATA` (`rust_stdf::V1`): the value's type, and a");
    let _ = writeln!(w, "/// column for each type, set only in rows of that type.");
    let _ = writeln!(w, "#[derive(Default)]");
    let _ = writeln!(w, "pub(crate) struct V1Columns {{");
    let _ = writeln!(w, "    value_type: col::Name,");
    for v in values {
        if let Some(code) = &v.code {
            let _ = writeln!(w, "    {}: col::Opt<col::{code}>,", v.name.to_lowercase());
        }
    }
    let _ = writeln!(w, "}}\n");
    let _ = writeln!(w, "impl V1Columns {{");
    let _ = writeln!(w, "    pub(crate) fn fields() -> Vec<Field> {{");
    let _ = writeln!(w, "        vec![");
    let _ = writeln!(w, "            col::Name::field(\"TYPE\"),");
    for v in values {
        if let Some(code) = &v.code {
            let _ = writeln!(w, "            <col::Opt<col::{code}>>::field(\"{}\"),", v.name);
        }
    }
    let _ = writeln!(w, "        ]");
    let _ = writeln!(w, "    }}\n");
    let _ = writeln!(w, "    pub(crate) fn append(&mut self, value: &V1) {{");
    let _ = writeln!(w, "        self.value_type.append(match value {{");
    for v in values {
        let pattern = if v.code.is_some() { format!("V1::{}(_)", v.name) } else { format!("V1::{}", v.name) };
        let _ = writeln!(w, "            {pattern} => \"{}\",", v.name);
    }
    let _ = writeln!(w, "        }});");
    for v in values {
        if v.code.is_some() {
            let _ = writeln!(
                w,
                "        self.{}.append_option(if let V1::{}(v) = value {{ Some(v) }} else {{ None }});",
                v.name.to_lowercase(),
                v.name
            );
        }
    }
    let _ = writeln!(w, "    }}\n");
    let _ = writeln!(w, "    pub(crate) fn finish(&mut self) -> Vec<ArrayRef> {{");
    let _ = writeln!(w, "        vec![");
    let _ = writeln!(w, "            self.value_type.finish(),");
    for v in values {
        if v.code.is_some() {
            let _ = writeln!(w, "            self.{}.finish(),", v.name.to_lowercase());
        }
    }
    let _ = writeln!(w, "        ]");
    let _ = writeln!(w, "    }}");
    let _ = writeln!(w, "}}\n");

    let _ = writeln!(w, "/// The columns of one record type.");
    let _ = writeln!(w, "pub(crate) enum Columns {{");
    for r in records {
        let _ = writeln!(w, "    {0}(Box<{0}Columns>),", r.name);
    }
    let _ = writeln!(w, "}}\n");
    let _ = writeln!(w, "impl Columns {{");
    let _ = writeln!(w, "    pub(crate) fn new(record_type: &str) -> Option<Self> {{");
    let _ = writeln!(w, "        Some(match record_type {{");
    for r in records {
        let _ = writeln!(w, "            \"{0}\" => Self::{0}(Box::default()),", r.name);
    }
    let _ = writeln!(w, "            _ => return None,");
    let _ = writeln!(w, "        }})");
    let _ = writeln!(w, "    }}\n");
    let _ = writeln!(w, "    /// The record's own fields, without the header columns.");
    let _ = writeln!(w, "    pub(crate) fn fields(record_type: &str) -> Option<Vec<Field>> {{");
    let _ = writeln!(w, "        Some(match record_type {{");
    for r in records {
        let _ = writeln!(w, "            \"{0}\" => {0}Columns::fields(),", r.name);
    }
    let _ = writeln!(w, "            _ => return None,");
    let _ = writeln!(w, "        }})");
    let _ = writeln!(w, "    }}\n");
    let _ = writeln!(w, "    /// Append `record`, which must be of this record type.");
    let _ = writeln!(w, "    pub(crate) fn append(&mut self, record: &crate::Record) {{");
    let _ = writeln!(w, "        match (self, &record.data) {{");
    for r in records {
        let (pattern, value) =
            if r.overridden { ("_", format!("&{}::of(record)", r.rust_path)) } else { ("r", "r".into()) };
        let _ = writeln!(
            w,
            "            (Self::{}(c), StdfRecord::{}({pattern})) => c.append({value}),",
            r.name, r.variant
        );
    }
    let _ =
        writeln!(w, "            _ => panic!(\"record appended to the columns of another record type\"),");
    let _ = writeln!(w, "        }}");
    let _ = writeln!(w, "    }}\n");
    let _ = writeln!(w, "    /// Take the columns built so far, leaving them empty.");
    let _ = writeln!(w, "    pub(crate) fn finish(&mut self) -> Vec<ArrayRef> {{");
    let _ = writeln!(w, "        match self {{");
    for r in records {
        let _ = writeln!(w, "            Self::{}(c) => c.finish(),", r.name);
    }
    let _ = writeln!(w, "        }}");
    let _ = writeln!(w, "    }}");
    let _ = writeln!(w, "}}");
    out
}
