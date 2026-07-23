#!/usr/bin/env python3
"""
Summarize the difference between two NIM usage scans: one run against the
committed (original) config and one run with `--refresh-repos`. Emits an HTML
fragment (suitable for GitHub Actions' $GITHUB_STEP_SUMMARY) that shows:

  * total repos and finding counts for both runs, side by side, with deltas
  * repositories added / removed in the config after refreshing

Inputs are the two report.json files and the two repos.yaml config files.
Missing/unreadable inputs degrade gracefully so the summary still renders when
one scan failed.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


def load_report(path: str) -> dict | None:
    """Load a report.json, returning None if it is missing or invalid."""
    try:
        with open(path, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, json.JSONDecodeError):
        return None


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


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--original-report", required=True)
    ap.add_argument("--refresh-report", required=True)
    ap.add_argument("--original-config", required=True)
    ap.add_argument("--refreshed-config", required=True)
    args = ap.parse_args()

    original_report = load_report(args.original_report)
    refresh_report = load_report(args.refresh_report)

    original_metrics = report_metrics(original_report)
    refresh_metrics = report_metrics(refresh_report)

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
    original = repo_names(args.original_config)
    refreshed = repo_names(args.refreshed_config)
    parts.append("<h3>Config changes after refresh</h3>")
    parts.append(
        f"<p>Config repo count: <strong>{len(original)}</strong> before → "
        f"<strong>{len(refreshed)}</strong> after refresh.</p>"
    )
    parts.append(
        render_list_section(
            "Repos added after refresh",
            refreshed - original,
            "none",
        )
    )
    parts.append(
        render_list_section(
            "Repos removed after refresh",
            original - refreshed,
            "none",
        )
    )

    print("\n".join(parts))


if __name__ == "__main__":
    main()
