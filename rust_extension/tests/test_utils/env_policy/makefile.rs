//! Queries over the repository `Makefile`, and the assertions built on them.
//!
//! Recipes are judged one whole command at a time rather than by searching
//! their text. A substring search is satisfied by `if false; then <command>;
//! fi`, which would certify a target that runs nothing.

use std::fmt::Write as _;
use std::process::Command;

use super::config::{declared_features, repository_root};
use super::{DISALLOWED_METHODS_LINT, Fallible, TestResult};

/// Feature lanes the policy must be linted under regardless of what else the
/// lane list gains. `none` and `all` between them compile both arms of every
/// feature gate, which `--all-features` alone cannot do.
const REQUIRED_FEATURE_LANES: [&str; 2] = ["none", "all"];

/// The name of a Makefile recipe this contract depends on.
#[derive(Clone, Copy)]
pub(crate) struct Recipe(&'static str);

/// The name of a Makefile variable this contract depends on.
#[derive(Clone, Copy)]
pub(crate) struct Variable(&'static str);

/// The aggregate lint target CI invokes.
const LINT: Recipe = Recipe("lint");
/// The Rust lint target, which must run the policy lane first.
const LINT_RUST: Recipe = Recipe("lint-rust");
/// The policy lane itself.
const LINT_ENV_POLICY: Recipe = Recipe("lint-env-policy");
/// The feature lanes the policy is linted under.
const FEATURE_LANES: Variable = Variable("ENV_POLICY_FEATURE_LANES");
/// The Cargo arguments every policy lane carries.
const CARGO_ARGS: Variable = Variable("ENV_POLICY_CARGO_ARGS");
/// The lint-driver arguments every policy lane carries.
const LINT_ARGS: Variable = Variable("ENV_POLICY_LINT_ARGS");
/// The script that walks the lanes.
const LANES_SCRIPT: Variable = Variable("LINT_LANES_SCRIPT");

/// Collapse runs of whitespace so a joined command compares predictably.
fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The repository `Makefile`, embedded once and queried by name.
pub(crate) struct Makefile(&'static str);

impl Makefile {
    /// Return the repository Makefile, embedded at compile time.
    pub(crate) fn embedded() -> Self {
        Self(include_str!("../../../../Makefile"))
    }

    /// Return the body of the named recipe, including its own line.
    pub(crate) fn recipe(&self, target: Recipe) -> Fallible<String> {
        let Recipe(name) = target;
        let prefix = format!("{name}:");
        let mut lines = self.0.lines().skip_while(|line| !line.starts_with(&prefix));
        let header = lines
            .next()
            .ok_or_else(|| format!("Makefile has no `{name}` target"))?;
        let mut recipe = String::from(header);
        for line in lines {
            if !line.starts_with('\t') && !line.trim().is_empty() {
                break;
            }
            writeln!(recipe)?;
            recipe.push_str(line);
        }
        Ok(recipe)
    }

    /// Return the recipe's commands, one per logical line.
    ///
    /// Backslash continuations are joined and interior whitespace collapsed,
    /// so a command spread over several lines is one string. Make's recipe
    /// prefixes (`@`, `-`, `+`) are stripped because they change reporting,
    /// not what runs.
    ///
    /// Callers judge a whole command rather than searching the recipe text.
    /// A substring search is satisfied by `if false; then <command>; fi`,
    /// which certifies a target that runs nothing.
    pub(crate) fn commands(&self, target: Recipe) -> Fallible<Vec<String>> {
        let body = self.recipe(target)?;
        let mut commands = Vec::new();
        let mut current = String::new();
        for line in body.lines().skip(1) {
            let Some(text) = line.strip_prefix('\t') else {
                continue;
            };
            let text = text.trim();
            current.push_str(text.strip_suffix('\\').unwrap_or(text));
            if text.ends_with('\\') {
                current.push(' ');
                continue;
            }
            commands.push(collapse_whitespace(
                current.trim_start_matches(['@', '-', '+']),
            ));
            current = String::new();
        }
        Ok(commands)
    }

    /// Return the recipe's declared prerequisites.
    ///
    /// Prerequisites cannot be wrapped or made conditional the way a recipe
    /// command can, so a target reached this way is reached unconditionally.
    pub(crate) fn prerequisites(&self, target: Recipe) -> Fallible<Vec<String>> {
        let body = self.recipe(target)?;
        let header = body
            .lines()
            .next()
            .ok_or_else(|| "recipe has no header".to_string())?;
        let (_, rest) = header
            .split_once(':')
            .ok_or_else(|| "recipe header has no colon".to_string())?;
        Ok(rest
            .split("##")
            .next()
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect())
    }

    /// Return the value of a `?=` variable.
    pub(crate) fn variable(&self, variable: Variable) -> Fallible<&'static str> {
        let Variable(name) = variable;
        let prefix = format!("{name} ?=");
        self.0
            .lines()
            .find_map(|line| line.strip_prefix(prefix.as_str()))
            .map(str::trim)
            .ok_or_else(|| format!("Makefile must define {name}").into())
    }
}

/// Fail unless the policy lane list covers both arms of every feature gate.
pub(crate) fn ensure_lanes_cover_every_feature(makefile: &Makefile) -> TestResult {
    let lanes: Vec<&str> = makefile
        .variable(FEATURE_LANES)?
        .split_whitespace()
        .collect();
    let missing_lane = REQUIRED_FEATURE_LANES
        .into_iter()
        .find(|required| !lanes.contains(required));
    if let Some(required) = missing_lane {
        return Err(format!(
            "ENV_POLICY_FEATURE_LANES must include the {required:?} lane, found {lanes:?}"
        )
        .into());
    }
    let missing_feature = declared_features()?
        .into_iter()
        .find(|feature| !lanes.contains(&feature.as_str()));
    match missing_feature {
        None => Ok(()),
        Some(feature) => Err(format!(
            "ENV_POLICY_FEATURE_LANES must lint the {feature:?} feature, found {lanes:?}"
        )
        .into()),
    }
}

/// Fail unless the policy lint reaches every target kind and denies the lint
/// outright.
pub(crate) fn ensure_flags_deny_the_policy(makefile: &Makefile) -> TestResult {
    let cargo_args = makefile.variable(CARGO_ARGS)?;
    if !cargo_args
        .split_whitespace()
        .any(|flag| flag == "--all-targets")
    {
        return Err(format!(
            "ENV_POLICY_CARGO_ARGS must lint every target kind, found {cargo_args:?}"
        )
        .into());
    }
    let lint_args = makefile.variable(LINT_ARGS)?;
    let denials: Vec<&str> = lint_args.split_whitespace().collect();
    if denials
        .windows(2)
        .all(|pair| pair != ["-D", DISALLOWED_METHODS_LINT])
    {
        return Err(format!(
            "ENV_POLICY_LINT_ARGS must deny {DISALLOWED_METHODS_LINT}, found {lint_args:?}"
        )
        .into());
    }
    Ok(())
}

/// The Makefile variables the policy recipe must hand to the driver.
const POLICY_EXPORTS: [&str; 3] = [
    "INPUT_LANES=\"$(ENV_POLICY_FEATURE_LANES)\"",
    "INPUT_CARGO_ARGS=\"$(ENV_POLICY_CARGO_ARGS)\"",
    "INPUT_LINT_ARGS=\"$(ENV_POLICY_LINT_ARGS)\"",
];

/// The targets `lint-rust` must run before the Whitaker suite.
const RUST_LINT_PREREQUISITES: [&str; 2] = ["lint-env-policy", "lint-lanes-test"];

/// Return the policy recipe's single command, or explain why there isn't one.
///
/// The recipe is one env-prefixed invocation. Requiring exactly one command
/// that *ends* with the driver call is what rules out a wrapper such as
/// `if false; then <command>; fi`, whose command ends with `fi`.
fn policy_command(makefile: &Makefile) -> Fallible<String> {
    let mut commands = makefile.commands(LINT_ENV_POLICY)?;
    if commands.len() != 1 {
        return Err(format!(
            "lint-env-policy must be one command, found {}: {commands:?}",
            commands.len()
        )
        .into());
    }
    Ok(commands.remove(0))
}

/// Fail unless the policy recipe hands the driver every input it needs.
fn ensure_policy_recipe_drives_the_script(makefile: &Makefile) -> TestResult {
    let command = policy_command(makefile)?;
    let missing = POLICY_EXPORTS
        .into_iter()
        .find(|exported| !command.contains(exported));
    if let Some(exported) = missing {
        return Err(format!("lint-env-policy must export {exported}").into());
    }
    if !command.ends_with("uv run --script $(LINT_LANES_SCRIPT)") {
        return Err(
            format!("lint-env-policy must end in the lane driver call, found {command:?}").into(),
        );
    }
    let script = makefile.variable(LANES_SCRIPT)?;
    if script != "scripts/lint_rust_lanes.py" {
        return Err(
            format!("LINT_LANES_SCRIPT must name the lane driver, found {script:?}").into(),
        );
    }
    Ok(())
}

/// Fail unless `parent` runs `child`, by either unwrappable route.
///
/// A prerequisite cannot be wrapped or made conditional at all. A recipe
/// command can, so it is matched whole: `if false; then $(MAKE) child; fi`
/// and `$(MAKE) child || true` are both rejected.
fn ensure_target_runs(makefile: &Makefile, parent: Recipe, child: &str) -> TestResult {
    let Recipe(parent_name) = parent;
    if makefile
        .prerequisites(parent)?
        .iter()
        .any(|declared| declared == child)
    {
        return Ok(());
    }
    let delegation = format!("$(MAKE) {child}");
    if makefile
        .commands(parent)?
        .iter()
        .any(|command| *command == delegation)
    {
        return Ok(());
    }
    Err(format!(
        "{parent_name} must run {child} as a prerequisite or as a command of its own, \
         not inside a wrapper"
    )
    .into())
}

/// Fail unless `make lint` reaches the policy recipe.
fn ensure_lint_reaches_lint_rust(makefile: &Makefile) -> TestResult {
    let declared = makefile.prerequisites(LINT_RUST)?;
    let missing = RUST_LINT_PREREQUISITES
        .into_iter()
        .find(|required| !declared.iter().any(|entry| entry == required));
    if let Some(required) = missing {
        return Err(format!("lint-rust must run {required}, found {declared:?}").into());
    }
    ensure_target_runs(makefile, LINT, "lint-rust")
}

/// Fail unless `make lint` still reaches the policy lane through the driver.
///
/// The lane walk lives in `scripts/lint_rust_lanes.py`, so the recipe has to
/// hand that script the lane list and both argument sets, and `make lint` has
/// to reach it.
pub(crate) fn ensure_lint_reaches_the_policy_lane(makefile: &Makefile) -> TestResult {
    ensure_policy_recipe_drives_the_script(makefile)?;
    ensure_lint_reaches_lint_rust(makefile)
}

/// Run `make lint-env-policy` over the given lanes and report whether it
/// succeeded.
///
/// The lint flags are narrowed to `-A clippy::all` so the run exercises the
/// recipe's exit-status handling rather than the policy itself. A lane naming
/// a feature the crate does not declare fails immediately, before any build.
pub(crate) fn policy_lane_run_succeeds(lanes: &str) -> Fallible<bool> {
    let status = Command::new("make")
        .current_dir(repository_root()?)
        .arg("lint-env-policy")
        .arg(format!("ENV_POLICY_FEATURE_LANES={lanes}"))
        .arg("ENV_POLICY_LINT_ARGS=-A clippy::all")
        .output()
        .map_err(|error| format!("run make lint-env-policy over lanes {lanes:?}: {error}"))?
        .status;
    Ok(status.success())
}
