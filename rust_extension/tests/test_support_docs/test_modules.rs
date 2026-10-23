//! Finds source files that belong to Rust test-support module scopes.

use std::collections::{HashSet, VecDeque};

use camino::{Utf8Path, Utf8PathBuf};
use syn::{Attribute, File, ItemMod, visit::Visit};

/// Finds test-support files, including external modules declared under tests.
pub(super) fn test_support_paths(sources: &[(Utf8PathBuf, File)]) -> HashSet<Utf8PathBuf> {
    let source_paths = sources
        .iter()
        .map(|(path, _)| path.clone())
        .collect::<HashSet<_>>();
    let mut test_paths = sources
        .iter()
        .map(|(path, _)| path)
        .filter(|path| is_test_support_file(path))
        .cloned()
        .collect::<HashSet<_>>();
    let mut pending = test_paths.iter().cloned().collect::<VecDeque<_>>();

    for (path, syntax) in sources {
        if test_paths.contains(path) {
            continue;
        }
        let mut collector = TestModuleCollector::new(
            module_directory_for_file(path),
            source_directory_for_file(path),
            false,
            &source_paths,
        );
        collector.visit_file(syntax);
        enqueue_test_modules(collector.test_modules, &mut test_paths, &mut pending);
    }

    while let Some(path) = pending.pop_front() {
        let Some((_, syntax)) = sources.iter().find(|(source_path, _)| *source_path == path) else {
            continue;
        };
        let mut collector = TestModuleCollector::new(
            module_directory_for_file(&path),
            source_directory_for_file(&path),
            true,
            &source_paths,
        );
        collector.visit_file(syntax);
        enqueue_test_modules(collector.test_modules, &mut test_paths, &mut pending);
    }

    test_paths
}

fn enqueue_test_modules(
    modules: Vec<Utf8PathBuf>,
    known_paths: &mut HashSet<Utf8PathBuf>,
    pending: &mut VecDeque<Utf8PathBuf>,
) {
    for path in modules {
        if known_paths.insert(path.clone()) {
            pending.push_back(path);
        }
    }
}

/// Finds external source files reached from `#[cfg(test)]` modules.
struct TestModuleCollector<'a> {
    /// Directory used to resolve ordinary `mod child;` declarations.
    module_directory: Utf8PathBuf,
    /// Directory used to resolve explicit `#[path = "..."]` attributes.
    source_directory: Utf8PathBuf,
    /// Whether the current inline or file scope is compiled for tests.
    in_test_module: bool,
    /// Source files available to the scanner.
    source_paths: &'a HashSet<Utf8PathBuf>,
    /// External module files that belong to test support.
    test_modules: Vec<Utf8PathBuf>,
}

impl<'a> TestModuleCollector<'a> {
    fn new(
        module_directory: Utf8PathBuf,
        source_directory: Utf8PathBuf,
        in_test_module: bool,
        source_paths: &'a HashSet<Utf8PathBuf>,
    ) -> Self {
        Self {
            module_directory,
            source_directory,
            in_test_module,
            source_paths,
            test_modules: Vec::new(),
        }
    }

    fn external_module_path(&self, item: &ItemMod) -> Option<Utf8PathBuf> {
        let path_override = module_path_override(&item.attrs);
        let candidates = match path_override {
            Some(path) => vec![self.source_directory.join(path)],
            None => {
                let name = item.ident.to_string();
                vec![
                    self.module_directory.join(format!("{name}.rs")),
                    self.module_directory.join(&name).join("mod.rs"),
                ]
            }
        };
        candidates
            .into_iter()
            .map(normalize_relative_path)
            .find(|path| self.source_paths.contains(path))
    }

    fn external_test_module_path(
        &self,
        item: &ItemMod,
        is_test_module: bool,
    ) -> Option<Utf8PathBuf> {
        if item.content.is_none() && is_test_module {
            self.external_module_path(item)
        } else {
            None
        }
    }
}

fn module_path_override(attrs: &[Attribute]) -> Option<String> {
    attrs.iter().find_map(|attribute| {
        let syn::Meta::NameValue(name_value) = &attribute.meta else {
            return None;
        };
        if !name_value.path.is_ident("path") {
            return None;
        }
        let syn::Expr::Lit(value) = &name_value.value else {
            return None;
        };
        let syn::Lit::Str(value) = &value.lit else {
            return None;
        };
        Some(value.value())
    })
}

impl<'ast> Visit<'ast> for TestModuleCollector<'_> {
    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        let was_in_test_module = self.in_test_module;
        let is_test_module = was_in_test_module || has_test_cfg(&item.attrs);

        if let Some(path) = self.external_test_module_path(item, is_test_module) {
            self.test_modules.push(path);
        }

        if item.content.is_some() {
            let previous_directory = self.module_directory.clone();
            let previous_source_directory = self.source_directory.clone();
            self.module_directory.push(item.ident.to_string());
            self.source_directory.push(item.ident.to_string());
            self.in_test_module = is_test_module;
            syn::visit::visit_item_mod(self, item);
            self.module_directory = previous_directory;
            self.source_directory = previous_source_directory;
        }
        self.in_test_module = was_in_test_module;
    }
}

fn normalize_relative_path(path: Utf8PathBuf) -> Utf8PathBuf {
    let mut components = Vec::new();
    for component in path.as_str().split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop();
            }
            _ => components.push(component),
        }
    }
    Utf8PathBuf::from(components.join("/"))
}

fn module_directory_for_file(path: &Utf8Path) -> Utf8PathBuf {
    let parent = path.parent().unwrap_or(Utf8Path::new(""));
    match path.file_name() {
        Some("lib.rs" | "main.rs" | "mod.rs") => parent.to_owned(),
        _ => path
            .file_stem()
            .map(|stem| parent.join(stem))
            .unwrap_or_else(|| parent.to_owned()),
    }
}

fn source_directory_for_file(path: &Utf8Path) -> Utf8PathBuf {
    path.parent().unwrap_or(Utf8Path::new("")).to_owned()
}

pub(super) fn has_test_cfg(attrs: &[syn::Attribute]) -> bool {
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
