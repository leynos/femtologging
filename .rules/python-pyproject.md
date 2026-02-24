# 1. Overview of `uv` and `pyproject.toml`

Astral's `uv` is a Rust-based project and package manager that uses
`pyproject.toml` as its central configuration file. When you run commands like
`uv init`, `uv sync` or `uv run`, `uv` will:

1. Look for a `pyproject.toml` in the project root and keep a lockfile
   (`uv.lock`) in sync with it.
2. Create a virtual environment (`.venv`) if one does not already exist.
3. Read dependency specifications (and any build-system directives) to install
   or update packages accordingly.

In other words, a `pyproject.toml` drives everything—from metadata to
dependencies to build instructions—without the need for `requirements.txt` or a
separate `setup.py` file.

______________________________________________________________________

## 2. The `[project]` Table (PEP 621)

The `[project]` table is defined by PEP 621 and is now the canonical place to
declare metadata (name, version, authors, etc.) and runtime dependencies. At
minimum, PEP 621 requires:

- `name`
- `version`

However, you almost always want to include at least the following additional
fields for clarity and compatibility:

```toml
[project]
name = "my_project"            # Project name (PEP 621 requirement)
version = "0.1.0"              # Initial semantic version
description = "A brief overview"       # Short summary
readme = "README.md"           # Path to your README file (automatically included)
requires-python = ">=3.10"     # Restrict Python versions, if needed
license = { text = "MIT" }     # SPDX-compatible license expression or file
authors = [
  { name = "Alice Example", email = "alice@example.org" }
]
keywords = ["uv", "astral", "example"]   # (Optional) for metadata registries
classifiers = [
  "Programming Language :: Python :: 3",
  "License :: OSI Approved :: MIT License",
  "Operating System :: OS Independent"
]
dependencies = [
  "requests>=2.25",            # Runtime dependency
  "numpy>=1.23"
]
```

- **`name` and `version`:** Mandatory per PEP 621.
- **`description` and `readme`:** Although not mandatory, they help with
  indexing and packaging tools; `readme = "README.md"` tells `uv` (and PyPI) to
  include your README as the long description.
- **`requires-python`:** Constrains which Python interpreters your package
  supports (e.g. `>=3.10`).
- **`license = { text = "MIT" }`:** You can specify a license either as a SPDX
  identifier (via `license = { text = "MIT" }`) or by pointing to a file (e.g.
  `license = { file = "LICENSE" }`).
- **`authors`:** A list of tables with `name` and `email`. Many registries
  (e.g., PyPI) pull this for display.
- **`keywords` and `classifiers`:** These help search engines and package
  indexes. Classifiers must follow the exact trove list defined by PyPA.
- **`dependencies`:** A list of PEP 508-style requirements (e.g.,
  `"requests>=2.25"`). `uv sync` resolves those specifiers to concrete versions
  and records them in the lockfile.

______________________________________________________________________

## 3. Optional and Development Dependencies

Modern projects distinguish between "production" dependencies (those needed at
runtime) and local tooling. Use `[project.optional-dependencies]` only for
extras intended for publication (e.g. documentation). Development tools belong
under `[dependency-groups]` so they remain local and are controlled via group
flags:

```toml
[project.optional-dependencies]
docs = [
  "sphinx>=5.0",        # Documentation builder (published extra)
  "sphinx-rtd-theme"
]

[dependency-groups]
dev = [
  "pytest>=7.0",        # Testing framework (local dev group)
  "black",              # Code formatter
  "flake8>=4.0"         # Linter
]
```

- **`[project.optional-dependencies]`:** Published extras appear here and are
  installed with `--extra` flags.
- **`[dependency-groups]`:** Local-only groups like `dev` are enabled with
  `uv add --group dev` or `uv sync --group dev`, keeping the lockfile
  deterministic while separating concerns.

______________________________________________________________________

## 4. Entry Points and Scripts

If you want to expose command-line interfaces (CLIs) or GUIs through your
package, PEP 621 provides the `[project.scripts]` and `[project.gui-scripts]`
tables:

```toml
[project.scripts]
mycli = "my_project.cli:main"    

[project.gui-scripts]
mygui = "my_project.gui:start"
```

- **`[project.scripts]`:** Defines console scripts. When you run `uv run mycli`,
  `uv` will invoke the `main` function in `my_project/cli.py`.
- **`[project.gui-scripts]`:** On Windows, `uv` will wrap these in a GUI
  executable; on Unix-like systems, they behave like normal console scripts.
- **Plugin Entry Points:** If your project supports plugins, use
  `[project.entry-points.'group.name']` to register them.

______________________________________________________________________

## 5. Declaring a Build System

PEP 517/518 require a `[build-system]` table to tell tools how to build and
install your project. A "modern" convention is to specify `setuptools>=61.0`
(for editable installs without `setup.py`) or a lighter alternative like
`flit_core`. Astral `uv` also recognizes a `[tool.uv]` table for its own
configuration:

```toml
[build-system]
requires = ["setuptools>=61.0", "wheel"]
build-backend = "setuptools.build_meta"

[tool.uv]
package = true
```

- **`requires`:** Packages needed at build time. For editable installs in
  `uv`, use at least `setuptools>=61.0` and `wheel`.
- **`build-backend`:** Entry point for your build backend;
  `setuptools.build_meta` is PEP 517‑compliant.
- **`tool.uv.package = true`:** Builds and installs your project into the
  virtual environment whenever dependencies change.
- **Note:** If `[build-system]` is omitted, `uv` assumes
  `setuptools.build_meta:__legacy__` and skips editable installs of your
  project unless `tool.uv.package = true`.

______________________________________________________________________

## 6. Putting It All Together: Example `pyproject.toml`

Below is a complete example that demonstrates all sections. Adjust values as
needed for your own project.

```toml
[project]
name = "my_project"
version = "0.1.0"
description = "An illustrative example for Astral uv"
readme = "README.md"
requires-python = ">=3.10"
license = { text = "MIT" }
authors = [
  { name = "Alice Example", email = "alice@example.org" }
]
keywords = ["astral", "uv", "pyproject", "example"]
classifiers = [
  "Programming Language :: Python :: 3",
  "License :: OSI Approved :: MIT License",
  "Operating System :: OS Independent"
]
dependencies = [
  "requests>=2.25",
  "numpy>=1.23"
]

[project.optional-dependencies]
docs = [
  "sphinx>=5.0",
  "sphinx-rtd-theme"
]

[dependency-groups]
dev = [
  "pytest>=7.0",
  "black",
  "flake8>=4.0"
]

[project.scripts]
mycli = "my_project.cli:main"

[build-system]
requires = ["setuptools>=61.0", "wheel"]
build-backend = "setuptools.build_meta"

[tool.uv]
package = true
```

**Explanation of key points:**

1. **Metadata under `[project]`:**

   - `name`, `version` (mandatory per PEP 621)
   - `description`, `readme`, `requires-python`: provide clarity about the
     project and help tools like PyPI.
   - `license`, `authors`, `keywords`, `classifiers`: standardised metadata,
     which improves discoverability.
   - `dependencies`: runtime requirements, expressed in PEP 508 syntax.

2. **Optional Dependencies (`[project.optional-dependencies]` and
   `[dependency-groups]`):**

   - `docs` is an optional dependency exposed as a published extra.
   - `dev` resides under `[dependency-groups]` so tooling remains local. Enable
     it with `uv add --group dev` or `uv sync --group dev`.

3. **Entry Points (`[project.scripts]`):**

   - Defines a console command `mycli` that maps to `my_project/cli.py:main`.
     Invoking `uv run mycli` will run the `main()` function.

4. **Build System:**

   - `setuptools>=61.0` plus `wheel` ensures both legacy and editable installs
     work. ✱ Newer versions of setuptools support PEP 660 editable installs
     without a `setup.py` stub.
   - `build-backend = "setuptools.build_meta"` tells `uv` how to compile your
     package.

5. **`[tool.uv]`:**

   - `package = true` ensures that `uv sync` will build and install your own
     project (in editable mode) every time dependencies change. Otherwise, `uv`
     treats your project as a collection of scripts only (no package).

______________________________________________________________________

## 7. Additional Tips & Best Practices

1. **Keep `pyproject.toml` Human-Readable:** Edit it by hand when possible.
   Modern editors (VS Code, PyCharm) offer TOML syntax highlighting and PEP 621
   autocompletion.

2. **Lockfile Discipline:** After modifying `dependencies` or any `[project]`
   fields, always run `uv sync` (or `uv lock`) to update `uv.lock`. This
   guarantees reproducible environments.

3. **Semantic Versioning:** Follow semantic versioning for `version` values
   (e.g., `1.2.3`).[^1] Bump patch versions for bug fixes, minor for
   backward-compatible changes, and major for breaking changes.

4. **Keep Build Constraints Minimal:** If you don't need editable installs, you
   can omit `[build-system]` (but then `uv` won't build your package; it will
   only install dependencies). To override, set `tool.uv.package = true`.

5. **Use Exact or Bounded Ranges for Dependencies:** Rather than `requests`, use
   `requests>=2.25, <3.0` to avoid unexpected major bumps.

6. **Consider Dynamic Fields Sparingly:** You can declare fields like
   `dynamic = ["version"]` if your version is computed at build time (e.g. via
   `setuptools_scm`). If you do so, ensure your build backend supports dynamic
   metadata.

______________________________________________________________________

## 8. Summary

A "modern" `pyproject.toml` for an Astral `uv` project should:

- Use the PEP 621 `[project]` table for metadata and `dependencies`.
- Distinguish published extras under `[project.optional-dependencies]` and
  development groups under `[dependency-groups]`.
- Define any CLI or GUI entry points under `[project.scripts]` or
  `[project.gui-scripts]`.
- Declare a PEP 517 `[build-system]` (e.g. `setuptools>=61.0`, `wheel`,
  `setuptools.build_meta`) to support editable installs, or omit it and rely on
  `tool.uv.package = true`.
- Include a `[tool.uv]` section, at minimum `package = true` if you want `uv` to
  build and install your own package.

Following these conventions ensures that your project is fully PEP-compliant,
easy to maintain, and integrates seamlessly with Astral `uv`. For detailed
configuration options, refer to the uv documentation.[^2]

<!-- markdownlint-disable MD053 -->
[^1]: <https://semver.org/>
[^2]: <https://docs.astral.sh/uv/concepts/projects/config/>
<!-- markdownlint-enable MD053 -->
