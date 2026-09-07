//! The CI workflow contract for the environment-access policy.
//!
//! Everything else in this suite proves the policy holds when the gates run.
//! This module proves CI runs them. A gate that CI skips is not a gate, and
//! the ways to skip one are all quiet: a condition on the step, a condition on
//! the job, a trigger that no longer fires for pull requests, or a command
//! wrapped so that it cannot fail.
//!
//! The workflow is parsed rather than searched. A text search is satisfied by
//! `if false; then make lint; fi` and by `make lint || true`, and no list of
//! falsy spellings is reliable, since YAML resolves `false` to a boolean whose
//! string form is `False`. So a required command must be some step's *whole*
//! `run` value, and neither that step nor its job may carry any condition at
//! all.

use saphyr::{LoadableYamlNode, Yaml};

use super::{Fallible, TestResult};

/// The workflow that must run the policy gates on every pull request.
const CI_WORKFLOW: &str = include_str!("../../../../.github/workflows/ci.yml");

/// Repository-relative path, used only in failure messages.
const CI_WORKFLOW_PATH: &str = ".github/workflows/ci.yml";

/// Commands CI must run for the policy to be enforced there.
///
/// `make lint` carries the policy lane, the feature lanes and Whitaker. The
/// Cargo invocation runs `tests/env_access_policy.rs`, which is what holds the
/// configuration to its contract.
const REQUIRED_STEPS: [&str; 2] = [
    "make lint",
    concat!(
        "cargo test --manifest-path rust_extension/Cargo.toml ",
        "--no-default-features -- --test-threads=1"
    ),
];

/// Parse the workflow, naming it in any failure.
fn workflow() -> Fallible<Yaml<'static>> {
    let mut documents = Yaml::load_from_str(CI_WORKFLOW)
        .map_err(|error| format!("parse {CI_WORKFLOW_PATH}: {error}"))?;
    if documents.len() != 1 {
        return Err(format!(
            "{CI_WORKFLOW_PATH} must be one document, found {}",
            documents.len()
        )
        .into());
    }
    Ok(documents.remove(0))
}

/// Fail unless the workflow still triggers on pull requests.
///
/// Without this trigger the whole workflow is inert on a pull request while
/// every step it contains remains correct.
fn ensure_pull_request_trigger(workflow: &Yaml<'_>) -> TestResult {
    let triggers = workflow
        .as_mapping_get("on")
        .ok_or_else(|| format!("{CI_WORKFLOW_PATH} must declare triggers"))?;
    if triggers.as_mapping_get("pull_request").is_none() {
        return Err(format!("{CI_WORKFLOW_PATH} must trigger on pull_request").into());
    }
    Ok(())
}

/// Return the workflow's jobs, by name.
fn jobs<'a>(workflow: &'a Yaml<'a>) -> Fallible<Vec<(String, &'a Yaml<'a>)>> {
    let Yaml::Mapping(jobs) = workflow
        .as_mapping_get("jobs")
        .ok_or_else(|| format!("{CI_WORKFLOW_PATH} must declare jobs"))?
    else {
        return Err(format!("{CI_WORKFLOW_PATH} jobs must be a mapping").into());
    };
    Ok(jobs
        .iter()
        .filter_map(|(name, job)| Some((name.as_str()?.to_owned(), job)))
        .collect())
}

/// Return a job's steps.
fn steps<'a>(job: &'a Yaml<'a>) -> &'a [Yaml<'a>] {
    match job.as_mapping_get("steps") {
        Some(Yaml::Sequence(steps)) => steps.as_slice(),
        _ => &[],
    }
}

/// Report whether a step or job carries any condition at all.
///
/// The value is not inspected. `if: false` and `if: github.event_name ==
/// 'push'` skip the gate just as effectively, and an allow-list of acceptable
/// conditions is a list someone will extend.
fn is_conditional(node: &Yaml<'_>) -> bool {
    node.as_mapping_get("if").is_some()
}

/// Report whether a job tolerates its own failure outright.
///
/// A matrix-driven expression is fine, since it can single out an experimental
/// leg. A literal `true` makes every failure advisory, which is the same hole
/// as a condition wearing different clothes.
fn is_always_tolerated(job: &Yaml<'_>) -> bool {
    job.as_mapping_get("continue-on-error")
        .and_then(Yaml::as_bool)
        == Some(true)
}

/// Fail unless some job runs `command` as a whole step, unconditionally.
fn ensure_command_runs(workflow: &Yaml<'_>, command: &str) -> TestResult {
    for (name, job) in jobs(workflow)? {
        let Some(step) = steps(job)
            .iter()
            .find(|step| step.as_mapping_get("run").and_then(Yaml::as_str) == Some(command))
        else {
            continue;
        };
        if is_conditional(step) {
            return Err(
                format!("the {command:?} step in job {name} must carry no condition").into(),
            );
        }
        if is_conditional(job) {
            return Err(format!("job {name} runs {command:?} but carries a condition").into());
        }
        if is_always_tolerated(job) {
            return Err(
                format!("job {name} runs {command:?} but tolerates its own failure").into(),
            );
        }
        return Ok(());
    }
    Err(format!("{CI_WORKFLOW_PATH} must run {command:?} as a step's whole command").into())
}

/// Fail unless CI runs every policy gate unconditionally on pull requests.
pub(crate) fn ensure_ci_runs_the_policy_gates() -> TestResult {
    let workflow = workflow()?;
    ensure_pull_request_trigger(&workflow)?;
    for command in REQUIRED_STEPS {
        ensure_command_runs(&workflow, command)?;
    }
    Ok(())
}
