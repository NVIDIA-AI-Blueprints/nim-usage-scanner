#!/usr/bin/env python3
"""
Build the daily scan's GitHub Actions summary and Slack message.

Reads the two JSON artifacts produced by the daily scan (both under the scan
output folder):
  - repos_refresh_summary.json         (repo adds/removes/deprecated)
  - deprecation_affected_blueprints.json (absent when nothing is affected)

and writes:
  - an HTML file for the GitHub Actions job summary (``--html-out``)
  - a JSON file for the Slack message (``--slack-out``), sent as a color-coded
    attachment.

Slack attachment color (also emitted as the ``color`` step output when
``$GITHUB_OUTPUT`` is set, for the workflow to decide whether to notify):
  - danger  : a blueprint is affected by a deprecated NIM (with
              ``--affected-local-nims-safe``, only a deprecated *hosted* NIM;
              blueprints affected only by deprecated *local* NIMs count as good)
  - warning : the repo lists changed (active added/removed, or newly deprecated)
  - good    : neither of the above

Pure stdlib (JSON only); no third-party deps.
"""

import argparse
import html
import json
import os
from pathlib import Path

COLOR_EMOJI = {
    "danger": ":rotating_light:",
    "warning": ":warning:",
    "good": ":white_check_mark:",
}
# Cap long lists in the Slack message so it stays readable.
SLACK_LIST_CAP = 20


def load_json(path: str, default):
    p = Path(path)
    if not p.is_file():
        return default
    try:
        return json.loads(p.read_text(encoding="utf-8"))
    except (json.JSONDecodeError, OSError):
        return default


def entries_with(affected: list, nims_key: str) -> list:
    """Affected blueprints that have at least one deprecated NIM under `nims_key`.

    A blueprint with both hosted and local deprecated NIMs appears in both lists.
    """
    return [e for e in affected if e.get(nims_key)]


def render_html(
    run_number: str,
    run_url: str,
    active_after,
    deprecated_after,
    added_active: list,
    removed_active: list,
    added_deprecated: list,
    affected: list,
) -> str:
    def details(title: str, items: list) -> str:
        if not items:
            return ""
        lis = "\n".join(f"<li>{html.escape(str(i))}</li>" for i in items)
        return (
            f"<details><summary>{html.escape(title)} ({len(items)})</summary>\n"
            f"<ul>\n{lis}\n</ul>\n</details>"
        )

    heading = f"Daily NIM Usage Scan #{html.escape(str(run_number))}" if run_number else "Daily NIM Usage Scan"
    parts = [
        f"<h2>{heading}</h2>",
        "<h3>Repos config refresh</h3>",
        "<table>",
        "<tr><th>Category</th><th>Count</th><th>Added</th><th>Removed</th></tr>",
        f"<tr><td>Active</td><td>{active_after}</td><td>{len(added_active)}</td><td>{len(removed_active)}</td></tr>",
        f"<tr><td>Deprecated</td><td>{deprecated_after}</td><td>{len(added_deprecated)}</td><td>&mdash;</td></tr>",
        "</table>",
        details("Added active", added_active),
        details("Removed active", removed_active),
        details("Added deprecated", added_deprecated),
    ]

    def affected_section(title: str, nims_key: str) -> None:
        entries = entries_with(affected, nims_key)
        parts.append(f"<h3>{html.escape(title)}: {len(entries)}</h3>")
        if entries:
            items = []
            for entry in entries:
                repo = html.escape(str(entry.get("repository", "")))
                nims = html.escape(", ".join(entry.get(nims_key) or []))
                items.append(f"<li><strong>{repo}</strong>{' &mdash; ' + nims if nims else ''}</li>")
            parts.append("<ul>\n" + "\n".join(items) + "\n</ul>")

    affected_section("Blueprints affected by deprecated NIMs (hosted)", "affected_hosted_nims")
    affected_section("Blueprints affected by deprecated NIMs (local)", "affected_local_nims")

    if run_url:
        parts.append(f'<p><a href="{html.escape(run_url)}">View run</a></p>')
    return "\n".join(p for p in parts if p) + "\n"


def render_slack(
    run_number: str,
    run_url: str,
    color: str,
    active_after,
    deprecated_after,
    added_active: list,
    removed_active: list,
    added_deprecated: list,
    affected: list,
) -> dict:
    emoji = COLOR_EMOJI.get(color, "")
    title = f"NIM Usage Scan #{run_number}" if run_number else "NIM Usage Scan"
    if emoji:
        title = f"{emoji} {title}"

    lines = [
        f"*Repos refresh:* active {active_after} (added {len(added_active)}, removed {len(removed_active)}), "
        f"deprecated {deprecated_after} (added {len(added_deprecated)})",
    ]

    def affected_section(title: str, nims_key: str) -> None:
        entries = entries_with(affected, nims_key)
        lines.append(f"*{title}:* {len(entries)}")
        for entry in entries[:SLACK_LIST_CAP]:
            lines.append(f"• {entry.get('repository', '')}")
        if len(entries) > SLACK_LIST_CAP:
            lines.append(f"…and {len(entries) - SLACK_LIST_CAP} more")

    affected_section("Blueprints affected by deprecated NIMs (hosted)", "affected_hosted_nims")
    affected_section("Blueprints affected by deprecated NIMs (local)", "affected_local_nims")

    attachment = {
        "color": color,
        "title": title,
        "text": "\n".join(lines),
        "mrkdwn_in": ["text"],
    }
    if run_url:
        attachment["title_link"] = run_url
        attachment["footer"] = f"<{run_url}|View run>"
    return {"attachments": [attachment]}


def write_color(color: str) -> None:
    gh_output = os.environ.get("GITHUB_OUTPUT")
    if gh_output:
        with open(gh_output, "a", encoding="utf-8") as f:
            f.write(f"color={color}\n")
    print(f"color={color}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--refresh-summary", default="output/repos_refresh_summary.json")
    parser.add_argument("--affected", default="output/deprecation_affected_blueprints.json")
    parser.add_argument("--run-number", default=os.environ.get("GITHUB_RUN_NUMBER", ""))
    parser.add_argument("--run-url", default="")
    parser.add_argument("--html-out", default="output/scan-summary.html")
    parser.add_argument("--slack-out", default="output/slack-message.json")
    parser.add_argument(
        "--affected-local-nims-safe",
        action="store_true",
        help="Treat blueprints affected only by deprecated LOCAL NIMs as safe "
             "(color 'good') instead of 'danger'.",
    )
    args = parser.parse_args()

    summary = load_json(args.refresh_summary, None)
    affected = load_json(args.affected, [])
    if not isinstance(affected, list):
        affected = []

    html_path = Path(args.html_out)
    slack_path = Path(args.slack_out)
    html_path.parent.mkdir(parents=True, exist_ok=True)
    slack_path.parent.mkdir(parents=True, exist_ok=True)

    # Refresh summary missing → the refresh step likely failed. Surface it.
    if not isinstance(summary, dict):
        color = "warning"
        html_path.write_text(
            "<h2>Daily NIM Usage Scan</h2>\n"
            "<p>:warning: No refresh summary found; the refresh step may have failed.</p>\n",
            encoding="utf-8",
        )
        attachment = {
            "color": color,
            "title": f"{COLOR_EMOJI['warning']} NIM Usage Scan #{args.run_number}".strip(),
            "text": "No refresh summary found; the refresh step may have failed.",
            "mrkdwn_in": ["text"],
        }
        if args.run_url:
            attachment["title_link"] = args.run_url
        slack_path.write_text(
            json.dumps({"attachments": [attachment]}, indent=2) + "\n", encoding="utf-8"
        )
        write_color(color)
        return

    added_active = summary.get("added_active_repos") or []
    removed_active = summary.get("removed_active_repos") or []
    added_deprecated = summary.get("added_deprecated_repos") or []
    counts = summary.get("counts") or {}
    active_after = counts.get("repos_active_after", "?")
    deprecated_after = counts.get("repos_deprecated_after", "?")

    has_changes = bool(added_active or removed_active or added_deprecated)
    if args.affected_local_nims_safe:
        escalating = entries_with(affected, "affected_hosted_nims")
    else:
        escalating = affected
    if escalating:
        color = "danger"
    elif has_changes:
        color = "warning"
    else:
        color = "good"

    html_path.write_text(
        render_html(
            args.run_number, args.run_url, active_after, deprecated_after,
            added_active, removed_active, added_deprecated, affected,
        ),
        encoding="utf-8",
    )
    slack_path.write_text(
        json.dumps(
            render_slack(
                args.run_number, args.run_url, color, active_after, deprecated_after,
                added_active, removed_active, added_deprecated, affected,
            ),
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )
    write_color(color)


if __name__ == "__main__":
    main()
