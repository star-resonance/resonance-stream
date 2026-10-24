#!/usr/bin/env python3
"""release: the commands the release workflows run (release-flow skill; kade-agent/agent-config
docs/release-flow-plan.md, sections 3 to 5a and 9). A release is Candidate (.github/workflows/candidate.yml),
run on main, which tags a commit v<version>-rc.N and makes the branch rc/<rc tag>, then Promote
(promote.yml), run on that branch, which adds v<version> on the same commit. Cargo.toml's
[workspace.package] version is a floor (plan 9.2): the release is that version when it is above the
latest final tag, else the latest tag's next patch. In that case Candidate stamps the final version
into a release commit whose parent is the main commit, and tags that commit. Same commands as
agent-config's release.cjs, less its date tag and AGENTS.md stamp.

  next              the final tag of this commit (final_tag's floor rule); refuses a HEAD that is a
                    candidate already (or whose release commit is) and a HEAD with no commit since
                    the latest release
  rc T              the next candidate tag of the final tag T: T-rc.1, T-rc.2, ...
  latest            the highest release tag, or nothing
  main-only REF     refuses unless REF is refs/heads/main (Candidate runs on main only)
  stamp T           writes T's version into Cargo.toml and the workspace crates' Cargo.lock entries
  checks-passed ID  reads a commit's check runs (the API's JSON) on stdin; refuses unless one
                    ran, all finished and none failed, leaving out run ID's own jobs and the
                    candidate workflows' jobs from any run
  check-promote R   refuses unless R is the highest rc tag of a new version whose commit's
                    Cargo.toml has that version; prints the final tag
  rc-branch B       the rc tag of a branch rc/<rc tag>, which Candidate makes at the tag's
                    commit for Promote to run from (the GitHub app lists no tags)
  superseded R      origin's rc branches of R's version below R, space-separated
  released T        origin's rc branches of version T and every older one, space-separated
"""
import json
import os
import re
import subprocess
import sys

RELEASE = re.compile(r"^v\d+\.\d+\.\d+$")
CANDIDATE = re.compile(r"^(v\d+\.\d+\.\d+)-rc\.(\d+)$")
RC_BRANCH = re.compile(r"^rc/(v\d+\.\d+\.\d+-rc\.\d+)$")
USAGE = (
    "usage: release.py next | rc <tag> | latest | main-only <ref> | stamp <tag> | "
    "checks-passed <run id> | check-promote <rc tag> | rc-branch <branch> | superseded <rc tag> | released <tag>"
)

# The jobs of candidate.yml and candidate-guard.yml: their check runs on a main commit, from this
# run or an earlier candidate there, are not the commit's CI. A called workflow's jobs are named
# "<caller job> / <job>".
CANDIDATE_JOBS = {
    "check the candidate",
    "build and sign the exe (Windows)",
    "build the test-env exe (Windows)",
    "real-app smoke test",
    "make the release files",
    "rc tag and prerelease",
    "rc branch unchanged",
}
FAILED = {"failure", "timed_out", "cancelled", "action_required", "stale", "startup_failure"}


class ReleaseError(Exception):
    pass


def version_key(tag):
    return tuple(int(p) for p in tag[1:].split("."))


def latest_release(tags):
    releases = [t for t in tags if RELEASE.match(t)]
    return max(releases, key=version_key) if releases else None


def is_candidate_job(name):
    return name in CANDIDATE_JOBS or any(str(name).startswith(f"{j} / ") for j in CANDIDATE_JOBS)


def version_of(cargo_toml):
    m = re.search(r'^\[workspace\.package\][^\[]*?^version\s*=\s*"([^"]*)"', cargo_toml, re.M | re.S)
    if not m:
        raise ReleaseError("no [workspace.package] version in Cargo.toml")
    return m.group(1)


def final_tag(version, tags):
    """The release of a version file: the version when above the latest final tag, else its next patch."""
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ReleaseError(f"version '{version}' is not X.Y.Z")
    latest = latest_release(tags)
    if latest is None or version_key(f"v{version}") > version_key(latest):
        return f"v{version}"
    major, minor, patch = version_key(latest)
    return f"v{major}.{minor}.{patch + 1}"


def check_main_ref(ref):
    if ref != "refs/heads/main":
        raise ReleaseError(f"a candidate is made from refs/heads/main only; this run is on {ref or '(none)'}")


WORKSPACE_VERSION = re.compile(r'(^\[workspace\.package\][^\[]*?^version\s*=\s*")[^"]*(")', re.M | re.S)


def stamp_cargo_toml(text, version):
    stamped, n = WORKSPACE_VERSION.subn(lambda m: f"{m.group(1)}{version}{m.group(2)}", text, count=1)
    if not n:
        raise ReleaseError("no [workspace.package] version in Cargo.toml")
    return stamped


def stamp_cargo_lock(lock, names, version):
    """Sets the version of the named packages only: a dependency may have the same version number."""
    for name in sorted(names):
        pattern = re.compile(
            r'(^\[\[package\]\]\r?\nname = "' + re.escape(name) + r'"\r?\nversion = ")[^"]*(")', re.M)
        lock, n = pattern.subn(lambda m: f"{m.group(1)}{version}{m.group(2)}", lock)
        if not n:
            raise ReleaseError(f"{name} is not in Cargo.lock")
    return lock


def workspace_crates(root="."):
    """The names of the crates that take the workspace version (version.workspace = true)."""
    names = set()
    for here, dirs, files in os.walk(root):
        dirs[:] = [d for d in dirs if d not in {"target", "node_modules", ".git", "dist"}]
        if "Cargo.toml" not in files:
            continue
        text = open(os.path.join(here, "Cargo.toml"), encoding="utf-8").read()
        name = re.search(r'^\[package\][^\[]*?^name\s*=\s*"([^"]+)"', text, re.M | re.S)
        if name and re.search(r"^version\.workspace\s*=\s*true", text, re.M):
            names.add(name.group(1))
    return names


def candidates(final, tags):
    return [int(m.group(2)) for t in tags if (m := CANDIDATE.match(t)) and m.group(1) == final]


def rc_tag(final, tags):
    if not final or not RELEASE.match(final):
        raise ReleaseError(f"{final or '(none)'} is not a release tag")
    return f"{final}-rc.{max([0, *candidates(final, tags)]) + 1}"


def rc_of_branch(branch):
    m = RC_BRANCH.match(branch or "")
    if not m:
        raise ReleaseError(f"{branch or '(none)'} is not rc/<rc tag>")
    return m.group(1)


def rc_branches(branches):
    return [(b, CANDIDATE.match(m.group(1))) for b in branches if (m := RC_BRANCH.match(b))]


def superseded_branches(branches, rc):
    final, n = CANDIDATE.match(rc).groups()
    return [b for b, m in rc_branches(branches) if m.group(1) == final and int(m.group(2)) < int(n)]


def released_branches(branches, final):
    return [b for b, m in rc_branches(branches) if version_key(m.group(1)) <= version_key(final)]


def checks_passed(runs, run_id):
    own = f"/actions/runs/{run_id}/"
    others = [c for c in runs if own not in str(c.get("details_url") or "") and not is_candidate_job(c.get("name"))]
    if not others:
        raise ReleaseError("no check run on this commit besides this run")
    bad = [
        f"{c['name']}: {c['conclusion'] if c.get('status') == 'completed' else c.get('status')}"
        for c in others
        if c.get("status") != "completed" or c.get("conclusion") in FAILED
    ]
    if bad:
        raise ReleaseError(f"checks not passed: {'; '.join(bad)}")


def check_promote(rc, tags, version):
    m = CANDIDATE.match(rc or "")
    if not m:
        raise ReleaseError(f"{rc or '(none)'} is not a candidate tag vX.Y.Z-rc.N")
    final, n = m.group(1), int(m.group(2))
    highest = max([*candidates(final, tags), n])
    if highest > n:
        raise ReleaseError(f"{final}-rc.{highest} is newer than {rc}")
    if final in tags:
        raise ReleaseError(f"{final} is already released")
    latest = latest_release(tags)
    if latest and version_key(final) <= version_key(latest):
        raise ReleaseError(f"{final} is not above the latest release {latest}")
    if f"v{version}" != final:
        raise ReleaseError(f"Cargo.toml at {rc} is {version}, not {final[1:]}")
    return final


def git(*args):
    p = subprocess.run(["git", *args], capture_output=True, text=True, encoding="utf-8")
    if p.returncode:
        raise ReleaseError(p.stderr.strip() or f"git {' '.join(args)} failed")
    return p.stdout.strip()


def release_tags():
    return [t for t in git("tag", "-l", "v*").split("\n") if t]


def origin_branches(pattern):
    out = git("ls-remote", "--heads", "origin", pattern)
    return [line.split("refs/heads/")[1] for line in out.split("\n") if line]


def next_tag():
    tags = release_tags()
    final = final_tag(version_of(open("Cargo.toml", encoding="utf-8").read()), tags)
    head = git("rev-parse", "HEAD")
    for t in tags:
        if CANDIDATE.match(t):
            # Candidate tags HEAD itself, or the release commit made on top of it.
            commit, *parents = git("rev-list", "--parents", "-n", "1", f"{t}^{{commit}}").split()
            if head in (commit, *parents):
                raise ReleaseError(f"{head} is already a candidate as {t}")
    latest = latest_release(tags)
    if latest and git("rev-list", "--count", f"{latest}^{{commit}}..HEAD") == "0":
        raise ReleaseError(f"nothing to release since {latest}")
    return final


def stamp(tag):
    if not RELEASE.match(tag or ""):
        raise ReleaseError(f"{tag or '(none)'} is not a release tag")
    version = tag[1:]
    names = workspace_crates()
    for path, edit in [("Cargo.toml", lambda t: stamp_cargo_toml(t, version)),
                       ("Cargo.lock", lambda t: stamp_cargo_lock(t, names, version))]:
        if not os.path.isfile(path):
            continue
        text = open(path, encoding="utf-8", newline="").read()
        open(path, "w", encoding="utf-8", newline="").write(edit(text))


def main(argv):
    cmd, rest = (argv[0] if argv else ""), argv[1:]
    arg = rest[0] if rest else ""
    try:
        os.chdir(git("rev-parse", "--show-toplevel"))
        if cmd == "next":
            print(next_tag())
        elif cmd == "rc":
            print(rc_tag(arg, release_tags()))
        elif cmd == "latest":
            print(latest_release(release_tags()) or "")
        elif cmd == "main-only":
            check_main_ref(arg)
        elif cmd == "stamp":
            stamp(arg)
        elif cmd == "checks-passed":
            checks_passed(json.load(sys.stdin).get("check_runs") or [], arg)
        elif cmd == "check-promote":
            print(check_promote(arg, release_tags(), version_of(git("show", f"{arg}^{{commit}}:Cargo.toml"))))
        elif cmd == "rc-branch":
            print(rc_of_branch(arg))
        elif cmd == "superseded":
            print(" ".join(superseded_branches(origin_branches("rc/*"), rc_of_branch(f"rc/{arg}"))))
        elif cmd == "released":
            if not RELEASE.match(arg):
                raise ReleaseError(f"{arg or '(none)'} is not a release tag")
            print(" ".join(released_branches(origin_branches("rc/*"), arg)))
        else:
            raise ReleaseError(USAGE)
    except ReleaseError as e:
        print(f"release: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
