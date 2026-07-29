#!/usr/bin/env python3
"""
Find blueprints affected by a deprecated NIM.

Reads the scanner's per-repo aggregate output (`report_aggregate.json`), a list
of deprecated NIM identifiers, and the repos config (for each repo's blueprints),
then writes a CSV + JSON report of the affected blueprints.

A NIM reference is considered affected when it contains a deprecated entry as a
case-insensitive substring (deprecated entry is a substring of the NIM
reference). The same deprecated list is matched against both hosted and local
NIMs; matches are reported under `affected_hosted_nims` / `affected_local_nims`
according to which list they came from.

The result is flattened per blueprint: a repo backing multiple blueprints yields
one record per blueprint, each carrying `blueprint_name` / `blueprint_url`.
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


def load_blueprints(path: Path) -> dict[str, list[dict]]:
    """Map repo name -> its blueprints ([{name, url, ...}]) from a repos config."""
    if not path.is_file():
        return {}
    data = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    mapping: dict[str, list[dict]] = {}
    for section in ("repos_active", "repos_github_only"):
        for entry in data.get(section) or []:
            if isinstance(entry, dict) and entry.get("name"):
                mapping[entry["name"]] = entry.get("blueprints") or []
    return mapping


def find_affected(
    repos: list[dict], deprecated: list[str], blueprint_map: dict[str, list[dict]]
) -> list[dict]:
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
        if not (affected_hosted or affected_local):
            continue
        repository = repo.get("repository", "")
        repository_url = repo.get("repository_url", "")
        # Flatten per blueprint; repos with no blueprints get one blank-blueprint record.
        blueprints = blueprint_map.get(repository) or [{}]
        for blueprint in blueprints:
            affected.append(
                {
                    "blueprint_name": blueprint.get("name", ""),
                    "blueprint_url": blueprint.get("url", ""),
                    "repository": repository,
                    "repository_url": repository_url,
                    "affected_hosted_nims": affected_hosted,
                    "affected_local_nims": affected_local,
                }
            )
    affected.sort(key=lambda r: (r["repository"].lower(), r["blueprint_name"].lower()))
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
    parser.add_argument(
        "--config",
        default="config/repos.yaml",
        help="repos.yaml providing each repo's blueprints. Default: config/repos.yaml",
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
    blueprint_map = load_blueprints(Path(args.config))
    affected = find_affected(repos, deprecated, blueprint_map)

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
