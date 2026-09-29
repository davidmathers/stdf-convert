import subprocess
import sys
from pathlib import Path

import stdf_convert

# Little-endian FAR (CPU_TYPE=2, STDF_VER=4) followed by PIR (head=1, site=2).
STDF_BYTES = bytes([2, 0, 0, 10, 2, 4, 2, 0, 5, 10, 1, 2])


def test_command_line(tmp_path: Path) -> None:
    (tmp_path / "in").mkdir()
    (tmp_path / "in" / "a.stdf").write_bytes(STDF_BYTES)
    result = subprocess.run(
        [sys.executable, "-m", "stdf_convert", "-q", "-o", str(tmp_path / "out"), str(tmp_path / "in")],
        capture_output=True, text=True,
    )
    assert result.returncode == 0, result.stderr
    assert (tmp_path / "out" / "a.jsonl").exists()
    result = subprocess.run([sys.executable, "-m", "stdf_convert", "--version"], capture_output=True, text=True)
    assert result.stdout.strip() == f"stdf-convert {stdf_convert.__version__}"
