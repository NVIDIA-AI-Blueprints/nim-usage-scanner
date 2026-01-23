#!/usr/bin/env python3
"""
Generate nim-usage-scanner repos.yaml from NGC blueprint endpoints.
"""

import argparse
import json
import re
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
from urllib.parse import quote
from urllib.request import Request, urlopen


NGC_SEARCH_ENDPOINT = "https://api.ngc.nvidia.com/v2/search/catalog/resources/ENDPOINT"
NGC_ENDPOINT_SPEC_BASE = "https://api.ngc.nvidia.com/v2/endpoints"


def fetch_json(url: str) -> dict:
    req = Request(url, headers={"User-Agent": "nim-usage-scanner/1.0"})
    with urlopen(req, timeout=30) as resp:
        data = resp.read().decode("utf-8")
    return json.loads(data)


def build_ngc_query(org_name: str, label: str, page: int, page_size: int) -> str:
    payload = {
        "filters": [
            {"field": "label", "value": label},
        ],
        "orderBy": [{"field": "dateCreated", "value": "DESC"}],
        "page": page,
        "pageSize": page_size,
        "query": f'orgName:"{org_name}"',
        "scoredSize": page_size,
    }
    return f"{NGC_SEARCH_ENDPOINT}?q={quote(json.dumps(payload))}"


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

    if not candidates and not download_candidates and not deploy_candidates and not blueprint_urls:
        return None

    candidates.sort(key=lambda x: (-x[0], x[1]))
    return candidates[0][1]


def repo_name_from_github_url(url: str) -> str | None:
    match = re.search(r"https?://github\.com/([^/]+)/([^/#?]+)", url)
    if not match:
        return None
    owner, repo = match.group(1), match.group(2)
    repo = repo.removesuffix(".git")
    return f"{owner}/{repo}"


def fetch_blueprint_repos(
    org_name: str,
    label: str,
    page_size: int,
    workers: int,
) -> tuple[
    list[str],
    list[str],
    list[tuple[str, str]],
    dict[str, list[str]],
    int,
]:
    repos: list[str] = []
    missing_github: list[str] = []
    invalid_github: list[tuple[str, str]] = []
    repo_to_resources: dict[str, list[str]] = {}
    total_resources = 0
    seen_resource_ids: set[str] = set()
    page = 0
    total_expected: int | None = None

    while True:
        url = build_ngc_query(org_name, label, page, page_size)
        data = fetch_json(url)

        if total_expected is None:
            total_expected = data.get("resultTotal")
            if isinstance(total_expected, int):
                print(f"[Build Page] Total blueprints: {total_expected}")

        groups = data.get("results", [])
        resources: list[dict] = []
        for group in groups:
            resources.extend(group.get("resources", []) or [])

        if not resources:
            break

        resource_ids: list[str] = []
        for resource in resources:
            resource_id = resource.get("resourceId")
            name = resource.get("name")
            if not resource_id or not name:
                continue
            if resource_id in seen_resource_ids:
                continue
            seen_resource_ids.add(resource_id)
            total_resources += 1
            resource_ids.append(resource_id)

        def fetch_spec(resource_id: str) -> tuple[str, dict] | tuple[str, None]:
            spec_url = f"{NGC_ENDPOINT_SPEC_BASE}/{resource_id}/spec"
            try:
                return resource_id, fetch_json(spec_url)
            except Exception as exc:
                print(f"[Build Page] Failed to fetch spec for {resource_id}: {exc}")
                return resource_id, None

        if resource_ids:
            with ThreadPoolExecutor(max_workers=workers) as executor:
                futures = [executor.submit(fetch_spec, rid) for rid in resource_ids]
                for future in as_completed(futures):
                    resource_id, spec = future.result()
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
                    repos.append(repo_name)
                    repo_to_resources.setdefault(repo_name, []).append(resource_id)

        if total_expected is not None and (page + 1) * page_size >= total_expected:
            break

        page += 1

    return (
        sorted(set(repos)),
        sorted(set(missing_github)),
        invalid_github,
        repo_to_resources,
        total_resources,
    )


def render_repos_yaml(
    repo_names: list[str],
    branch: str,
    depth: int,
) -> str:
    lines: list[str] = [
        "# NIM Usage Scanner Configuration",
        "# This file defines the repositories to scan for NIM usage",
        "",
        'version: "1.0"',
        "",
        "# Default settings applied to all repositories",
        "defaults:",
        f"  branch: {branch}",
        f"  depth: {depth}",
        "",
        "# List of repositories to scan",
        "repos:",
    ]

    for name in repo_names:
        url = f"https://github.com/{name}.git"
        lines.extend([
            f"  - name: {name}",
            f"    url: {url}",
            f"    branch: {branch}",
            "    enabled: true",
            "",
        ])

    if lines[-1] == "":
        lines.pop()

    return "\n".join(lines) + "\n"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Generate nim-usage-scanner repos.yaml from NGC blueprint endpoints"
    )
    parser.add_argument(
        "--org",
        default="qc69jvmznzxy",
        help="NGC org name (default: qc69jvmznzxy)",
    )
    parser.add_argument("--label", default="blueprint", help="NGC label filter")
    parser.add_argument("--page-size", type=int, default=1000, help="NGC page size")
    parser.add_argument("--workers", type=int, default=8, help="Spec fetch workers")
    parser.add_argument("--branch", default="main", help="Default branch")
    parser.add_argument("--depth", type=int, default=1, help="Git clone depth")
    parser.add_argument(
        "--output",
        default="config/repos.yaml",
        help="Output repos.yaml path",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    repos, missing, invalid, repo_to_resources, total_resources = fetch_blueprint_repos(
        args.org,
        args.label,
        args.page_size,
        args.workers,
    )
    if not repos:
        print("Error: No repositories found from NGC API.")
        raise SystemExit(1)

    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    content = render_repos_yaml(repos, args.branch, args.depth)
    output_path.write_text(content, encoding="utf-8")
    print(f"[Build Page] Total resources processed: {total_resources}")
    print(f"[Build Page] Wrote {len(repos)} repos to {output_path}")
    if missing:
        print("[Build Page] Missing GitHub URL for:")
        for resource_id in missing:
            print(f"  - {resource_id}")
    if invalid:
        print("[Build Page] Invalid GitHub URL for:")
        for resource_id, url in invalid:
            print(f"  - {resource_id}: {url}")
    duplicates = {k: v for k, v in repo_to_resources.items() if len(v) > 1}
    if duplicates:
        print("[Build Page] Multiple blueprints mapped to same repo:")
        for repo, resources in sorted(duplicates.items()):
            print(f"  - {repo}")
            for resource_id in resources:
                print(f"    * {resource_id}")

    not_written = set(missing)
    for resources in duplicates.values():
        not_written.update(resources[1:])
    if not_written:
        print("[Build Page] Blueprints not written to repos.yaml:")
        for resource_id in sorted(not_written):
            print(f"  - {resource_id}")


if __name__ == "__main__":
    main()
