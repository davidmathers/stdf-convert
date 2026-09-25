import sys

from ._stdf_convert import run_cli


def main() -> None:
    sys.exit(run_cli(["stdf-convert", *sys.argv[1:]]))


if __name__ == "__main__":
    main()
