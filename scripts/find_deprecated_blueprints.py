#!/usr/bin/env python3
"""
Find blueprints that still reference a deprecated NIM.

Reads the scanner's per-repo aggregate output (`report_aggregate.json`) and a
list of deprecated NIM identifiers, then writes a CSV + JSON report of only the
affected blueprints.

A NIM reference is considered affected when it contains a deprecated entry as a
case-insensitive substring (deprecated entry is a substring of the NIM
reference). The same deprecated list is matched against both hosted and local
NIMs; matches are reported under `affected_hosted_nims` / `affected_local_nims`
according to which list they came from.
"""

import argparse
import csv
import json
import sys
from pathlib import Path

import yaml

AGGREGATE_FILENAME = "report_aggregate.json"
CSV_FILENAME = "deprecation_affected_blueprints.csv"
JSON_FILENAME = "deprecation_affected_blueprints.json"


def load_deprecated(path: Path) -> list[str]:
    """Load the flat `deprecated:` list from a YAML file."""
    data = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    entries = data.get("deprecated", []) or []
    return [str(e).strip() for e in entries if str(e).strip()]


def find_affected(repos: list[dict], deprecated: list[str]) -> list[dict]:
    deps_lower = [d.lower() for d in deprecated]
    affected = []
    for repo in repos:
        hosted = repo.get("hosted_nims") or []
        local = repo.get("local_nims") or []
        affected_hosted = sorted(
            n for n in hosted if any(d in n.lower() for d in deps_lower)
        )
        affected_local = sorted(
            n for n in local if any(d in n.lower() for d in deps_lower)
        )
        if affected_hosted or affected_local:
            affected.append(
                {
                    "repository": repo.get("repository", ""),
                    "repository_url": repo.get("repository_url", ""),
                    "affected_hosted_nims": affected_hosted,
                    "affected_local_nims": affected_local,
                }
            )
    affected.sort(key=lambda r: r["repository"].lower())
    return affected


def write_json(path: Path, rows: list[dict]) -> None:
    path.write_text(json.dumps(rows, indent=2) + "\n", encoding="utf-8")


def write_csv(path: Path, rows: list[dict]) -> None:
    fields = list(rows[0].keys())
    with path.open("w", newline="", encoding="utf-8") as f:
        writer = csv.writer(f)
        writer.writerow(fields)
        for row in rows:
            writer.writerow(
                [
                    "\n".join(value) if isinstance(value, list) else value
                    for value in (row[field] for field in fields)
                ]
            )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        default="output",
        help="Scanner output folder (contains %s; reports are written here). "
        "Default: output" % AGGREGATE_FILENAME,
    )
    parser.add_argument(
        "--deprecated-file",
        default="config/nims.deprecated.yaml",
        help="YAML file with a flat `deprecated:` list of NIM identifiers. "
        "Default: config/nims.deprecated.yaml",
    )
    args = parser.parse_args()

    output_dir = Path(args.output)
    aggregate_path = output_dir / AGGREGATE_FILENAME
    deprecated_path = Path(args.deprecated_file)

    if not aggregate_path.is_file():
        print(f"error: aggregate report not found: {aggregate_path}", file=sys.stderr)
        return 1
    if not deprecated_path.is_file():
        print(f"error: deprecated file not found: {deprecated_path}", file=sys.stderr)
        return 1

    deprecated = load_deprecated(deprecated_path)
    if not deprecated:
        print(f"error: no deprecated entries found in {deprecated_path}", file=sys.stderr)
        return 1

    repos = json.loads(aggregate_path.read_text(encoding="utf-8"))
    affected = find_affected(repos, deprecated)

    summary = (
        f"{len(affected)} affected blueprint(s) "
        f"(of {len(repos)} scanned, {len(deprecated)} deprecated NIMs)"
    )
    if not affected:
        print(f"{summary}; no report files written")
        return 0

    csv_path = output_dir / CSV_FILENAME
    json_path = output_dir / JSON_FILENAME
    write_json(json_path, affected)
    write_csv(csv_path, affected)

    print(summary)
    print(f"  wrote {json_path}")
    print(f"  wrote {csv_path}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
