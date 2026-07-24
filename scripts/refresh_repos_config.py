#!/usr/bin/env python3
"""
Refresh nim-usage-scanner repos.yaml from NGC blueprint endpoints.

Reads a categorized repos.yaml (``repos_active`` / ``repos_github_only`` /
``repos_deprecated``), queries the current NGC blueprints, splits them into
active vs deprecated (via the ``DEPRECATION`` attribute on each blueprint),
reconciles them with the existing config, writes the updated config, and emits a
machine-readable ``repos_refresh_summary.json``.

Requires PyYAML (see requirements.txt) to read the config.
"""

import argparse
import json
import re
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from urllib.parse import quote
from urllib.request import Request, urlopen

try:
    import yaml
except ImportError:  # pragma: no cover - surfaced at runtime
    yaml = None


# List all blueprints: /v2/search/catalog/resources/BLUEPRINT with query "" and pageSize 1000 (returns all in one response).
NGC_BLUEPRINT_LIST_URL = "https://api.ngc.nvidia.com/v2/search/catalog/resources/BLUEPRINT"
# Spec URL pattern: https://api.ngc.nvidia.com/v2/blueprints/{orgName}/{name}/spec
NGC_BLUEPRINTS_SPEC_URL_TEMPLATE = "https://api.ngc.nvidia.com/v2/blueprints/{org_name}/{name}/spec"
# Attribute key that marks a blueprint as deprecated on the Build catalog.
DEPRECATION_ATTR_KEY = "DEPRECATION"


def fetch_json(url: str) -> dict:
    req = Request(url, headers={"User-Agent": "nim-usage-scanner/1.0"})
    with urlopen(req, timeout=30) as resp:
        data = resp.read().decode("utf-8")
    return json.loads(data)


def build_blueprint_list_url(page_size: int = 1000) -> str:
    """Build URL for resources/BLUEPRINT list API; returns all blueprints in one response."""
    payload = {"query": "", "pageSize": page_size}
    return f"{NGC_BLUEPRINT_LIST_URL}?q={quote(json.dumps(payload))}"


def find_github_url(payload: object) -> str | None:
    candidates: list[tuple[int, str]] = []
    download_candidates: list[tuple[int, str]] = []
    deploy_candidates: list[tuple[int, str]] = []
    blueprint_urls: list[str] = []

    def walk(obj: object) -> None:
        if isinstance(obj, dict):
            if "blueprintUrl" in obj and isinstance(obj.get("blueprintUrl"), str):
                blueprint_urls.append(obj["blueprintUrl"])
            url = obj.get("url")
            text = obj.get("text")
            if isinstance(url, str) and isinstance(text, str):
                text_lower = text.lower()
                if text_lower == "view github":
                    candidates.append((3, url))
                elif text_lower in ("download blueprint", "download now"):
                    download_candidates.append((2, url))
                elif text_lower in ("deploy local", "deploy on cloud"):
                    deploy_candidates.append((1, url))
            for value in obj.values():
                walk(value)
        elif isinstance(obj, list):
            for item in obj:
                walk(item)
        elif isinstance(obj, str):
            # Some specs encode JSON in strings (e.g. attributes: "{\"blueprintUrl\": ...}")
            try:
                decoded = json.loads(obj)
            except json.JSONDecodeError:
                decoded = None
            if isinstance(decoded, dict):
                notify_when_available = False
                for key in ("cta", "secondaryCta"):
                    cta = decoded.get(key)
                    if not isinstance(cta, dict):
                        continue
                    cta_text = cta.get("text")
                    if isinstance(cta_text, str) and cta_text.lower() == "notify when available":
                        notify_when_available = True

                    menu = cta.get("menu")
                    if isinstance(menu, list):
                        for item in menu:
                            if not isinstance(item, dict):
                                continue
                            item_text = item.get("text")
                            item_url = item.get("url")
                            if not isinstance(item_text, str) or not isinstance(item_url, str):
                                continue
                            item_text_lower = item_text.lower()
                            if item_text_lower == "view github":
                                candidates.append((3, item_url))
                            elif item_text_lower in ("download blueprint", "download now"):
                                download_candidates.append((2, item_url))
                            elif item_text_lower == "deploy local":
                                if "github.com" in item_url:
                                    deploy_candidates.append((2, item_url))

                    cta_url = cta.get("url")
                    if isinstance(cta_text, str) and isinstance(cta_url, str):
                        cta_text_lower = cta_text.lower()
                        if cta_text_lower == "view github":
                            candidates.append((3, cta_url))
                        elif cta_text_lower in ("download blueprint", "download now"):
                            download_candidates.append((2, cta_url))
                        elif cta_text_lower == "deploy local":
                            if "github.com" in cta_url:
                                deploy_candidates.append((2, cta_url))

                blueprint_url = decoded.get("blueprintUrl")
                if isinstance(blueprint_url, str) and not notify_when_available:
                    blueprint_urls.append(blueprint_url)

    walk(payload)

    if candidates:
        candidates.sort(key=lambda x: (-x[0], x[1]))
        return candidates[0][1]

    if download_candidates:
        download_candidates.sort(key=lambda x: (-x[0], x[1]))
        return download_candidates[0][1]

    if deploy_candidates:
        deploy_candidates.sort(key=lambda x: (-x[0], x[1]))
        return deploy_candidates[0][1]

    if blueprint_urls:
        return blueprint_urls[0]

    return None


def repo_name_from_github_url(url: str) -> str | None:
    match = re.search(r"https?://github\.com/([^/]+)/([^/#?]+)", url)
    if not match:
        return None
    owner, repo = match.group(1), match.group(2)
    repo = repo.removesuffix(".git")
    return f"{owner}/{repo}"


def is_deprecated(resource: dict) -> bool:
    """A blueprint is deprecated if it carries a DEPRECATION attribute."""
    for attr in resource.get("attributes") or []:
        if isinstance(attr, dict) and str(attr.get("key", "")).upper() == DEPRECATION_ATTR_KEY:
            return True
    return False


def catalog_types(resource: dict) -> set[str]:
    """Collect the `apicatalogtype_*` labels for a blueprint resource."""
    types: set[str] = set()
    for label in resource.get("labels") or []:
        if not isinstance(label, dict):
            continue
        for value in label.get("unresolvedValues") or []:
            if isinstance(value, str) and value.startswith("apicatalogtype_"):
                types.add(value)
    return types


# repos_active is grouped into these sections, in this order. A repo's section is
# derived from its blueprint's apicatalogtype_* label (and its owner, for
# partners). Entries with no NGC metadata fall back to DEFAULT_SECTION.
ACTIVE_SECTIONS: list[tuple[str, str]] = [
    ("Enterprise Blueprints", "  # Enterprise Blueprints"),
    ("Developer Examples", "  # Developer Examples"),
    ("Partner Examples", "  # Partner Examples"),
    ("NemoClaw", "  # NemoClaw"),
]
DEFAULT_SECTION = "Developer Examples"


def categorize(repo_name: str, types: set[str]) -> str:
    """Assign an active repo to a section from its catalog types and owner."""
    if "apicatalogtype_nemoclaw_blueprint" in types:
        return "NemoClaw"
    owner = repo_name.split("/", 1)[0]
    if not owner.lower().startswith("nvidia"):
        return "Partner Examples"
    if "apicatalogtype_enterprise_blueprint" in types:
        return "Enterprise Blueprints"
    return "Developer Examples"


def fetch_current_blueprints(
    org_name: str,
    page_size: int,
    workers: int,
) -> tuple[set[str], set[str], dict[str, set[str]], dict]:
    """List blueprints, split active vs deprecated, and resolve each to its GitHub repo.

    Returns ``(active_repo_names, deprecated_repo_names, active_repo_types,
    diagnostics)``, where ``active_repo_types`` maps each active repo to the union
    of its blueprints' ``apicatalogtype_*`` labels (used for sectioning). A repo
    that backs both an active and a deprecated blueprint is treated as active.
    """
    url = build_blueprint_list_url(page_size)
    data = fetch_json(url)

    total = data.get("resultTotal")
    if isinstance(total, int):
        print(f"[Build Page] Total blueprints: {total}")

    resources: list[dict] = []
    for group in data.get("results", []):
        resources.extend(group.get("resources", []) or [])

    seen: set[tuple[str, str]] = set()
    items: list[tuple[str, str, bool, set[str]]] = []
    for res in resources:
        org = res.get("orgName") or ""
        name = res.get("name") or ""
        if not name:
            rid = res.get("resourceId") or ""
            if "/" in rid:
                org, _, name = rid.partition("/")
            else:
                continue
        if org_name and org != org_name:
            continue
        key = (org, name)
        if key in seen:
            continue
        seen.add(key)
        items.append((org, name, is_deprecated(res), catalog_types(res)))

    active_repos: set[str] = set()
    deprecated_repos: set[str] = set()
    active_repo_types: dict[str, set[str]] = {}
    missing_github: list[str] = []
    invalid_github: list[tuple[str, str]] = []
    repo_to_resources: dict[str, list[str]] = {}

    def fetch_spec(
        item: tuple[str, str, bool, set[str]],
    ) -> tuple[tuple[str, str, bool, set[str]], str, dict | None]:
        org, name, _deprecated, _types = item
        resource_id = f"{org}/{name}"
        spec_url = NGC_BLUEPRINTS_SPEC_URL_TEMPLATE.format(org_name=org, name=name)
        try:
            return item, resource_id, fetch_json(spec_url)
        except Exception as exc:  # noqa: BLE001 - report and skip
            print(f"[Build Page] Failed to fetch spec for {resource_id}: {exc}")
            return item, resource_id, None

    if items:
        with ThreadPoolExecutor(max_workers=workers) as executor:
            for future in as_completed([executor.submit(fetch_spec, it) for it in items]):
                item, resource_id, spec = future.result()
                _, _, deprecated, types = item
                if not spec:
                    continue
                github_url = find_github_url(spec)
                if not github_url:
                    missing_github.append(resource_id)
                    continue
                repo_name = repo_name_from_github_url(github_url)
                if not repo_name:
                    invalid_github.append((resource_id, github_url))
                    continue
                if deprecated:
                    deprecated_repos.add(repo_name)
                else:
                    active_repos.add(repo_name)
                    active_repo_types.setdefault(repo_name, set()).update(types)
                repo_to_resources.setdefault(repo_name, []).append(resource_id)

    # Active wins: a repo backing any active blueprint is active, not deprecated.
    deprecated_repos -= active_repos

    diagnostics = {
        "missing_github": sorted(set(missing_github)),
        "invalid_github": invalid_github,
        "repo_to_resources": repo_to_resources,
        "total_items": len(items),
    }
    return active_repos, deprecated_repos, active_repo_types, diagnostics


# ---------------------------------------------------------------------------
# Config read / reconcile / write
# ---------------------------------------------------------------------------

def load_config(path: Path) -> dict:
    if yaml is None:
        raise SystemExit(
            "PyYAML is required to read the repos config. Install it with "
            "`pip install -r requirements.txt`."
        )
    if not path.exists():
        return {}
    return yaml.safe_load(path.read_text(encoding="utf-8")) or {}


def entry_name(entry: object) -> str | None:
    if isinstance(entry, dict):
        name = entry.get("name")
        return name if isinstance(name, str) and name else None
    if isinstance(entry, str) and entry:
        return entry
    return None


def normalize_entry(entry: object, default_branch: str) -> dict:
    """Return a full {name, url, branch, [depth], enabled} object for a repo."""
    if isinstance(entry, str):
        entry = {"name": entry}
    name = entry.get("name")
    url = entry.get("url") or f"https://github.com/{name}.git"
    branch = entry.get("branch") or default_branch
    result = {
        "name": name,
        "url": url,
        "branch": branch,
        "enabled": bool(entry.get("enabled", True)),
    }
    depth = entry.get("depth")
    if depth is not None:
        result["depth"] = depth
    return result


def _entry_lines(e: dict) -> list[str]:
    lines = [
        f"  - name: {e['name']}",
        f"    url: {e['url']}",
        f"    branch: {e['branch']}",
    ]
    if "depth" in e:
        lines.append(f"    depth: {e['depth']}")
    lines.append(f"    enabled: {'true' if e['enabled'] else 'false'}")
    return lines


def render_object_section(key: str, comment: str, entries: list[dict]) -> list[str]:
    lines = [comment]
    if not entries:
        lines.append(f"{key}: []")
        return lines
    lines.append(f"{key}:")
    for e in entries:
        lines.extend(_entry_lines(e))
    return lines


def render_active_section(
    comment: str, entries: list[dict], category_of: dict[str, str]
) -> list[str]:
    """Render repos_active grouped into ACTIVE_SECTIONS with sub-headers."""
    lines = [comment]
    if not entries:
        lines.append("repos_active: []")
        return lines
    lines.append("repos_active:")
    by_section: dict[str, list[dict]] = {section: [] for section, _ in ACTIVE_SECTIONS}
    for e in entries:
        section = category_of.get(e["name"], DEFAULT_SECTION)
        by_section.setdefault(section, []).append(e)
    first = True
    for section, header in ACTIVE_SECTIONS:
        section_entries = sorted(by_section.get(section, []), key=lambda e: e["name"].lower())
        if not section_entries:
            continue
        if not first:
            lines.append("")
        first = False
        lines.append(header)
        for e in section_entries:
            lines.extend(_entry_lines(e))
    return lines


def render_name_section(key: str, comment: str, names: list[str]) -> list[str]:
    lines = [comment]
    if not names:
        lines.append(f"{key}: []")
        return lines
    lines.append(f"{key}:")
    for name in names:
        lines.append(f"  - {name}")
    return lines


def render_repos_yaml(
    version: str,
    branch: str,
    depth: int,
    active: list[dict],
    github_only: list[dict],
    deprecated: list[str],
    category_of: dict[str, str],
) -> str:
    lines: list[str] = [
        "# NIM Usage Scanner Configuration",
        "# Repositories to scan for NIM usage, grouped by category.",
        "",
        f'version: "{version}"',
        "",
        "# Default settings applied to all repositories",
        "defaults:",
        f"  branch: {branch}",
        f"  depth: {depth}",
        "",
    ]
    lines += render_active_section(
        "# Active on Build and not deprecated. Scanned.",
        active,
        category_of,
    )
    lines.append("")
    lines += render_object_section(
        "repos_github_only",
        "# Only on GitHub (not returned by the Build API). Scanned.",
        github_only,
    )
    lines.append("")
    lines += render_name_section(
        "repos_deprecated",
        "# Deprecated on Build (DEPRECATION attribute). NOT scanned.",
        deprecated,
    )
    return "\n".join(lines) + "\n"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Refresh nim-usage-scanner repos.yaml from NGC blueprint endpoints"
    )
    parser.add_argument("--config", default="config/repos.yaml", help="Input repos.yaml path")
    parser.add_argument(
        "--output",
        default=None,
        help="Write the updated config to this path (default: use --in-place)",
    )
    parser.add_argument(
        "--in-place",
        action="store_true",
        help="Overwrite --config in place (ignored when --output is given)",
    )
    parser.add_argument(
        "--summary-json",
        default="repos_refresh_summary.json",
        help="Path for the refresh summary JSON",
    )
    parser.add_argument(
        "--prune-active",
        action="store_true",
        help="Remove active repos no longer returned as active by the Build API",
    )
    parser.add_argument("--org", default="qc69jvmznzxy", help="NGC org name (default: qc69jvmznzxy)")
    parser.add_argument("--page-size", type=int, default=1000, help="NGC page size")
    parser.add_argument("--workers", type=int, default=8, help="Spec fetch workers")
    parser.add_argument("--branch", default="main", help="Default branch for new entries")
    parser.add_argument("--depth", type=int, default=1, help="Default clone depth")
    return parser.parse_args()


def main() -> None:
    args = parse_args()

    config_path = Path(args.config)
    output_path = Path(args.output) if args.output else (config_path if args.in_place else None)
    if output_path is None:
        raise SystemExit("Specify --output PATH or --in-place to write the updated config.")

    config = load_config(config_path)
    version = str(config.get("version") or "1.0")
    defaults = config.get("defaults") or {}
    branch = defaults.get("branch") or args.branch
    depth = defaults.get("depth")
    depth = args.depth if depth is None else depth

    existing_active = config.get("repos_active") or []
    existing_github_only = config.get("repos_github_only") or []
    existing_deprecated = config.get("repos_deprecated") or []

    existing_active_by_name: dict[str, dict] = {}
    for e in existing_active:
        name = entry_name(e)
        if name:
            existing_active_by_name[name] = e if isinstance(e, dict) else {"name": name}
    existing_active_names = set(existing_active_by_name)
    # Disabled active entries (enabled: false) are intentionally parked; treat
    # them as inactive so a refresh does not flag them as removed or prune them.
    enabled_active_names = {
        name for name, entry in existing_active_by_name.items() if entry.get("enabled", True) is not False
    }
    github_only_names = {n for n in (entry_name(e) for e in existing_github_only) if n}
    existing_deprecated_names = {n for n in (entry_name(e) for e in existing_deprecated) if n}

    active_repos, deprecated_repos, active_repo_types, diag = fetch_current_blueprints(
        args.org, args.page_size, args.workers
    )
    if not active_repos and not deprecated_repos:
        print("Error: No blueprints found from NGC API.")
        raise SystemExit(1)

    # Repos tracked as GitHub-only are managed manually; never touch them.
    current_active = active_repos - github_only_names
    current_deprecated = deprecated_repos - github_only_names

    added_active = sorted(current_active - existing_active_names)
    # Only enabled active repos are reconciled against the catalog; disabled ones
    # are inactive and never reported as removed (nor pruned below).
    removed_active = sorted(enabled_active_names - current_active)
    added_deprecated = sorted(current_deprecated - existing_deprecated_names)

    # Build the updated active list, preserving existing entries verbatim.
    new_active_by_name = dict(existing_active_by_name)
    for name in added_active:
        new_active_by_name[name] = {
            "name": name,
            "url": f"https://github.com/{name}.git",
            "branch": branch,
            "enabled": True,
        }
    if args.prune_active:
        for name in removed_active:
            new_active_by_name.pop(name, None)

    new_active_names = set(new_active_by_name)
    new_active = [normalize_entry(new_active_by_name[n], branch) for n in sorted(new_active_by_name)]
    new_github_only = [normalize_entry(e, branch) for e in existing_github_only]
    # Deprecated never overlaps active or github-only (a reactivated repo drops out).
    new_deprecated = sorted(
        (existing_deprecated_names | set(added_deprecated)) - new_active_names - github_only_names
    )

    # Section for each active repo, derived from NGC labels; unknowns (e.g. repos
    # not currently returned by the catalog) fall back to DEFAULT_SECTION.
    category_of = {
        name: categorize(name, active_repo_types.get(name, set())) for name in new_active_names
    }

    content = render_repos_yaml(
        version, branch, depth, new_active, new_github_only, new_deprecated, category_of
    )
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(content, encoding="utf-8")

    summary = {
        "added_active_blueprints": added_active,
        "removed_active_blueprints": removed_active,
        "added_deprecated_blueprints": added_deprecated,
        "pruned_active": bool(args.prune_active),
        "counts": {
            "current_active": len(current_active),
            "current_deprecated": len(current_deprecated),
            "repos_active_before": len(existing_active_names),
            "repos_active_after": len(new_active),
            "repos_deprecated_before": len(existing_deprecated_names),
            "repos_deprecated_after": len(new_deprecated),
        },
    }
    summary_path = Path(args.summary_json)
    summary_path.parent.mkdir(parents=True, exist_ok=True)
    summary_path.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")

    # Operator diagnostics.
    print(f"[Build Page] Blueprints processed: {diag['total_items']}")
    print(f"[Build Page] Current: {len(current_active)} active, {len(current_deprecated)} deprecated")
    print(
        f"[Build Page] Active added: {len(added_active)}, removed: {len(removed_active)}"
        f"{' (pruned)' if args.prune_active else ' (kept; use --prune-active to remove)'}"
    )
    print(f"[Build Page] Deprecated added: {len(added_deprecated)}")
    print(f"[Build Page] Wrote {output_path} and {summary_path}")
    if added_active:
        print("[Build Page] Added active:")
        for name in added_active:
            print(f"  + {name}")
    if removed_active:
        print("[Build Page] Removed active (candidates):")
        for name in removed_active:
            print(f"  - {name}")
    if added_deprecated:
        print("[Build Page] Added deprecated:")
        for name in added_deprecated:
            print(f"  ! {name}")
    if diag["missing_github"]:
        print("[Build Page] Missing GitHub URL for:")
        for resource_id in diag["missing_github"]:
            print(f"  - {resource_id}")
    if diag["invalid_github"]:
        print("[Build Page] Invalid GitHub URL for:")
        for resource_id, url in diag["invalid_github"]:
            print(f"  - {resource_id}: {url}")
    duplicates = {k: v for k, v in diag["repo_to_resources"].items() if len(v) > 1}
    if duplicates:
        print("[Build Page] Repos backed by multiple NGC blueprint IDs:")
        for repo, resources in sorted(duplicates.items()):
            print(f"  - {repo}")
            for resource_id in resources:
                print(f"    * {resource_id}")


if __name__ == "__main__":
    main()
