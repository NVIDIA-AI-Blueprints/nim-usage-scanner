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
from dataclasses import dataclass, field, replace
from enum import Enum
from pathlib import Path
from urllib.parse import quote
from urllib.request import Request, urlopen

try:
    import yaml
except ImportError:  # pragma: no cover - surfaced at runtime
    yaml = None


# ===========================================================================
# Configuration
# ===========================================================================
# CLI defaults.
DEFAULT_CONFIG_PATH = "config/repos.yaml"
DEFAULT_SUMMARY_JSON = "repos_refresh_summary.json"

# NGC catalog endpoints.
NGC_BLUEPRINT_LIST_URL = "https://api.ngc.nvidia.com/v2/search/catalog/resources/BLUEPRINT"
NGC_BLUEPRINTS_SPEC_URL_TEMPLATE = "https://api.ngc.nvidia.com/v2/blueprints/{org_name}/{name}/spec"

# Prefix for operator log lines.
LOG_PREFIX = "[Build Page]"


# ===========================================================================
# Data model
# ===========================================================================

class BlueprintCategory(Enum):
    """A repos_active section and the singular category label written per blueprint.

    Declaration order is the order sections appear in repos_active; a repo with
    no blueprints falls back to ``DEVELOPER``.
    """

    ENTERPRISE = ("Enterprise Blueprints", "Enterprise Blueprint")
    DEVELOPER = ("Developer Examples", "Developer Example")
    PARTNER = ("Partner Examples", "Partner Example")
    NEMOCLAW = ("NemoClaw", "NemoClaw")

    def __init__(self, section: str, label: str) -> None:
        self.section = section
        self.label = label


@dataclass
class BlueprintRepository:
    """One entry as it appears under repos_active / repos_github_only.

    ``blueprints`` is a list of ``{name, url, category}`` dicts (the Build
    blueprints backing this GitHub repo).
    """

    name: str
    url: str
    branch: str | None = None
    enabled: bool | None = None
    depth: int | None = None
    blueprints: list[dict] = field(default_factory=list)


# ===========================================================================
# NGC catalog API
# ===========================================================================

def fetch_json(url: str) -> dict:
    req = Request(url, headers={"User-Agent": "nim-usage-scanner/1.0"})
    with urlopen(req, timeout=30) as resp:
        data = resp.read().decode("utf-8")
    return json.loads(data)


def build_blueprint_list_url(page_size: int = 1000) -> str:
    """Build URL for resources/BLUEPRINT list API; returns all blueprints in one response."""
    payload = {"query": "", "pageSize": page_size}
    return f"{NGC_BLUEPRINT_LIST_URL}?q={quote(json.dumps(payload))}"


# ===========================================================================
# Blueprint metadata parsing
# ===========================================================================

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
        if isinstance(attr, dict) and str(attr.get("key", "")).upper() == "DEPRECATION":
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


def publisher_of(resource: dict) -> str:
    """Return the blueprint's `publisher` label (the build.nvidia.com org slug).

    Every current blueprint publishes under ``nvidia``; default to it when the
    label is missing so a build-page URL can still be constructed.
    """
    for label in resource.get("labels") or []:
        if not isinstance(label, dict) or label.get("key") != "publisher":
            continue
        for value in label.get("unresolvedValues") or []:
            if isinstance(value, str) and value:
                return value
    return "nvidia"


def category_label(repo_name: str, types: set[str]) -> str:
    """Singular category label for a blueprint, from its catalog types and owner."""
    if "apicatalogtype_nemoclaw_blueprint" in types:
        category = BlueprintCategory.NEMOCLAW
    elif not repo_name.split("/", 1)[0].lower().startswith("nvidia"):
        category = BlueprintCategory.PARTNER
    elif "apicatalogtype_enterprise_blueprint" in types:
        category = BlueprintCategory.ENTERPRISE
    else:
        category = BlueprintCategory.DEVELOPER
    return category.label


# ===========================================================================
# Config file I/O and repo entries
# ===========================================================================

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


def parse_repo_entry(entry: object) -> BlueprintRepository | None:
    """Build a BlueprintRepository from a raw repos.yaml entry (dict or bare name).

    ``branch`` and ``depth`` are carried only when explicitly set on the entry; a
    repo without them inherits the config-level ``defaults``.
    """
    if isinstance(entry, str):
        entry = {"name": entry}
    if not isinstance(entry, dict):
        return None
    name = entry.get("name")
    if not isinstance(name, str) or not name:
        return None
    blueprints = entry.get("blueprints")
    return BlueprintRepository(
        name=name,
        url=entry.get("url") or f"https://github.com/{name}.git",
        branch=entry.get("branch"),
        enabled=entry.get("enabled"),
        depth=entry.get("depth"),
        blueprints=blueprints if isinstance(blueprints, list) else [],
    )


def dedupe_blueprints(blueprints: list[dict]) -> list[dict]:
    """Sort blueprints by name and drop (name, url) duplicates for stable output."""
    seen: set[tuple[str, str]] = set()
    unique: list[dict] = []
    for bp in sorted(blueprints, key=lambda b: b["name"].lower()):
        key = (bp["name"], bp["url"])
        if key not in seen:
            seen.add(key)
            unique.append(bp)
    return unique


# ===========================================================================
# YAML rendering
# ===========================================================================

def render_repos_yaml(
    active_repos: list[BlueprintRepository],
    github_only_repos: list[BlueprintRepository],
    deprecated_repos: list[str],
    header: dict,
) -> str:
    """Render the full repos.yaml text.

    All YAML string-building lives in this function's nested helpers, so
    switching to PyYAML (or another emitter) only requires changing this one
    function.
    """
    # Plain YAML scalars need no quoting; anything else (colons, hashes, quotes,
    # leading/trailing space) is double-quoted. Blueprint display names are free text.
    plain_scalar = re.compile(r"^[A-Za-z0-9][A-Za-z0-9 _.,/&()'+-]*$")

    def yaml_scalar(value: str) -> str:
        """Render a string as a YAML-safe scalar, double-quoting when needed."""
        if value and plain_scalar.match(value) and not value.endswith(" "):
            return value
        escaped = value.replace("\\", "\\\\").replace('"', '\\"')
        return f'"{escaped}"'

    # repos_active is grouped into these sections, in this order; a repo's
    # section is derived from its blueprints' category labels.
    category_by_label = {category.label: category for category in BlueprintCategory}

    def category_of(repo: BlueprintRepository) -> BlueprintCategory:
        """Place a repo into a section from its blueprints' categories."""
        categories = {category_by_label.get(bp.get("category")) for bp in repo.blueprints}
        for category in BlueprintCategory:
            if category in categories:
                return category
        return BlueprintCategory.DEVELOPER

    def entry_lines(repo: BlueprintRepository) -> list[str]:
        lines = [
            f"  - name: {repo.name}",
            f"    url: {repo.url}",
        ]
        if repo.branch is not None:
            lines.append(f"    branch: {repo.branch}")
        if repo.depth is not None:
            lines.append(f"    depth: {repo.depth}")
        # enabled defaults to true; only the explicit false override is written.
        if repo.enabled is False:
            lines.append("    enabled: false")
        if not repo.blueprints:
            lines.append("    blueprints: []")
        else:
            lines.append("    blueprints:")
            for bp in repo.blueprints:
                lines.append(f"      - name: {yaml_scalar(bp['name'])}")
                lines.append(f"        url: {bp['url']}")
                lines.append(f"        category: {bp['category']}")
        return lines

    def object_section(key: str, comment: str, repos: list[BlueprintRepository]) -> list[str]:
        lines = [comment]
        if not repos:
            lines.append(f"{key}: []")
            return lines
        lines.append(f"{key}:")
        for repo in repos:
            lines.extend(entry_lines(repo))
        return lines

    def active_section(comment: str, repos: list[BlueprintRepository]) -> list[str]:
        """Render repos_active grouped by category (declaration order) with sub-headers."""
        lines = [comment]
        if not repos:
            lines.append("repos_active: []")
            return lines
        lines.append("repos_active:")
        by_category: dict[BlueprintCategory, list[BlueprintRepository]] = {c: [] for c in BlueprintCategory}
        for repo in repos:
            by_category[category_of(repo)].append(repo)
        first = True
        for category in BlueprintCategory:
            section_repos = sorted(by_category[category], key=lambda r: r.name.lower())
            if not section_repos:
                continue
            if not first:
                lines.append("")
            first = False
            lines.append(f"  # {category.section}")
            for repo in section_repos:
                lines.extend(entry_lines(repo))
        return lines

    def name_section(key: str, comment: str, names: list[str]) -> list[str]:
        lines = [comment]
        if not names:
            lines.append(f"{key}: []")
            return lines
        lines.append(f"{key}:")
        for name in names:
            lines.append(f"  - {name}")
        return lines

    lines: list[str] = [
        "# NIM Usage Scanner Configuration",
        "# Repositories to scan for NIM usage, grouped by category.",
        "",
        f'version: "{header["version"]}"',
        "",
        "# Default settings applied to all repositories",
        "defaults:",
        f"  branch: {header['branch']}",
        f"  depth: {header['depth']}",
        "",
    ]
    lines += active_section("# Active on Build and not deprecated. Scanned.", active_repos)
    lines.append("")
    lines += object_section(
        "repos_github_only", "# Only on GitHub (not returned by the Build API). Scanned.", github_only_repos
    )
    lines.append("")
    lines += name_section(
        "repos_deprecated", "# Deprecated on Build (DEPRECATION attribute). NOT scanned.", deprecated_repos
    )
    return "\n".join(lines) + "\n"


# ===========================================================================
# Pipeline steps
# ===========================================================================

def load_current_repos(
    path: Path,
) -> tuple[dict[str, BlueprintRepository], list[str]]:
    """Step 1: read the current config into (active, deprecated).

    ``active`` maps repo name -> BlueprintRepository; ``deprecated`` is a list of
    names. GitHub-only repos are not returned here: they are reconciled against
    the catalog like any other repo (so one going live is auto-detected) and are
    re-read verbatim by ``write_config``.
    """
    config = load_config(path)

    current_active_repos: dict[str, BlueprintRepository] = {}
    for entry in config.get("repos_active") or []:
        repo = parse_repo_entry(entry)
        if repo:
            current_active_repos[repo.name] = repo

    current_deprecated_repos = [n for n in (entry_name(e) for e in config.get("repos_deprecated") or []) if n]

    return current_active_repos, current_deprecated_repos


def fetch_latest_repos(
    org_name: str = "qc69jvmznzxy", page_size: int = 1000, workers: int = 8
) -> tuple[dict[str, BlueprintRepository], list[str]]:
    """Step 2: list blueprints and resolve them to (active, deprecated) repos.

    ``latest_active_repos`` maps each active repo name -> BlueprintRepository
    (with its blueprints, including any newly-deprecated ones so a kept repo
    still has data). ``latest_deprecated_repos`` is a list of names of repos
    backing only deprecated blueprints. A repo backing any active blueprint is
    treated as active.
    """
    data = fetch_json(build_blueprint_list_url(page_size))

    total = data.get("resultTotal")
    if isinstance(total, int):
        print(f"{LOG_PREFIX} Total blueprints: {total}")

    resources: list[dict] = []
    for group in data.get("results", []):
        resources.extend(group.get("resources", []) or [])

    # One (org, name, deprecated, types, display_name, build_url) item per unique blueprint.
    seen: set[tuple[str, str]] = set()
    items: list[tuple[str, str, bool, set[str], str, str]] = []
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
        display_name = res.get("displayName") or name
        build_url = f"https://build.nvidia.com/{publisher_of(res)}/{name}"
        items.append((org, name, is_deprecated(res), catalog_types(res), display_name, build_url))

    active_repos: set[str] = set()
    deprecated_repos: set[str] = set()
    missing_github: list[str] = []
    invalid_github: list[tuple[str, str]] = []
    repo_to_resources: dict[str, list[str]] = {}
    repo_to_blueprints: dict[str, list[dict]] = {}

    def fetch_spec(
        item: tuple[str, str, bool, set[str], str, str],
    ) -> tuple[tuple[str, str, bool, set[str], str, str], str, dict | None]:
        org, name, _deprecated, _types, _display, _build = item
        resource_id = f"{org}/{name}"
        spec_url = NGC_BLUEPRINTS_SPEC_URL_TEMPLATE.format(org_name=org, name=name)
        try:
            return item, resource_id, fetch_json(spec_url)
        except Exception as exc:  # noqa: BLE001 - report and skip
            print(f"{LOG_PREFIX} Failed to fetch spec for {resource_id}: {exc}")
            return item, resource_id, None

    if items:
        with ThreadPoolExecutor(max_workers=workers) as executor:
            for future in as_completed([executor.submit(fetch_spec, it) for it in items]):
                item, resource_id, spec = future.result()
                _, _, deprecated, types, display_name, build_url = item
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
                # Collect blueprints for every repo, including newly-deprecated ones,
                # so a repo kept in repos_active still gets its blueprint data
                # populated (the maintainer curates it by hand as needed).
                repo_to_blueprints.setdefault(repo_name, []).append(
                    {
                        "name": display_name,
                        "url": build_url,
                        "category": category_label(repo_name, types),
                    }
                )
                repo_to_resources.setdefault(repo_name, []).append(resource_id)

    # Active wins: a repo backing any active blueprint is active, not deprecated.
    deprecated_repos -= active_repos

    # New active repos are written without branch/depth/enabled so they inherit
    # the config-level defaults; the maintainer can add overrides by hand.
    latest_active_repos = {
        name: BlueprintRepository(
            name=name,
            url=f"https://github.com/{name}.git",
            blueprints=dedupe_blueprints(repo_to_blueprints.get(name, [])),
        )
        for name in sorted(active_repos)
    }
    latest_deprecated_repos = sorted(deprecated_repos)

    def print_diagnostics() -> None:
        """Operator diagnostics about resolving blueprints to GitHub repos."""
        print(f"{LOG_PREFIX} Blueprints processed: {len(items)}")
        if missing_github:
            print(f"{LOG_PREFIX} Missing GitHub URL for:")
            for resource_id in sorted(set(missing_github)):
                print(f"  - {resource_id}")
        if invalid_github:
            print(f"{LOG_PREFIX} Invalid GitHub URL for:")
            for resource_id, url in invalid_github:
                print(f"  - {resource_id}: {url}")
        duplicates = {k: v for k, v in repo_to_resources.items() if len(v) > 1}
        if duplicates:
            print(f"{LOG_PREFIX} Repos backed by multiple NGC blueprint IDs:")
            for repo, resources in sorted(duplicates.items()):
                print(f"  - {repo}")
                for resource_id in resources:
                    print(f"    * {resource_id}")

    print_diagnostics()
    return latest_active_repos, latest_deprecated_repos


def calculate_difference(
    current_active_repos: dict[str, BlueprintRepository],
    current_deprecated_repos: list[str],
    latest_active_repos: dict[str, BlueprintRepository],
    latest_deprecated_repos: list[str],
    prune_active: bool,
) -> tuple[dict[str, BlueprintRepository], list[str], dict]:
    """Step 3: reconcile current against latest -> (output_active_repos, output_deprecated_repos, summary).

    GitHub-only repos are deliberately not excluded: a repo currently tracked as
    GitHub-only that goes live on the catalog surfaces here as a newly-added
    active repo (the maintainer then drops it from the github-only list).
    """
    current_active_repos_names = set(current_active_repos)
    # Disabled active entries (enabled: false) are intentionally parked; treat
    # them as inactive so a refresh does not flag them as removed or prune them.
    enabled_current_active_repos = {
        name for name, repo in current_active_repos.items() if repo.enabled is not False
    }
    current_deprecated_repos_names = set(current_deprecated_repos)

    latest_active_repos_names = set(latest_active_repos)
    latest_deprecated_repos_names = set(latest_deprecated_repos)

    added_active_repos_names = sorted(latest_active_repos_names - current_active_repos_names)
    # Only enabled active repos are reconciled against the catalog; disabled ones
    # are inactive and never reported as removed (nor pruned below).
    removed_active_repos_names = sorted(enabled_current_active_repos - latest_active_repos_names)
    added_deprecated_repos = sorted(latest_deprecated_repos_names - current_deprecated_repos_names)

    # Preserve existing entries verbatim; refresh their blueprints from the
    # catalog; add newly-active repos. Repos no longer active are kept (their
    # blueprint data comes from the current config) unless --prune-active.
    output_active_repos = dict(current_active_repos)
    for name in sorted(latest_active_repos_names):
        if name in output_active_repos:
            output_active_repos[name] = replace(
                output_active_repos[name], blueprints=latest_active_repos[name].blueprints
            )
        else:
            output_active_repos[name] = latest_active_repos[name]
    if prune_active:
        for name in removed_active_repos_names:
            output_active_repos.pop(name, None)

    output_active_repos_names = set(output_active_repos)
    # Deprecated never overlaps active (a reactivated repo drops out).
    output_deprecated_repos = sorted(
        (current_deprecated_repos_names | set(added_deprecated_repos)) - output_active_repos_names
    )

    summary = {
        "added_active_repos_names": added_active_repos_names,
        "removed_active_repos_names": removed_active_repos_names,
        "added_deprecated_repos": added_deprecated_repos,
        "counts": {
            "current_active": len(latest_active_repos_names),
            "current_deprecated": len(latest_deprecated_repos_names),
            "repos_active_before": len(current_active_repos_names),
            "repos_active_after": len(output_active_repos),
            "repos_deprecated_before": len(current_deprecated_repos_names),
            "repos_deprecated_after": len(output_deprecated_repos),
        },
    }
    return output_active_repos, output_deprecated_repos, summary


def write_config(
    config_path: Path,
    output_path: Path,
    output_active_repos: dict[str, BlueprintRepository],
    output_deprecated_repos: list[str],
) -> None:
    """Step 4: render and write the updated repos.yaml.

    The top-level ``version``/``defaults`` (branch/depth) and the github-only
    repos are read back from the existing config so they carry through as-is.
    """
    config = load_config(config_path)
    defaults = config.get("defaults") or {}
    header = {
        "version": str(config.get("version")),
        "branch": defaults.get("branch"),
        "depth": defaults.get("depth"),
    }
    github_only_repos = [
        repo for repo in (parse_repo_entry(e) for e in config.get("repos_github_only") or []) if repo
    ]
    content = render_repos_yaml(
        list(output_active_repos.values()),
        github_only_repos,
        output_deprecated_repos,
        header,
    )
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(content, encoding="utf-8")


def write_summary(path: Path, summary: dict, prune_active: bool) -> None:
    """Step 5a: write the machine-readable refresh summary JSON."""
    data = {
        "added_active_repos": summary["added_active_repos_names"],
        "removed_active_repos": summary["removed_active_repos_names"],
        "added_deprecated_repos": summary["added_deprecated_repos"],
        "pruned_active": bool(prune_active),
        "counts": summary["counts"],
    }
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")


def print_summary(summary: dict, output_path: Path, summary_path: Path, prune_active: bool) -> None:
    """Step 5b: print the reconciliation result for operators."""
    counts = summary["counts"]
    added_active = summary["added_active_repos_names"]
    removed_active = summary["removed_active_repos_names"]
    added_deprecated = summary["added_deprecated_repos"]
    print(f"{LOG_PREFIX} Current: {counts['current_active']} active, {counts['current_deprecated']} deprecated")
    print(
        f"{LOG_PREFIX} Active added: {len(added_active)}, removed: {len(removed_active)}"
        f"{' (pruned)' if prune_active else ' (kept; use --prune-active to remove)'}"
    )
    print(f"{LOG_PREFIX} Deprecated added: {len(added_deprecated)}")
    print(f"{LOG_PREFIX} Wrote {output_path} and {summary_path}")
    if added_active:
        print(f"{LOG_PREFIX} Added active:")
        for name in added_active:
            print(f"  + {name}")
    if removed_active:
        print(f"{LOG_PREFIX} Removed active (candidates):")
        for name in removed_active:
            print(f"  - {name}")
    if added_deprecated:
        print(f"{LOG_PREFIX} Added deprecated:")
        for name in added_deprecated:
            print(f"  ! {name}")


# ===========================================================================
# Entry point
# ===========================================================================

def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Refresh nim-usage-scanner repos.yaml from NGC blueprint endpoints"
    )
    parser.add_argument("--config", default=DEFAULT_CONFIG_PATH, help="Input repos.yaml path")
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
        default=DEFAULT_SUMMARY_JSON,
        help="Path for the refresh summary JSON",
    )
    parser.add_argument(
        "--prune-active",
        action="store_true",
        help="Remove active repos no longer returned as active by the Build API",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()

    config_path = Path(args.config)
    output_path = Path(args.output) if args.output else (config_path if args.in_place else None)
    if output_path is None:
        raise SystemExit("Specify --output PATH or --in-place to write the updated config.")

    # 1. Load the current config.
    current_active_repos, current_deprecated_repos = load_current_repos(config_path)

    # 2. Fetch the latest blueprints from the Build catalog.
    latest_active_repos, latest_deprecated_repos = fetch_latest_repos()
    if not latest_active_repos and not latest_deprecated_repos:
        print("Error: No blueprints found from NGC API.")
        raise SystemExit(1)

    # 3. Calculate the difference between current and latest.
    output_active_repos, output_deprecated_repos, summary = calculate_difference(
        current_active_repos,
        current_deprecated_repos,
        latest_active_repos,
        latest_deprecated_repos,
        args.prune_active,
    )

    # 4. Write the updated config (github-only repos pass through unchanged).
    write_config(config_path, output_path, output_active_repos, output_deprecated_repos)

    # 5. Write and print the summary.
    summary_path = Path(args.summary_json)
    write_summary(summary_path, summary, args.prune_active)
    print_summary(summary, output_path, summary_path, args.prune_active)


if __name__ == "__main__":
    main()
