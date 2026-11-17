import argparse
import tomllib
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import Request, urlopen


def is_published(crate, version):
    request = Request(
        f"https://crates.io/api/v1/crates/{crate}/{version}",
        headers={"User-Agent": "memlite-mcp release workflow"},
    )
    try:
        with urlopen(request, timeout=30):
            return True
    except HTTPError as error:
        if error.code == 404:
            return False
        raise


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("tag")
    parser.add_argument("--check-published", action="store_true")
    args = parser.parse_args()

    # Validating the tag against the checked-out package
    package = tomllib.loads(Path("Cargo.toml").read_text())["package"]
    version = package["version"]
    if args.tag != version:
        parser.error(f"tag {args.tag!r} does not match Cargo.toml version {version!r}")

    # Checking only the exact version, without treating registry outages as absence
    print(f"version={version}")
    if args.check_published:
        published = is_published(package["name"], version)
        print(f"published={str(published).lower()}")


if __name__ == "__main__":
    main()
