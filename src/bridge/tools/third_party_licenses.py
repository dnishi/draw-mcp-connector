"""Collects the licenses of the crates built into the app into one file (THIRD-PARTY-LICENSES.txt).

MIT and similar licenses require copyright and permission notices to be included in what is distributed.
The GitHub Actions build puts it into the .app's Resources (.github/workflows/bridge.yml).

Usage (needs only the standard library and cargo):
    python3 -I tools/third_party_licenses.py <target> <output file>
    Example: python3 -I tools/third_party_licenses.py aarch64-apple-darwin target/THIRD-PARTY-LICENSES.txt

- Covers the dependencies built into the app for that target (following normal dependencies; test and build dependencies are excluded)
- Includes the full text of files such as LICENSE, COPYING, and NOTICE in each crate's package
- A crate with no license file is accepted only if its license includes MIT, and MIT is chosen.
  A copyright line is made from the authors in Cargo.toml, followed by the MIT text (same as the repository's LICENSE).
  Fails if MIT is not included (decide how to list it, then fix this)
"""

import json
import os
import re
import subprocess
import sys

BRIDGE_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REPO_LICENSE = os.path.join(BRIDGE_DIR, "..", "..", "LICENSE")
LICENSE_FILE = re.compile(r"^(licen[cs]e|copying|notice|unlicense)", re.IGNORECASE)


def metadata(target):
    output = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target],
        cwd=BRIDGE_DIR,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return json.loads(output)


def shipped_packages(meta):
    """Packages reached from root through normal dependencies only (kind is null). Excludes root"""
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    seen, stack = set(), [meta["resolve"]["root"]]
    while stack:
        for dep in nodes[stack.pop()]["deps"]:
            normal = any(kind["kind"] is None for kind in dep["dep_kinds"])
            if normal and dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                stack.append(dep["pkg"])
    return sorted((packages[i] for i in seen), key=lambda p: (p["name"], p["version"]))


def mit_body():
    """From the repository's LICENSE, takes what follows the copyright line (the permission notice and warranty disclaimer)"""
    with open(REPO_LICENSE, encoding="utf-8") as f:
        text = f.read()
    return text.split("\n\n", 2)[2].strip()


def license_texts(package):
    """List of (file name, full text). Includes license-file from Cargo.toml if present"""
    directory = os.path.dirname(package["manifest_path"])
    names = sorted(name for name in os.listdir(directory) if LICENSE_FILE.match(name))
    paths = [os.path.join(directory, name) for name in names]
    if package.get("license_file"):
        custom = os.path.join(directory, package["license_file"])
        if custom not in paths:
            paths.append(custom)
    texts = []
    for path in paths:
        if os.path.isfile(path):
            with open(path, encoding="utf-8", errors="replace") as f:
                texts.append((os.path.basename(path), f.read().strip()))
    return texts


def fallback_mit(package):
    """Text listing a crate that has no license file under MIT"""
    expression = package.get("license") or ""
    if "MIT" not in re.split(r"[^A-Za-z0-9.-]+", expression):
        raise SystemExit(f"{package['name']} {package['version']}: ライセンスのファイルが無く、MIT も選べない ({expression})")
    authors = ", ".join(re.sub(r"\s*<[^>]*>", "", a) for a in package.get("authors") or [])
    holder = authors or f"The {package['name']} authors"
    source = package.get("repository") or package.get("homepage") or ""
    note = (
        "The crate package does not include a license file. "
        f"It is used under the MIT License (license: {expression})."
        + (f" Source: {source}" if source else "")
    )
    return f"{note}\n\nCopyright (c) {holder}\n\n{mit_body()}"


def section(package):
    title = f"{package['name']} {package['version']} ({package.get('license') or 'see below'})"
    texts = license_texts(package)
    body = (
        "\n\n".join(f"--- {name} ---\n{text}" for name, text in texts)
        if texts
        else fallback_mit(package)
    )
    return f"{'=' * 72}\n{title}\n{'=' * 72}\n\n{body}\n"


def main(target, output):
    packages = shipped_packages(metadata(target))
    header = (
        "Third-party software included in Draw MCP Connector\n"
        f"(Rust crates linked into the app for {target}, from Cargo.lock)\n"
        "The glyph data from KanjiVG is listed in KanjiVG-LICENSE.md.\n\n"
    )
    with open(output, "w", encoding="utf-8") as f:
        f.write(header + "\n".join(section(p) for p in packages))
    print(f"{len(packages)} クレート → {output}")


if __name__ == "__main__":
    main(*sys.argv[1:])
