"""Keep a lasting record of how often SiliconMetaTrader5 is downloaded and visited.

Writes to $STATS_DIR (the `stats` branch, checked out by traffic.yml):
- traffic.json: daily views and clones (GitHub keeps only 14 days), and a daily
  snapshot of every release file's download count, the Docker Hub pulls and the
  PyPI installs (pepy.tech).
- badge-downloads.json: the README's single downloads badge: PyPI + Docker Hub +
  release files (+ DOWNLOADS_BASELINE).
- README.md: a readable summary.
"""
import datetime
import json
import os
import pathlib
import re
import subprocess
import urllib.request

REPO = os.environ["REPO"]
OUT = pathlib.Path(os.environ.get("STATS_DIR", ".github/stats"))
PYPI = os.environ.get("PYPI_PACKAGE", "siliconmetatrader5")
DOCKERHUB = [r for r in os.environ.get("DOCKERHUB_REPOS", "bahadirumutiscimen/pysiliconwine").split() if r]
# Downloads that happened somewhere nothing can count any more (set as the
# DOWNLOADS_BASELINE repository variable); 0 by default.
BASELINE = int(os.environ.get("DOWNLOADS_BASELINE") or 0)


def api(path):
    return json.loads(subprocess.check_output(["gh", "api", "-H", "Accept: application/vnd.github+json", path]))


def dockerhub_pulls(repo):
    req = urllib.request.Request(f"https://hub.docker.com/v2/repositories/{repo}/", headers={"User-Agent": "siliconmt5-stats"})
    with urllib.request.urlopen(req, timeout=20) as r:
        return int(json.load(r)["pull_count"])


def pypi_installs(package, period):
    """Exact PyPI downloads from pepy.tech's badge (period: total | month)."""
    url = f"https://static.pepy.tech/personalized-badge/{package}?period={period}&units=NONE&left_text=x"
    req = urllib.request.Request(url, headers={"User-Agent": "siliconmt5-stats"})
    with urllib.request.urlopen(req, timeout=20) as r:
        nums = re.findall(r">\s*([0-9]+)\s*<", r.read().decode())
    return int(nums[-1])


def short(n):
    for div, unit in ((1_000_000, "M"), (1_000, "k")):
        if n >= div:
            return f"{n / div:.1f}".rstrip("0").rstrip(".") + unit
    return str(n)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    path = OUT / "traffic.json"
    data = json.loads(path.read_text()) if path.exists() else {}
    for key in ("views", "clones", "downloads", "dockerhub", "pypi"):
        data.setdefault(key, {})
    today = datetime.date.today().isoformat()

    try:  # needs TRAFFIC_TOKEN ("Administration: Read-only")
        for kind in ("views", "clones"):
            for day in api(f"repos/{REPO}/traffic/{kind}?per=day")[kind]:
                data[kind][day["timestamp"][:10]] = {"count": day["count"], "uniques": day["uniques"]}
    except subprocess.CalledProcessError:
        print("::warning::traffic not readable (TRAFFIC_TOKEN missing or without Administration: Read-only); downloads only")

    data["downloads"][today] = {f'{r["tag_name"]}/{a["name"]}': a["download_count"]
                                for r in api(f"repos/{REPO}/releases?per_page=100") for a in r["assets"]}
    pulls = {}
    for repo in DOCKERHUB:
        try:
            pulls[repo] = dockerhub_pulls(repo)
        except Exception as e:  # keep the last known value
            print(f"::warning::Docker Hub {repo}: {e}")
            last = [v[repo] for v in data["dockerhub"].values() if repo in v]
            if last:
                pulls[repo] = last[-1]
    data["dockerhub"][today] = pulls

    pypi = {}
    for period in ("total", "month"):
        try:
            pypi[period] = pypi_installs(PYPI, period)
        except Exception as e:
            print(f"::warning::PyPI {period}: {e}")
            last = [v[period] for v in data["pypi"].values() if period in v]
            if last:
                pypi[period] = last[-1]
    data["pypi"][today] = pypi

    for key in data:
        data[key] = dict(sorted(data[key].items()))
    path.write_text(json.dumps(data, indent=1) + "\n")

    releases = sum(data["downloads"][today].values())
    docker = sum(pulls.values())
    pip = pypi.get("total", 0)
    total = pip + releases + docker + BASELINE
    (OUT / "badge-downloads.json").write_text(json.dumps(
        {"schemaVersion": 1, "label": "downloads", "message": short(total), "color": "24292f"}) + "\n")
    write_summary(data, today, pypi, releases, docker, total)


def write_summary(data, today, pypi, releases, docker, total):
    months = {}
    for kind in ("views", "clones"):
        for day, v in data[kind].items():
            m = months.setdefault(day[:7], {"views": 0, "view_uniques": 0, "clones": 0, "clone_uniques": 0})
            m[kind] += v["count"]
            m["view_uniques" if kind == "views" else "clone_uniques"] += v["uniques"]
    by_release = {}
    for name, n in data["downloads"][today].items():
        tag = name.split("/", 1)[0]
        by_release[tag] = by_release.get(tag, 0) + n

    lines = [
        "# SiliconMetaTrader5 statistics",
        "",
        f"Updated {today} by the `traffic` workflow (daily). GitHub keeps only 14 days of traffic; this keeps all of it.",
        "",
        "## Downloads",
        "",
        "| Source | Count |",
        "| :-- | --: |",
        f"| PyPI `{PYPI}` (pip installs, all time) | {pypi.get('total', 0)} |",
        f"| GitHub release files (Homebrew installs, .deb, AppImage, CLI) | {releases} |",
        f"| Docker Hub pulls ({', '.join(DOCKERHUB)}, 1.x) | {docker} |",
    ]
    if BASELINE:
        lines.append(f"| Earlier downloads that can no longer be counted | {BASELINE} |")
    lines += [f"| **Total (README badge)** | **{total}** |", "",
              f"PyPI installs in the last 30 days: {pypi.get('month', 0)} (source: pepy.tech).", "",
              "## Release files", "", "| Release | Downloads |", "| :-- | --: |"]
    lines += [f"| {tag} | {n} |" for tag, n in sorted(by_release.items(), reverse=True)]
    lines += ["", "## Traffic per month", "",
              "Daily unique visitors are added up per month, so one person visiting on two days counts twice.", "",
              "| Month | Views | Daily unique visitors | Clones | Daily unique cloners |", "| :-- | --: | --: | --: | --: |"]
    lines += [f'| {m} | {v["views"]} | {v["view_uniques"]} | {v["clones"]} | {v["clone_uniques"]} |'
              for m, v in sorted(months.items(), reverse=True)]
    (OUT / "README.md").write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
