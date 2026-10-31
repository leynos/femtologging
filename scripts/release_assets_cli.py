"""Command-line interface for the release asset reconciliation helpers.

The release workflow runs this module with the standard library only. It has
no network access; requests remain in workflow steps that call ``curl`` or
``gh``.
"""

from __future__ import annotations

import argparse
import sys
import typing as typ
from pathlib import Path

from scripts.release_asset_errors import MissingWorkflowOptionError
from scripts.release_assets import (
    ReleaseAssetError,
    State,
    carryover,
    check_fetched,
    stage_github,
    stage_pypi,
    verify,
    write_output,
)

if typ.TYPE_CHECKING:
    import collections.abc as cabc


def _parser() -> argparse.ArgumentParser:
    """Build the CLI called by the release workflow."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("command", choices=_COMMANDS)
    parser.add_argument("--publish", type=Path, required=True)
    parser.add_argument("--assets", type=Path, required=True)
    parser.add_argument("--index", type=Path)
    parser.add_argument("--github-files", type=Path)
    parser.add_argument("--bundles", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument(
        "--github-output",
        type=Path,
        help="GitHub Actions output file for commands that report a result.",
    )
    return parser


def _path(args: argparse.Namespace, option: str) -> Path:
    """Return a path option required by the selected CLI command."""
    value = getattr(args, option)
    if value is None:
        raise MissingWorkflowOptionError(option.replace("_", "-"), args.command)
    return value


def _carryover(state: State, args: argparse.Namespace) -> None:
    """Print names shared by the build and GitHub Release, one per line."""
    del args
    for name in carryover(state):
        print(name)


def _stage_pypi(state: State, args: argparse.Namespace) -> None:
    """Stage the PyPI upload set and report whether any name remains."""
    destination = _path(args, "github_output")
    remaining = stage_pypi(state, _path(args, "github_files"))
    write_output("remaining", value=remaining, destination=destination)


def _stage_github(state: State, args: argparse.Namespace) -> None:
    """Stage missing GitHub names and print required PyPI URLs as TSV."""
    output = _path(args, "output")
    destination = _path(args, "github_output")
    github_files = args.github_files
    fetch = stage_github(
        state,
        _path(args, "bundles"),
        output,
        github_files,
    )
    for name, url in fetch:
        print(f"{name}\t{url}")
    write_output(
        "pending", value=bool(fetch) or any(output.iterdir()), destination=destination
    )


def _check_fetched(state: State, args: argparse.Namespace) -> None:
    """Check every PyPI file staged for upload against its index hash."""
    check_fetched(state, _path(args, "output"))


def _verify(state: State, args: argparse.Namespace) -> None:
    """Fail unless PyPI and GitHub agree on every artefact digest."""
    problems = verify(state, _path(args, "bundles"))
    if problems:
        raise ReleaseAssetError("; ".join(problems))
    print("PyPI and the GitHub Release hold matching artefact bytes.")


_COMMANDS: dict[str, cabc.Callable[[State, argparse.Namespace], None]] = {
    "carryover": _carryover,
    "stage-pypi": _stage_pypi,
    "stage-github": _stage_github,
    "check-fetched": _check_fetched,
    "verify": _verify,
}
_INDEX_COMMANDS = frozenset({"stage-pypi", "stage-github", "check-fetched", "verify"})


def main(argv: cabc.Sequence[str] | None = None) -> int:
    """Run one reconciliation command and report failures as annotations.

    Parameters
    ----------
    argv
        Optional argument sequence. When omitted, parse the process arguments.

    Returns
    -------
    int
        ``0`` when the command succeeds, or ``1`` for a reconciliation error.

    Examples
    --------
    Running ``main(["verify", "--publish", "dist", "--assets", "assets.json",
    "--bundles", "bundles"])`` checks the local release snapshot.
    """
    args = _parser().parse_args(argv)
    try:
        index = _path(args, "index") if args.command in _INDEX_COMMANDS else None
        state = State.load(args.publish, index, args.assets)
        _COMMANDS[args.command](state, args)
    except (ReleaseAssetError, OSError, KeyError, ValueError, TypeError) as error:
        print(f"::error title=release-assets::{error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
