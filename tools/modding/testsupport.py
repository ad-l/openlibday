"""Shared test configuration: locates the game install and prepares fixtures.

Tests need an original Liberation Day install, given by $LIBDAY_ROOT. Nothing
derived from it is committed; registries and fixtures are generated locally.
"""
import os
from pathlib import Path
import unittest
import zipfile

import registries

TOOLS = Path(__file__).resolve().parent
_root = os.environ.get("LIBDAY_ROOT")
if not _root or not (Path(_root) / "GLOBALS" / "GLOBAL.MDB").is_file():
    raise unittest.SkipTest("set LIBDAY_ROOT to a Liberation Day install to run the modding tests")
ROOT = Path(_root).resolve()
registries.ensure(ROOT)

FIXTURES = TOOLS / ".fixtures"

# Jet round trips and Tk GUI drives take ~10 minutes; opt in before committing.
slow = unittest.skipUnless(
    os.environ.get("LIBDAY_SLOW_TESTS") == "1", "slow Jet/GUI test; set LIBDAY_SLOW_TESTS=1"
)


def green_skirmish() -> Path:
    """Returns an unmodified copy of SCEN1.ZIP!s1-99-1.mdb (the Green Thaw skirmish)."""
    path = FIXTURES / "s1-99-1.mdb"
    if not path.is_file():
        FIXTURES.mkdir(exist_ok=True)
        with zipfile.ZipFile(ROOT / "SCENS" / "SCEN1.ZIP") as archive:
            temp = path.with_suffix(".tmp")
            temp.write_bytes(archive.read("s1-99-1.mdb"))
            temp.replace(path)
    return path


GREEN_SKIRMISH = green_skirmish()
