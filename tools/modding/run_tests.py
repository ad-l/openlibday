"""Runs the modding test suite in parallel worker processes.

Usage: python run_tests.py [--slow] [--jobs N] [pattern ...]

Fast tests run together in one worker; with --slow (or LIBDAY_SLOW_TESTS=1)
each slow Jet/GUI test also gets its own worker, longest-first. Tests only
share the registries and fixtures, which this process prepares up front.
Optional patterns filter test ids by substring.
"""
import argparse
import os
import subprocess
import sys
import time
import unittest
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
LONGEST = ("test_unit_later_campaign_roundtrip", "test_unit_gui_turn_selector",
           "test_native_noop_preserves_every_table", "test_native_noop_both_types")


def test_ids(suite):
    for item in suite:
        if isinstance(item, unittest.TestSuite):
            yield from test_ids(item)
        else:
            yield item


def is_slow(test) -> bool:
    method = getattr(test, test._testMethodName, None)
    return any(getattr(obj, "__unittest_skip__", False) and "LIBDAY_SLOW_TESTS" in
               getattr(obj, "__unittest_skip_why__", "") for obj in (type(test), method))


def run(ids: list[str], env: dict) -> tuple[list[str], int, float, str]:
    start = time.perf_counter()
    result = subprocess.run([sys.executable, "-m", "unittest", *ids], cwd=TOOLS, env=env,
                            capture_output=True, text=True)
    return ids, result.returncode, time.perf_counter() - start, result.stderr


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--slow", action="store_true", help="include the slow Jet/GUI tests")
    parser.add_argument("--jobs", type=int, default=max(2, (os.cpu_count() or 4) // 2))
    parser.add_argument("patterns", nargs="*")
    args = parser.parse_args()
    slow = args.slow or os.environ.get("LIBDAY_SLOW_TESTS") == "1"

    os.chdir(TOOLS)
    sys.path.insert(0, str(TOOLS))
    os.environ["LIBDAY_SLOW_TESTS"] = "0"  # discover with slow tests marked as skipped
    try:
        import testsupport  # noqa: F401  (generates registries and fixtures once)
    except unittest.SkipTest as skip:
        print(f"skipped: {skip}")
        return 0
    tests = [t for t in test_ids(unittest.defaultTestLoader.discover(str(TOOLS)))
             if not args.patterns or any(p in t.id() for p in args.patterns)]
    fast = [t.id() for t in tests if not is_slow(t)]
    batches = [[t.id()] for t in tests if slow and is_slow(t)]
    # Start the longest tests first (measured ~90 s each), then the Jet bridge classes.
    batches.sort(key=lambda ids: (not any(name in ids[0] for name in LONGEST), "Bridge" not in ids[0]))
    if fast:
        batches.insert(0, fast)

    env = dict(os.environ, LIBDAY_SLOW_TESTS="1" if slow else "0")
    start, failed = time.perf_counter(), []
    count = sum(len(b) for b in batches)
    with ThreadPoolExecutor(args.jobs) as pool:
        futures = [pool.submit(run, ids, env) for ids in batches]
        for future in as_completed(futures):
            ids, code, seconds, output = future.result()
            label = ids[0] if len(ids) == 1 else f"{len(ids)} fast tests"
            print(f"{'ok  ' if code == 0 else 'FAIL'} {seconds:6.1f}s {label}", flush=True)
            if code:
                failed.append((label, output))
    for label, output in failed:
        print(f"\n===== {label} =====\n{output}")
    skipped = "" if slow else " (slow tests skipped; use --slow)"
    print(f"\n{count} tests in {len(batches)} workers, {time.perf_counter() - start:.1f}s, "
          f"{len(failed)} failed{skipped}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
