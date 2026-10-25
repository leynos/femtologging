//! Conservative process-wide maximum-level maintenance for the Rust log bridge.

use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use parking_lot::Mutex;

use crate::level::FemtoLevel;
use crate::logger::DEFAULT_LOGGER_LEVEL;
use crate::manager;

use super::INSTALL_RESULT;

/// Serializes recomputation and publication of the facade's global maximum.
static MAX_LEVEL_UPDATE_LOCK: Mutex<()> = Mutex::new(());

/// Read the cached result of the one-time adapter installation attempt.
///
/// # Examples
///
/// ```ignore
/// if install_global_logger() {
///     assert!(adapter_is_installed());
/// }
/// ```
fn adapter_is_installed() -> bool {
    INSTALL_RESULT.get().is_some_and(|installed| *installed)
}

/// Map a femtologging threshold to the broadest compatible Rust filter.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(map_femto_to_log_filter(FemtoLevel::Critical), log::LevelFilter::Off);
/// ```
pub(super) fn map_femto_to_log_filter(level: FemtoLevel) -> log::LevelFilter {
    match level {
        FemtoLevel::Trace => log::LevelFilter::Trace,
        FemtoLevel::Debug => log::LevelFilter::Debug,
        FemtoLevel::Info => log::LevelFilter::Info,
        FemtoLevel::Warn => log::LevelFilter::Warn,
        FemtoLevel::Error => log::LevelFilter::Error,
        FemtoLevel::Critical => log::LevelFilter::Off,
    }
}

/// Fold logger thresholds and the default threshold into a safe global maximum.
///
/// # Examples
///
/// ```ignore
/// assert_eq!(
///     conservative_max_level_from([FemtoLevel::Debug].into_iter()),
///     log::LevelFilter::Debug,
/// );
/// ```
pub(super) fn conservative_max_level_from(
    levels: impl Iterator<Item = FemtoLevel>,
) -> log::LevelFilter {
    levels
        .chain(std::iter::once(DEFAULT_LOGGER_LEVEL))
        .map(map_femto_to_log_filter)
        .fold(log::LevelFilter::Off, std::cmp::max)
}

/// Compute the facade maximum from the current Python-free threshold index.
///
/// # Examples
///
/// ```ignore
/// let maximum = conservative_max_level();
/// assert!(maximum <= log::LevelFilter::Trace);
/// ```
fn conservative_max_level() -> log::LevelFilter {
    conservative_max_level_from(manager::registered_logger_levels().into_iter())
}

/// Publish a freshly computed maximum while `MAX_LEVEL_UPDATE_LOCK` is held.
///
/// # Examples
///
/// ```ignore
/// let _guard = MAX_LEVEL_UPDATE_LOCK.lock();
/// refresh_global_max_level_locked();
/// ```
fn refresh_global_max_level_locked() {
    log::set_max_level(conservative_max_level());
}

/// Recompute and publish the global maximum after a registry change.
pub(super) fn refresh_global_max_level() {
    if !adapter_is_installed() {
        return;
    }

    let _guard = MAX_LEVEL_UPDATE_LOCK.lock();
    refresh_global_max_level_locked();
}

/// Update a threshold while keeping the facade maximum conservative.
pub(super) fn update_logger_level(level: &Arc<AtomicU8>, new_level: FemtoLevel) {
    let _guard = MAX_LEVEL_UPDATE_LOCK.lock();
    let installed = adapter_is_installed();
    if installed {
        let new_maximum = map_femto_to_log_filter(new_level);
        if new_maximum > log::max_level() {
            // Widen before lowering the logger threshold so a concurrent
            // logging macro cannot suppress a newly accepted record.
            log::set_max_level(new_maximum);
        }
    }

    level.store(u8::from(new_level), Ordering::Relaxed);
    if installed {
        refresh_global_max_level_locked();
    }
}
