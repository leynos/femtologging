"""Tests for the en-GB-oxendict typos enforcement.

These cover the generator that produces ``typos.toml`` (``render_config`` and
``main``), the guarantee that the checked-in configuration is regenerated
rather than hand-edited, and the Makefile wiring that runs ``typos`` through
the ``markdownlint`` target.
"""

from __future__ import annotations

import importlib.util as ilu
import pathlib as pth
import re
import shutil
import subprocess  # noqa: S404 - tests invoke the generator and typos CLIs.
import sys
import typing as typ

import pytest

if typ.TYPE_CHECKING:
    import types

_UVX_AVAILABLE = shutil.which("uvx") is not None


def repo_root() -> pth.Path:
    """Return the repository root directory."""
    return pth.Path(__file__).resolve().parents[1]


def _load_generator() -> types.ModuleType:
    """Load ``scripts/generate_typos_config.py`` as an importable module."""
    path = repo_root() / "scripts" / "generate_typos_config.py"
    spec = ilu.spec_from_file_location("generate_typos_config", path)
    if spec is None or spec.loader is None:
        msg = f"could not load generator module from {path}"
        raise RuntimeError(msg)
    module = ilu.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


@pytest.fixture(scope="module")
def generator() -> types.ModuleType:
    """Return the loaded typos-config generator module."""
    return _load_generator()


@pytest.fixture(scope="module")
def rendered_config(generator: types.ModuleType) -> str:
    """Return the generated ``typos.toml`` contents."""
    text: str = generator.render_config()
    return text


def test_render_config_is_deterministic(generator: types.ModuleType) -> None:
    """``render_config`` yields identical output on repeated calls."""
    assert generator.render_config() == generator.render_config()


def test_render_config_declares_locale_and_tables(rendered_config: str) -> None:
    """The config selects the en-GB locale and the expected TOML tables."""
    assert 'locale = "en-gb"' in rendered_config
    assert "[files]" in rendered_config
    assert "[default.extend-words]" in rendered_config
    assert rendered_config.endswith("\n")


@pytest.mark.parametrize(
    ("ise", "ize"),
    [
        ("organise", "organize"),
        ("organisation", "organization"),
        ("specialise", "specialize"),
        ("tokenise", "tokenize"),
        ("initialise", "initialize"),
    ],
)
def test_oxford_inflections_are_restored(
    rendered_config: str,
    ise: str,
    ize: str,
) -> None:
    """Each stem accepts the Oxford form and corrects the ``-ise`` form."""
    assert f'{ize} = "{ize}"' in rendered_config
    assert f'{ise} = "{ize}"' in rendered_config


@pytest.mark.parametrize(
    "entry",
    [
        'amortize = "amortize"',
        'amortise = "amortize"',
        'amortization = "amortization"',
    ],
)
def test_amortize_is_accepted_as_oxford(rendered_config: str, entry: str) -> None:
    """Oxford retains ``amortize``; the ``amort`` stem must accept it."""
    assert entry in rendered_config


@pytest.mark.parametrize("word", ["OT", "astroid", "yse", "mis"])
def test_extra_words_are_accepted_verbatim(rendered_config: str, word: str) -> None:
    """Project-specific words are accepted without correction."""
    assert f'{word} = "{word}"' in rendered_config


@pytest.mark.parametrize("word", ["advise", "revise", "supervise", "exercise"])
def test_ise_only_words_are_left_untouched(rendered_config: str, word: str) -> None:
    """Genuinely ``-ise``-only words are not forced to ``-ize``."""
    assert f"{word} = " not in rendered_config


@pytest.mark.parametrize("word", ["analyse", "analyze", "paralyse"])
def test_yse_words_are_left_to_the_locale(rendered_config: str, word: str) -> None:
    """``-yse`` words carry no override; the locale already enforces them."""
    assert f"{word} = " not in rendered_config


def test_main_writes_rendered_config(
    generator: types.ModuleType,
    tmp_path: pth.Path,
) -> None:
    """``main`` writes exactly ``render_config`` output to the given path."""
    output = tmp_path / "typos.toml"
    generator.main(output)
    assert output.read_text(encoding="utf-8") == generator.render_config()


def test_committed_config_matches_generator(rendered_config: str) -> None:
    """The checked-in ``typos.toml`` is regenerated, not hand-edited."""
    committed = (repo_root() / "typos.toml").read_text(encoding="utf-8")
    assert committed == rendered_config


def test_generator_cli_writes_default_config(
    generator: types.ModuleType,
    tmp_path: pth.Path,
) -> None:
    """Running the generator as a script writes the config to its default path."""
    scripts_dir = tmp_path / "scripts"
    scripts_dir.mkdir()
    source = repo_root() / "scripts" / "generate_typos_config.py"
    script = scripts_dir / "generate_typos_config.py"
    script.write_text(source.read_text(encoding="utf-8"), encoding="utf-8")

    subprocess.run(  # noqa: S603 - trusted interpreter runs a copied script.
        [sys.executable, str(script)],
        check=True,
    )

    written = (tmp_path / "typos.toml").read_text(encoding="utf-8")
    assert written == generator.render_config()


def test_main_raises_when_parent_directory_missing(
    generator: types.ModuleType,
    tmp_path: pth.Path,
) -> None:
    """``main`` surfaces a filesystem error when the target directory is absent."""
    with pytest.raises(FileNotFoundError):
        generator.main(tmp_path / "absent" / "typos.toml")


def _find_target_line(lines: list[str], prefix: str) -> int:
    """Return the index of the first line starting with a prefix.

    Parameters
    ----------
    lines : list[str]
        The Makefile split into individual lines.
    prefix : str
        The target prefix to search for, such as ``markdownlint:``.

    Returns
    -------
    int
        The index of the first matching line, or ``-1`` if none matches.

    """
    for index, line in enumerate(lines):
        if line.startswith(prefix):
            return index
    return -1


def _collect_recipe_lines(lines: list[str], start: int) -> list[str]:
    """Collect the tab-indented recipe lines of a Makefile target.

    Parameters
    ----------
    lines : list[str]
        The Makefile split into individual lines.
    start : int
        The index at which to begin collecting recipe lines.

    Returns
    -------
    list[str]
        The stripped recipe lines, stopping at the first non-indented,
        non-blank line.

    """
    recipe: list[str] = []
    for line in lines[start:]:
        if line.startswith("\t"):
            recipe.append(line.strip())
        elif line.strip():
            break
    return recipe


def _markdownlint_recipe() -> str:
    """Return the recipe lines of the Makefile ``markdownlint`` target."""
    lines = (repo_root() / "Makefile").read_text(encoding="utf-8").splitlines()
    start = _find_target_line(lines, "markdownlint:")
    if start == -1:
        msg = "markdownlint target recipe not found in Makefile"
        raise AssertionError(msg)
    recipe = _collect_recipe_lines(lines, start + 1)
    if not recipe:
        msg = "markdownlint target recipe not found in Makefile"
        raise AssertionError(msg)
    return "\n".join(recipe)


def test_markdownlint_target_invokes_typos() -> None:
    """The ``markdownlint`` target runs ``typos`` against ``typos.toml``."""
    recipe = _markdownlint_recipe()
    assert "$(TYPOS) --config typos.toml --force-exclude" in recipe


def test_typos_version_is_pinned() -> None:
    """The Makefile pins the ``typos`` version as a single source of truth."""
    makefile = (repo_root() / "Makefile").read_text(encoding="utf-8")
    assert "TYPOS_VERSION ?= 1.48.0" in makefile
    assert "TYPOS ?= uvx typos@$(TYPOS_VERSION)" in makefile


def test_find_target_line_returns_negative_one_when_absent() -> None:
    """The finder reports ``-1`` when no line starts with the prefix."""
    assert _find_target_line(["all:", "\techo hi"], "markdownlint:") == -1


def test_collect_recipe_lines_stops_at_next_target() -> None:
    """Recipe collection halts at the next non-indented, non-blank line."""
    lines = ["markdownlint:", "\tone", "\ttwo", "", "nixie:", "\tthree"]
    assert _collect_recipe_lines(lines, 1) == ["one", "two"]


def test_collect_recipe_lines_empty_without_recipe() -> None:
    """A target with no indented lines yields an empty recipe."""
    assert _collect_recipe_lines(["markdownlint:", "nixie:"], 1) == []


def _pinned_typos_version() -> str:
    """Return the ``typos`` version pinned by ``TYPOS_VERSION`` in the Makefile."""
    makefile = (repo_root() / "Makefile").read_text(encoding="utf-8")
    match = re.search(r"TYPOS_VERSION \?= (\S+)", makefile)
    if match is None:
        msg = "TYPOS_VERSION not found in Makefile"
        raise AssertionError(msg)
    return match.group(1)


@pytest.mark.skipif(not _UVX_AVAILABLE, reason="uvx/typos CLI unavailable")
def test_typos_cli_enforces_oxford_spelling(tmp_path: pth.Path) -> None:
    """The ``typos`` CLI accepts Oxford ``-ize`` forms and rejects ``-ise`` forms."""
    config = repo_root() / "typos.toml"
    base = ["uvx", f"typos@{_pinned_typos_version()}", "--config", str(config)]

    accepted = tmp_path / "accepted.md"
    accepted.write_text("We organize and serialize tokens.\n", encoding="utf-8")
    rejected = tmp_path / "rejected.md"
    rejected.write_text("We organise and serialise tokens.\n", encoding="utf-8")

    accept = subprocess.run(  # noqa: S603 - command list uses trusted local paths.
        [*base, str(accepted)],
        capture_output=True,
        text=True,
        check=False,
    )
    reject = subprocess.run(  # noqa: S603 - command list uses trusted local paths.
        [*base, str(rejected)],
        capture_output=True,
        text=True,
        check=False,
    )

    assert accept.returncode == 0, accept.stdout + accept.stderr
    assert reject.returncode != 0
    assert "organize" in reject.stdout + reject.stderr
