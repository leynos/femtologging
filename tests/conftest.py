from __future__ import annotations

from pathlib import Path
import sys

# Add project root to sys.path so femtologging can be imported without hacks.
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
