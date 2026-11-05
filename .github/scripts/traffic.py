"""Merge GitHub's 14-day traffic (views, clones) and release download counts into
.github/stats/traffic.json and write a readable summary to .github/stats/README.md."""
import datetime
import json
import os
import pathlib
import subprocess

REPO = os.environ["REPO"]
OUT = pathlib.Path(".github/stats")
DATA = OUT / "traffic.json"


def api(path):
    return json.loads(subprocess.check_output(["gh", "api", "-H", "Accept: application/vnd.github+json", path]))


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    data = json.loads(DATA.read_text()) if DATA.exists() else {}
    for key in ("views", "clones", "downloads"):
        data.setdefault(key, {})

    # one entry per day; a later run overwrites the (then complete) earlier value
    for kind in ("views", "clones"):
        for day in api(f"repos/{REPO}/traffic/{kind}?per=day")[kind]:
            data[kind][day["timestamp"][:10]] = {"count": day["count"], "uniques": day["uniques"]}

    snapshot = {}
    for rel in api(f"repos/{REPO}/releases?per_page=100"):
        for asset in rel["assets"]:
            snapshot[f'{rel["tag_name"]}/{asset["name"]}'] = asset["download_count"]
    today = datetime.date.today().isoformat()
    data["downloads"][today] = snapshot

    for key in data:
        data[key] = dict(sorted(data[key].items()))
    DATA.write_text(json.dumps(data, indent=1) + "\n")
    write_summary(data, today)


def write_summary(data, today):
    months = {}
    for kind in ("views", "clones"):
        for day, v in data[kind].items():
            m = months.setdefault(day[:7], {"views": 0, "view_uniques": 0, "clones": 0, "clone_uniques": 0})
            m[kind] += v["count"]
            m["view_uniques" if kind == "views" else "clone_uniques"] += v["uniques"]
    latest = data["downloads"].get(today, {})
    by_release = {}
    for name, n in latest.items():
        tag = name.split("/", 1)[0]
        by_release[tag] = by_release.get(tag, 0) + n

    lines = [
        "# Repository statistics",
        "",
        f"Updated {today} by the `traffic` workflow. GitHub only keeps 14 days of traffic; this keeps all of it.",
        "Daily unique visitors are added up per month, so one person visiting on two days counts twice.",
        "",
        "## Release downloads (Homebrew installs, .deb, AppImage, CLI)",
        "",
        "| Release | Downloads |",
        "| :-- | --: |",
    ]
    lines += [f"| {tag} | {n} |" for tag, n in sorted(by_release.items(), reverse=True)]
    lines += [f"| **Total** | **{sum(by_release.values())}** |", "", "## Traffic per month", "",
              "| Month | Views | Daily unique visitors | Clones | Daily unique cloners |", "| :-- | --: | --: | --: | --: |"]
    lines += [f'| {m} | {v["views"]} | {v["view_uniques"]} | {v["clones"]} | {v["clone_uniques"]} |'
              for m, v in sorted(months.items(), reverse=True)]
    (OUT / "README.md").write_text("\n".join(lines) + "\n")


if __name__ == "__main__":
    main()
