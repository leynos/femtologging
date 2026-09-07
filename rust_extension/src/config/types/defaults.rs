//! Default values and identifier normalization for configuration builders.

use super::ConfigBuilder;

pub(crate) fn normalize_vec(ids: Vec<String>) -> Vec<String> {
    use std::collections::HashSet;

    let mut seen = HashSet::new();
    ids.into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect()
}

impl Default for ConfigBuilder {
    fn default() -> Self {
        Self {
            version: 1,
            disable_existing_loggers: false,
            default_level: None,
            formatters: std::collections::BTreeMap::new(),
            filters: std::collections::BTreeMap::new(),
            handlers: std::collections::BTreeMap::new(),
            loggers: std::collections::BTreeMap::new(),
            root_logger: None,
        }
    }
}
