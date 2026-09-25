"""The `knf` command installed by the pyknf wheel.

The work is the Rust command line in ``_knf.cli``. This module exists so that
command can sit on ``PATH`` without a second package.
"""


def main() -> None:
    from ._knf import cli

    cli()
