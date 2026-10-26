#!/usr/bin/env python3
"""Work out the next version and the release notes from conventional commits.

Run from the repository root. Prints GitHub-Actions-style `key=value` lines on
stdout (so the workflow can append them to $GITHUB_OUTPUT) and writes the notes
markdown to the path given by --notes.

Rules, and why:

- Squash merges carry the PR title as the commit subject, so a conventional PR
  title is all this needs: `feat: …`, `fix(app): …`, `feat!:` / `BREAKING
  CHANGE`.
- Pre-1.0, a breaking change and a feature both bump the minor: `0.x` has no
  compatibility promise to break, and burning 1.0.0 on the second week is how
  projects end up at 7.0.0 with no users.
- Fixes and anything else that changes behaviour bump the patch.
- Only chores, docs, CI or tests since the last tag: no release at all. A tag
  every time somebody fixes a typo in the README is noise.
- No previous tag: the version in Cargo.toml is the first release, unbumped.
"""
import argparse
import re
import subprocess
import sys

# type(scope)!: subject
CONVENTIONAL = re.compile(r"^(?P<type>[a-z]+)(?:\([^)]*\))?(?P<breaking>!)?:\s*(?P<subject>.+)$")
TYPES = {
    "feat": "Features",
    "fix": "Fixes",
    "perf": "Performance",
    "refactor": "Refactoring",
    "docs": "Documentation",
    "build": "Build",
    "ci": "CI",
    "test": "Tests",
    "chore": "Chores",
    "style": "Style",
}
RELEASING = {"feat", "fix", "perf"}


def git(*args: str) -> str:
    return subprocess.run(["git", *args], capture_output=True, text=True, check=True).stdout.strip()


def last_tag() -> str | None:
    tags = git("tag", "--list", "v*", "--sort=-v:refname").splitlines()
    return tags[0] if tags else None


def commits(since: str | None) -> list[tuple[str, str]]:
    """(subject, short sha) since the tag, oldest first, merge commits dropped."""
    rng = f"{since}..HEAD" if since else "HEAD"
    out = git("log", rng, "--no-merges", "--format=%s%x00%h")
    rows = []
    for line in reversed(out.splitlines()):
        if not line.strip():
            continue
        subject, _, sha = line.partition("\x00")
        rows.append((subject.strip(), sha.strip()))
    return rows


def parse(subject: str) -> tuple[str | None, bool, str]:
    m = CONVENTIONAL.match(subject)
    if not m:
        # no type: treat it as an unreleased fix rather than dropping it
        return None, False, subject
    return m.group("type"), bool(m.group("breaking")), m.group("subject")


def bump(current: str, kinds: set[str], breaking: bool) -> str:
    major, minor, patch = (int(part) for part in current.split("."))
    if major == 0:
        if breaking or "feat" in kinds:
            minor, patch = minor + 1, 0
        else:
            patch += 1
    else:
        if breaking:
            major, minor, patch = major + 1, 0, 0
        elif "feat" in kinds:
            minor, patch = minor + 1, 0
        else:
            patch += 1
    return f"{major}.{minor}.{patch}"


def notes(rows: list[tuple[str, str]]) -> str:
    groups: dict[str, list[str]] = {}
    breaking: list[str] = []
    for subject, sha in rows:
        kind, is_breaking, text = parse(subject)
        heading = TYPES.get(kind or "", "Other changes")
        line = f"- {text} ({sha})"
        groups.setdefault(heading, []).append(line)
        if is_breaking:
            breaking.append(f"- {text} ({sha})")

    order = ["Features", "Fixes", "Performance", "Refactoring", "Documentation", "Build", "CI", "Tests", "Chores", "Other changes"]
    out = []
    if breaking:
        out.append("## Breaking changes")
        out.append("")
        out.extend(breaking)
        out.append("")
    for heading in order:
        if heading in groups:
            out.append(f"## {heading}")
            out.append("")
            out.extend(groups[heading])
            out.append("")
    return "\n".join(out).strip()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--current", required=True, help="version in Cargo.toml")
    ap.add_argument("--notes", required=True, help="where to write the notes markdown")
    ap.add_argument("--dry-run-range", help="use this git range instead of the last tag")
    args = ap.parse_args()

    since = args.dry_run_range or last_tag()
    rows = commits(since)
    kinds: set[str] = set()
    breaking = False
    for subject, _ in rows:
        kind, is_breaking, _text = parse(subject)
        if kind:
            kinds.add(kind)
        breaking = breaking or is_breaking

    if since is None:
        version, kind_of_bump = args.current, "initial"
    elif not (kinds & RELEASING) and not breaking:
        version, kind_of_bump = "", "none"
    else:
        version, kind_of_bump = bump(args.current, kinds, breaking), "release"

    text = notes(rows) if version else ""
    with open(args.notes, "w") as fh:
        fh.write(text + "\n" if text else "")

    print(f"version={version}")
    print(f"bump={kind_of_bump}")
    print(f"since={since or ''}")
    print(f"commits={len(rows)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
