import os
from pathlib import Path
import subprocess
import sys
import sysconfig

import knf
import pytest


def test_config_requires_exec(monkeypatch):
    monkeypatch.delenv("KNF_CONFIG_PATH", raising=False)
    with pytest.raises(RuntimeError, match=r"--exec -- COMMAND"):
        knf.config


@pytest.mark.parametrize("entry_point", ["wheel", "standalone"])
def test_exec_loads_resolved_native_values(tmp_path, entry_point):
    if entry_point == "standalone":
        executable = os.environ.get("KNF_TEST_STANDALONE")
        if executable is None:
            message = "set KNF_TEST_STANDALONE to the standalone CLI binary"
            if os.environ.get("CI"):
                pytest.fail(message)
            pytest.skip(message)
        if not Path(executable).is_file():
            pytest.fail(f"KNF_TEST_STANDALONE does not point to a file: {executable}")
    else:
        scripts = Path(sysconfig.get_path("scripts"))
        executable = scripts / ("knf.exe" if os.name == "nt" else "knf")
        if not executable.is_file():
            pytest.fail(f"knf script not found at {executable}; install the wheel")

    (tmp_path / "base.toml").write_text(
        "run_date = 2024-03-14\nseed = 1\nliteral = '$${seed}'\n", encoding="utf-8"
    )
    (tmp_path / "pilot.toml").write_text("replicate = 2\n", encoding="utf-8")
    (tmp_path / "fit.py").write_text(
        """\
from datetime import date
import os
from pathlib import Path
import sys

import knf

BAD = 97  # distinct from the expected exit code 23

snapshot = Path(os.environ["KNF_CONFIG_PATH"])
if not (snapshot.is_absolute() and snapshot.suffix == ".toml" and snapshot.is_file()):
    print("bad snapshot", file=sys.stderr)
    sys.exit(BAD)
expected = {
    "run_date": date(2024, 3, 14),
    "seed": 42,
    "literal": "${seed}",
    "replicate": 2,
}
if knf.config != expected:
    print(f"bad config: {knf.config!r}", file=sys.stderr)
    sys.exit(BAD)
del os.environ["KNF_CONFIG_PATH"]
from knf import config
if knf.config is not config:
    print("config not cached", file=sys.stderr)
    sys.exit(BAD)
if sys.argv[1:] != ["--replicate", "two words", "--", "-i"]:
    print(f"bad argv: {sys.argv!r}", file=sys.stderr)
    sys.exit(BAD)
print(snapshot)
print("child stderr", file=sys.stderr)
sys.exit(23)
""",
        encoding="utf-8",
    )

    result = subprocess.run(
        [
            executable, "base.toml", "pilot.toml", "-c", "seed=42", "-i",
            "--exec", "--", sys.executable, "fit.py", "--replicate", "two words",
            "--", "-i",
        ],
        cwd=tmp_path,
        capture_output=True,
        text=True,
    )
    assert result.returncode == 23, result.stderr
    assert result.stderr == "child stderr\n"
    snapshot = Path(result.stdout.strip())
    assert snapshot.is_absolute()
    assert not snapshot.exists()
