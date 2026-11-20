"""Scenario scripts that ``tests.reentrant_finalizer_support`` runs in a child.

Each module is a standalone program: running it executes the scenario, while
importing it (as pytest's ``--doctest-modules`` does) leaves the body inert
behind a ``__main__`` guard.
"""
