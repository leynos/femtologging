//! Checks that Rust test-support types and fields carry meaningful Rustdoc.
//!
//! Clippy's private-item documentation lint intentionally skips code compiled
//! in test mode. This source-level gate covers the helper types that support
//! unit and integration tests, while leaving individual test functions free to
//! describe their scenarios through their names and assertions. UI fixtures are
//! excluded because their source is intentionally minimal compiler input.

use camino::{Utf8Path, Utf8PathBuf};
use cap_std::{ambient_authority, fs_utf8::Dir};
use syn::{Attribute, File, ItemMod, visit::Visit};

#[path = "test_support_docs/test_modules.rs"]
mod test_modules;

use test_modules::{has_test_cfg, test_support_paths};

#[test]
fn test_support_types_have_documentation() -> Result<(), String> {
    let parsed_sources = parse_rust_sources(rust_sources()?)?;
    let test_support_paths = test_support_paths(&parsed_sources);

    let mut checker = DocumentationChecker::default();
    for (path, syntax) in parsed_sources {
        checker.file_is_test_support = test_support_paths.contains(&path);
        checker.path = path.to_string();
        checker.visit_file(&syntax);
    }

    assert!(
        checker.errors.is_empty(),
        "undocumented test-support items:\n{}",
        checker.errors.join("\n")
    );
    Ok(())
}

#[test]
fn parse_failure_reports_the_source_path() {
    let error = parse_rust_sources(vec![(Utf8PathBuf::from("src/broken.rs"), "fn".to_owned())])
        .err()
        .expect("invalid Rust should fail to parse");

    assert!(error.contains("src/broken.rs"), "{error}");
}

#[test]
fn checker_rejects_undocumented_test_struct_and_field() {
    let syntax: File = syn::parse_str(
        r#"
            struct Missing {
                field: usize,
            }
        "#,
    )
    .expect("fixture should parse");
    let mut checker = DocumentationChecker {
        file_is_test_support: true,
        path: "<fixture>".to_owned(),
        ..DocumentationChecker::default()
    };
    checker.visit_file(&syntax);

    assert_eq!(
        checker.errors,
        [
            "<fixture>: undocumented struct `Missing`".to_owned(),
            "<fixture>: undocumented field `Missing::field`".to_owned(),
        ]
    );
}

#[test]
fn checker_rejects_undocumented_enum_and_named_variant_field() {
    let syntax: File = syn::parse_str(
        r#"
            enum Missing {
                Variant {
                    field: usize,
                },
            }
        "#,
    )
    .expect("fixture should parse");
    let mut checker = DocumentationChecker {
        file_is_test_support: true,
        path: "<fixture>".to_owned(),
        ..DocumentationChecker::default()
    };
    checker.visit_file(&syntax);

    assert_eq!(
        checker.errors,
        [
            "<fixture>: undocumented enum `Missing`".to_owned(),
            "<fixture>: undocumented field `Missing::Variant::field`".to_owned(),
        ]
    );
}

#[test]
fn external_cfg_test_modules_are_test_support() {
    let sources = [
        (
            Utf8PathBuf::from("src/lib.rs"),
            syn::parse_str("#[cfg(test)] mod tests;").expect("root fixture should parse"),
        ),
        (
            Utf8PathBuf::from("src/tests.rs"),
            syn::parse_str("struct Missing;").expect("module fixture should parse"),
        ),
        (
            Utf8PathBuf::from("src/log_compat.rs"),
            syn::parse_str("#[cfg(test)] #[path = \"support/custom.rs\"] mod tests;")
                .expect("path fixture should parse"),
        ),
        (
            Utf8PathBuf::from("src/support/custom.rs"),
            syn::parse_str("struct MissingPath;").expect("path module should parse"),
        ),
    ];

    let paths = test_support_paths(&sources);

    let mut checker = DocumentationChecker::default();
    for (path, syntax) in sources.iter().filter(|(path, _)| paths.contains(path)) {
        checker.file_is_test_support = true;
        checker.path = path.to_string();
        checker.visit_file(syntax);
    }
    assert_eq!(
        checker.errors,
        [
            "src/tests.rs: undocumented struct `Missing`".to_owned(),
            "src/support/custom.rs: undocumented struct `MissingPath`".to_owned(),
        ]
    );
}

/// AST visitor that records missing Rustdoc in test-support item scopes.
#[derive(Default)]
struct DocumentationChecker {
    /// Current source path used to make each finding actionable.
    path: String,
    /// Whether the visited file is a test-support file.
    file_is_test_support: bool,
    /// Whether traversal is currently inside a `#[cfg(test)]` module.
    in_test_module: bool,
    /// Missing item and field documentation found during traversal.
    errors: Vec<String>,
}

impl DocumentationChecker {
    fn checks_current_scope(&self) -> bool {
        self.file_is_test_support || self.in_test_module
    }

    fn is_test_scoped(&self, attrs: &[Attribute]) -> bool {
        self.checks_current_scope() || has_test_cfg(attrs)
    }

    fn check_item(&mut self, attrs: &[Attribute], kind: &str, name: &str) {
        if !self.is_test_scoped(attrs) || has_doc(attrs) {
            return;
        }
        self.errors
            .push(format!("{}: undocumented {kind} `{name}`", self.path));
    }

    fn check_named_fields(&mut self, item: &syn::ItemStruct) {
        let syn::Fields::Named(fields) = &item.fields else {
            return;
        };
        for field in fields.named.iter().filter(|field| !has_doc(&field.attrs)) {
            if let Some(name) = &field.ident {
                self.errors.push(format!(
                    "{}: undocumented field `{}::{name}`",
                    self.path, item.ident
                ));
            }
        }
    }

    fn check_named_variant_fields(&mut self, item: &syn::ItemEnum) {
        for variant in &item.variants {
            let syn::Fields::Named(fields) = &variant.fields else {
                continue;
            };
            for field in fields.named.iter().filter(|field| !has_doc(&field.attrs)) {
                if let Some(name) = &field.ident {
                    self.errors.push(format!(
                        "{}: undocumented field `{}::{}::{name}`",
                        self.path, item.ident, variant.ident
                    ));
                }
            }
        }
    }
}

impl<'ast> Visit<'ast> for DocumentationChecker {
    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        let was_in_test_module = self.in_test_module;
        if item.content.is_some() && has_test_cfg(&item.attrs) {
            self.in_test_module = true;
        }
        syn::visit::visit_item_mod(self, item);
        self.in_test_module = was_in_test_module;
    }

    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        self.check_item(&item.attrs, "struct", &item.ident.to_string());
        if self.is_test_scoped(&item.attrs) {
            self.check_named_fields(item);
        }
        syn::visit::visit_item_struct(self, item);
    }

    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        self.check_item(&item.attrs, "enum", &item.ident.to_string());
        if self.is_test_scoped(&item.attrs) {
            self.check_named_variant_fields(item);
        }
        syn::visit::visit_item_enum(self, item);
    }

    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        self.check_item(&item.attrs, "type alias", &item.ident.to_string());
        syn::visit::visit_item_type(self, item);
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        self.check_item(&item.attrs, "trait", &item.ident.to_string());
        syn::visit::visit_item_trait(self, item);
    }
}

fn has_doc(attrs: &[Attribute]) -> bool {
    attrs
        .iter()
        .any(|attribute| attribute.path().is_ident("doc"))
}

fn rust_sources() -> Result<Vec<(Utf8PathBuf, String)>, String> {
    let root_path = Utf8Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = Dir::open_ambient_dir(root_path, ambient_authority())
        .map_err(|error| format!("cannot open crate root `{root_path}`: {error}"))?;
    let mut paths = Vec::new();
    for root_name in ["src", "tests"] {
        let directory = root
            .open_dir(root_name)
            .map_err(|error| format!("cannot open source directory `{root_name}`: {error}"))?;
        collect_rust_files(&directory, Utf8Path::new(root_name), &mut paths)?;
    }
    paths.sort_by(|(left, _), (right, _)| left.cmp(right));
    Ok(paths)
}

fn parse_rust_sources(
    sources: Vec<(Utf8PathBuf, String)>,
) -> Result<Vec<(Utf8PathBuf, File)>, String> {
    sources
        .into_iter()
        .map(|(path, source)| {
            let syntax = syn::parse_file(&source)
                .map_err(|error| format!("cannot parse `{path}`: {error}"))?;
            Ok((path, syntax))
        })
        .collect()
}

fn collect_rust_files(
    directory: &Dir,
    relative: &Utf8Path,
    paths: &mut Vec<(Utf8PathBuf, String)>,
) -> Result<(), String> {
    let entries = directory
        .entries()
        .map_err(|error| format!("cannot list `{relative}`: {error}"))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("cannot read an entry under `{relative}`: {error}"))?;
        let name = entry
            .file_name()
            .map_err(|error| format!("cannot read an entry name under `{relative}`: {error}"))?;
        if name == "ui" {
            continue;
        }
        let path = relative.join(&name);
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot inspect `{path}`: {error}"))?;
        if file_type.is_dir() {
            let child = entry
                .open_dir()
                .map_err(|error| format!("cannot open source directory `{path}`: {error}"))?;
            collect_rust_files(&child, &path, paths)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = directory
                .read_to_string(&name)
                .map_err(|error| format!("cannot read Rust source `{path}`: {error}"))?;
            paths.push((path, source));
        }
    }
    Ok(())
}
