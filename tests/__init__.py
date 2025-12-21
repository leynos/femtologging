"""Test package for femtologging.

Contains unit tests for core logging functionality, integration tests for the
Rust log bridge (log-compat), and BDD-style feature tests for end-to-end
scenarios.

Test Organisation
-----------------
- Unit tests (test_*.py): Test individual components such as handlers,
  builders, filters, and configuration.
- BDD tests (features/): Gherkin feature files with step definitions in
  steps/.
- Shared fixtures (conftest.py): pytest fixtures including
  `file_handler_factory` for handler setup and `_clean_logging_manager` for
  automatic manager reset.
- Shared helpers (helpers.py): Common test utilities such as
  `poll_file_for_text` for async file content verification.

Running Tests
-------------
Run all tests::

    pytest tests/

Run BDD tests only::

    pytest tests/features/

Run a specific test file::

    pytest tests/test_handler_builders.py
"""
