#!/usr/bin/env python3
"""
Summarize the difference between two NIM usage scans: one run against the
committed (original) config and one run with `--refresh-repos`. Emits an HTML
fragment (suitable for GitHub Actions' $GITHUB_STEP_SUMMARY) that shows:

  * total repos and finding counts for both runs, side by side, with deltas
  * repositories added / removed in the config after refreshing
  * blueprints affected by deprecated NIMs in each run (from --check-deprecation)

Inputs are the two scan output folders (each holding report.json and, when any
blueprint is affected, deprecation_affected_blueprints.json) plus the two
repos.yaml config files. Missing/unreadable inputs degrade gracefully so the
summary still renders when one scan failed or produced no affected blueprints.

When `--summary-json` is given it also writes a small machine-readable summary
of the config change (repo counts before/after refresh, added/removed counts,
plus affected-blueprint counts). The workflow reads that file to decide whether
to notify Slack.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

# Report files the scanner writes into each scan's output folder.
REPORT_FILENAME = "report.json"
DEPRECATION_FILENAME = "deprecation_affected_blueprints.json"


def load_report(path: str | Path) -> dict | None:
    """Load a report.json, returning None if it is missing or invalid."""
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, json.JSONDecodeError):
        return None


def load_affected(path: str | Path) -> list:
    """Load a deprecation_affected_blueprints.json, returning [] when the file
    is absent (no blueprint was affected, so the scanner wrote nothing) or
    invalid."""
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, json.JSONDecodeError):
        return []
    return data if isinstance(data, list) else []


def repo_names(config_path: str) -> set[str]:
    """Extract repo names (the `name:` field under `repos:`) from a repos.yaml.

    Uses PyYAML when available and falls back to a line regex, so the script has
    no hard dependency on PyYAML (which is not a project requirement).
    """
    try:
        text = Path(config_path).read_text(encoding="utf-8")
    except OSError:
        return set()

    try:
        import yaml  # type: ignore

        data = yaml.safe_load(text) or {}
        return {
            str(r["name"]).strip()
            for r in data.get("repos", [])
            if isinstance(r, dict) and r.get("name")
        }
    except Exception:
        # Fallback: match `- name: <value>` entries.
        return {
            m.group(1).strip()
            for m in re.finditer(r"^\s*-\s*name:\s*(.+?)\s*$", text, re.MULTILINE)
        }


def report_metrics(report: dict | None) -> dict:
    """Pull the headline numbers out of a report.json."""
    if not report:
        return {}
    summary = report.get("summary", {}) or {}
    aggregated = report.get("aggregated", {}) or {}
    return {
        "total_repos": report.get("total_repos"),
        "repos_with_nim": summary.get("repos_with_nim"),
        "total_local_nim": summary.get("total_local_nim"),
        "total_hosted_nim": summary.get("total_hosted_nim"),
        "unique_local_nim": len(aggregated.get("local_nim", []) or []),
        "unique_hosted_nim": len(aggregated.get("hosted_nim", []) or []),
    }


def esc(value) -> str:
    if value is None:
        return "—"
    return (
        str(value)
        .replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
    )


def delta_cell(before, after) -> str:
    """Render a signed delta with an indicator, or an em dash when unknown."""
    if not isinstance(before, (int, float)) or not isinstance(after, (int, float)):
        return "—"
    diff = after - before
    if diff == 0:
        return "0"
    sign = "🔺 +" if diff > 0 else "🔻 "
    return f"{sign}{diff}"


METRIC_ROWS = [
    ("total_repos", "Total repos scanned"),
    ("repos_with_nim", "Repos with NIM usage"),
    ("total_local_nim", "Local NIM findings"),
    ("total_hosted_nim", "Hosted NIM findings"),
    ("unique_local_nim", "Unique local NIM images"),
    ("unique_hosted_nim", "Unique hosted NIM models"),
]


def render_metrics_table(original: dict, refreshed: dict) -> str:
    rows = []
    for key, label in METRIC_ROWS:
        before = original.get(key)
        after = refreshed.get(key)
        rows.append(
            "<tr>"
            f"<td>{esc(label)}</td>"
            f"<td align=\"right\">{esc(before)}</td>"
            f"<td align=\"right\">{esc(after)}</td>"
            f"<td align=\"right\">{delta_cell(before, after)}</td>"
            "</tr>"
        )
    return (
        "<table>\n"
        "<thead><tr><th>Metric</th><th>Original</th>"
        "<th>Refreshed</th><th>Δ</th></tr></thead>\n"
        "<tbody>\n" + "\n".join(rows) + "\n</tbody>\n</table>"
    )


def render_list_section(title: str, items: set[str], empty_msg: str) -> str:
    items = sorted(items)
    if not items:
        return f"<p><strong>{esc(title)}:</strong> {esc(empty_msg)}</p>"
    lis = "\n".join(f"<li><code>{esc(i)}</code></li>" for i in items)
    return (
        f"<details><summary><strong>{esc(title)}</strong> "
        f"({len(items)})</summary>\n<ul>\n{lis}\n</ul>\n</details>"
    )


def render_affected_section(title: str, affected: list) -> str:
    """Render a collapsible list of blueprints affected by deprecated NIMs, each
    with its offending hosted/local NIM references."""
    if not affected:
        return f"<p><strong>{esc(title)}:</strong> none affected</p>"
    rows = []
    for bp in affected:
        repo = bp.get("repository", "")
        url = bp.get("repository_url", "")
        nims = (bp.get("affected_hosted_nims") or []) + (bp.get("affected_local_nims") or [])
        name = f'<a href="{esc(url)}">{esc(repo)}</a>' if url else f"<code>{esc(repo)}</code>"
        nims_html = ", ".join(f"<code>{esc(n)}</code>" for n in nims) or "—"
        rows.append(f"<li>{name}: {nims_html}</li>")
    lis = "\n".join(rows)
    return (
        f"<details><summary><strong>{esc(title)}</strong> "
        f"({len(affected)})</summary>\n<ul>\n{lis}\n</ul>\n</details>"
    )


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--original-output",
        required=True,
        help=f"Output folder of the original scan (contains {REPORT_FILENAME} "
        f"and, when any blueprint is affected, {DEPRECATION_FILENAME}).",
    )
    ap.add_argument(
        "--refresh-output",
        required=True,
        help="Output folder of the refreshed scan (same layout as "
        "--original-output).",
    )
    ap.add_argument("--original-config", required=True)
    ap.add_argument("--refreshed-config", required=True)
    ap.add_argument(
        "--summary-json",
        help="Write a machine-readable JSON summary of the config change here.",
    )
    args = ap.parse_args()

    original_out = Path(args.original_output)
    refresh_out = Path(args.refresh_output)

    original_report = load_report(original_out / REPORT_FILENAME)
    refresh_report = load_report(refresh_out / REPORT_FILENAME)

    original_metrics = report_metrics(original_report)
    refresh_metrics = report_metrics(refresh_report)

    original = repo_names(args.original_config)
    refreshed = repo_names(args.refreshed_config)
    added = refreshed - original
    removed = original - refreshed

    original_affected = load_affected(original_out / DEPRECATION_FILENAME)
    refresh_affected = load_affected(refresh_out / DEPRECATION_FILENAME)

    parts: list[str] = ["<h2>NIM Usage Scan — refresh comparison</h2>"]

    # Warn if either scan produced no report.
    warnings = []
    if original_report is None:
        warnings.append("the <em>original</em> scan produced no report")
    if refresh_report is None:
        warnings.append("the <em>refreshed</em> scan produced no report")
    if warnings:
        parts.append(
            "<blockquote>⚠️ " + "; ".join(warnings) + ".</blockquote>"
        )

    # 1) Headline metrics (includes total repos for both runs).
    parts.append("<h3>Totals &amp; findings</h3>")
    parts.append(render_metrics_table(original_metrics, refresh_metrics))

    # 2) Repos added / removed after refreshing.
    parts.append("<h3>Config changes after refresh</h3>")
    parts.append(
        f"<p>Config repo count: <strong>{len(original)}</strong> before → "
        f"<strong>{len(refreshed)}</strong> after refresh.</p>"
    )
    parts.append(render_list_section("Repos added after refresh", added, "none"))
    parts.append(render_list_section("Repos removed after refresh", removed, "none"))

    # 3) Blueprints affected by deprecated NIMs (from --check-deprecation).
    parts.append("<h3>Blueprints affected by deprecated NIMs</h3>")
    parts.append(
        f"<p>Affected blueprints: <strong>{len(original_affected)}</strong> (original) → "
        f"<strong>{len(refresh_affected)}</strong> (refreshed).</p>"
    )
    parts.append(render_affected_section("Original — affected blueprints", original_affected))
    parts.append(render_affected_section("Refreshed — affected blueprints", refresh_affected))

    print("\n".join(parts))

    # Machine-readable summary of the config change for the workflow to act on.
    if args.summary_json:
        Path(args.summary_json).write_text(
            json.dumps(
                {
                    "repos_before": len(original),
                    "repos_after": len(refreshed),
                    "added": len(added),
                    "removed": len(removed),
                    "affected_before": len(original_affected),
                    "affected_after": len(refresh_affected),
                },
                indent=2,
            )
            + "\n",
            encoding="utf-8",
        )


if __name__ == "__main__":
    main()
