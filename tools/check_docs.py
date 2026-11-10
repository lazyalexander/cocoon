#!/usr/bin/env python3
"""Check English/Chinese Markdown pairs and relative document links."""
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent


def check():
    names = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=ROOT
    ).decode().split("\0")
    documents = sorted({ROOT / name for name in names if name.endswith(".md") and (ROOT / name).is_file()})
    errors = []
    for path in documents:
        companion = path.with_name(path.name.replace(".zh-CN.md", ".md")) if path.name.endswith(".zh-CN.md") else path.with_name(path.stem + ".zh-CN.md")
        text = path.read_text()
        if not companion.is_file():
            errors.append(f"{path.relative_to(ROOT)}: missing language companion")
        if f"]({companion.name})" not in text[:600]:
            errors.append(f"{path.relative_to(ROOT)}: missing introductory language link")
        for target in re.findall(r"\]\(([^)\s]+)\)", text):
            if re.match(r"^[a-zA-Z][a-zA-Z0-9+.-]*:", target) or target.startswith("#"):
                continue
            destination = target.split("#", 1)[0]
            if destination and not (path.parent / destination).exists():
                errors.append(f"{path.relative_to(ROOT)}: missing local link {destination}")
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"Checked {len(documents)} Markdown files: bilingual links and local destinations valid")
    return 0


if __name__ == "__main__":
    raise SystemExit(check())
