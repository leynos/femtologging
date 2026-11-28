//! Compile-pass coverage for the config builders' `const` items.
//!
//! Each `const fn` below applies one public setter, or reads one public getter,
//! on a builder passed in as an argument, so the fixture stops compiling the
//! moment an item loses its `const` qualifier. A run-time test cannot detect
//! that. The builders arrive as parameters because their constructors are
//! ordinary `fn`s.

use _femtologging_rs::{ConfigBuilder, FemtoLevel, LoggerConfigBuilder, LoggerMutationBuilder};

const fn logger_config_setters(builder: LoggerConfigBuilder) -> LoggerConfigBuilder {
    builder
        .with_level(FemtoLevel::Info)
        .with_propagate(false)
}

const fn logger_config_getters(
    builder: &LoggerConfigBuilder,
) -> (Option<FemtoLevel>, Option<bool>) {
    (builder.level_opt(), builder.propagate_opt())
}

const fn config_builder_setters(builder: ConfigBuilder) -> ConfigBuilder {
    builder
        .with_version(1)
        .with_disable_existing_loggers(true)
        .with_default_level(FemtoLevel::Info)
}

const fn config_builder_getters(builder: &ConfigBuilder) -> (u8, Option<FemtoLevel>) {
    (builder.version(), builder.default_level())
}

const fn logger_mutation_setters(builder: LoggerMutationBuilder) -> LoggerMutationBuilder {
    builder
        .with_level(FemtoLevel::Info)
        .with_propagate(false)
}

fn main() {
    // The const fns are the assertion; naming them keeps them used.
    let _: fn(LoggerConfigBuilder) -> LoggerConfigBuilder = logger_config_setters;
    let _: fn(&LoggerConfigBuilder) -> (Option<FemtoLevel>, Option<bool>) =
        logger_config_getters;
    let _: fn(ConfigBuilder) -> ConfigBuilder = config_builder_setters;
    let _: fn(&ConfigBuilder) -> (u8, Option<FemtoLevel>) = config_builder_getters;
    let _: fn(LoggerMutationBuilder) -> LoggerMutationBuilder = logger_mutation_setters;
}