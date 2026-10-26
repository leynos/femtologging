"""Domain-specific failures reported by release asset reconciliation."""

from __future__ import annotations


class ReleaseAssetError(Exception):
    """Base exception for unsafe or inconsistent release assets."""


class MissingDigestError(ReleaseAssetError):
    """A destination did not provide the SHA-256 digest required for a file."""

    def __init__(self, source: str) -> None:
        """Name the destination that omitted its digest."""
        super().__init__(f"{source} does not contain a SHA-256 digest")


class MalformedDigestError(ReleaseAssetError):
    """A destination provided a malformed SHA-256 digest."""

    def __init__(self, source: str) -> None:
        """Name the file or destination with the malformed digest."""
        super().__init__(f"{source} has a malformed SHA-256 digest")


class UnsafeArtifactNamesError(ReleaseAssetError):
    """A staging directory contains an unsafe artefact filename."""

    def __init__(self, names: list[str]) -> None:
        """List the filenames rejected by the path safety check."""
        super().__init__(f"refusing unsafe artefact names: {names}")


class ArtifactDigestMismatchError(ReleaseAssetError):
    """A local file does not match its published SHA-256 digest."""

    def __init__(self, name: str, actual: str, expected: str | None) -> None:
        """Describe the actual and expected digests for one artefact."""
        super().__init__(f"{name} has sha256 {actual}, expected {expected}")


class NonHttpsDownloadError(ReleaseAssetError):
    """A PyPI Simple index lists a download URL that is not HTTPS."""

    def __init__(self) -> None:
        """Explain the required transport for PyPI download URLs."""
        super().__init__("every PyPI download must use HTTPS")


class DestinationDigestMismatchError(ReleaseAssetError):
    """Existing PyPI and GitHub bytes disagree for the same filename."""

    def __init__(self, name: str) -> None:
        """Name the file that differs between destinations."""
        super().__init__(f"{name} differs between PyPI and the GitHub Release")


class MissingWorkflowOptionError(ReleaseAssetError):
    """A reconciliation command is missing an option it requires."""

    def __init__(self, option: str, command: str) -> None:
        """Name the required option and the command that needs it."""
        super().__init__(f"--{option} is required for {command}")
