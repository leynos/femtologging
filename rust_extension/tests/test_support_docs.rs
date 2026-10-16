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

#[test]
fn test_support_types_have_documentation() {
    let mut checker = DocumentationChecker::default();
    for (path, source) in rust_sources() {
        let syntax = syn::parse_file(&source).expect("test-support source should parse");
        checker.file_is_test_support = is_test_support_file(&path);
        checker.path = path.to_string();
        checker.visit_file(&syntax);
    }

    assert!(
        checker.errors.is_empty(),
        "undocumented test-support items:\n{}",
        checker.errors.join("\n")
    );
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
        if self.checks_current_scope() {
            return true;
        }
        has_test_cfg(attrs)
    }

    fn check_item(&mut self, attrs: &[Attribute], kind: &str, name: &str) {
        if !self.is_test_scoped(attrs) {
            return;
        }
        if has_doc(attrs) {
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
            self.errors.push(format!(
                "{}: undocumented field `{}::{}`",
                self.path,
                item.ident,
                field.ident.as_ref().expect("named field")
            ));
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

fn has_test_cfg(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && matches!(
                &attribute.meta,
                syn::Meta::List(list) if list.tokens.to_string().contains("test")
            )
    })
}

fn is_test_support_file(path: &Utf8Path) -> bool {
    path.starts_with("tests")
        || path
            .file_name()
            .is_some_and(|name| name.ends_with("_tests.rs"))
}

fn rust_sources() -> Vec<(Utf8PathBuf, String)> {
    let root = Dir::open_ambient_dir(env!("CARGO_MANIFEST_DIR"), ambient_authority())
        .expect("open crate root for test-support documentation scan");
    let mut paths = Vec::new();
    for root_name in ["src", "tests"] {
        let directory = root
            .open_dir(root_name)
            .expect("test-support source directory should be readable");
        collect_rust_files(&directory, Utf8Path::new(root_name), &mut paths);
    }
    paths.sort_by(|(left, _), (right, _)| left.cmp(right));
    paths
}

fn collect_rust_files(
    directory: &Dir,
    relative: &Utf8Path,
    paths: &mut Vec<(Utf8PathBuf, String)>,
) {
    for entry in directory
        .entries()
        .expect("test-support directory should be readable")
    {
        let entry = entry.expect("test-support directory entry should be readable");
        let name = entry
            .file_name()
            .expect("test-support entry name should be valid UTF-8");
        if name == "ui" {
            continue;
        }
        let path = relative.join(&name);
        if entry
            .file_type()
            .expect("test-support entry type should be readable")
            .is_dir()
        {
            let child = entry
                .open_dir()
                .expect("test-support child directory should be readable");
            collect_rust_files(&child, &path, paths);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = directory
                .read_to_string(&name)
                .expect("test-support Rust source should be readable");
            paths.push((path, source));
        }
    }
}
