#!/usr/bin/env python3
"""release: the commands the release workflows run (release-flow skill; kade-agent/agent-config
docs/release-flow-plan.md, sections 3 to 5a). A release is Candidate (.github/workflows/candidate.yml),
which tags the main commit v<version>-rc.N and makes the branch rc/<rc tag>, then Promote
(promote.yml), run on that branch, which adds v<version> on the same commit. The version is the one
in Cargo.toml [workspace.package]. Same commands as agent-config's release.cjs, less its date tag
and AGENTS.md stamp.

  next              the final tag of Cargo.toml's version (v<X.Y.Z>); refuses a version that is
                    released or not above the latest release, and a HEAD that is a candidate already
  rc T              the next candidate tag of the final tag T: T-rc.1, T-rc.2, ...
  latest            the highest release tag, or nothing
  check-candidate B refuses unless B is candidate/<12-character sha of HEAD>, HEAD is on
                    origin/main and origin has no other candidate/* branch
  should-candidate  true when `next` would accept HEAD (its version has no final tag, is above the
                    latest one, HEAD is no candidate yet) and release-notes/v<version>.md exists;
                    otherwise false, the reason on stderr (auto-candidate.yml)
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
    "usage: release.py next | rc <tag> | latest | check-candidate <branch> | should-candidate | "
    "checks-passed <run id> | check-promote <rc tag> | rc-branch <branch> | superseded <rc tag> | released <tag>"
)

# The jobs of candidate.yml, candidate-guard.yml and auto-candidate.yml: their check runs on a main commit, from this
# run or an earlier candidate there, are not the commit's CI. A called workflow's jobs are named
# "<caller job> / <job>".
CANDIDATE_JOBS = {
    "check the candidate",
    "build and sign the exe (Windows)",
    "build the test-env exe (Windows)",
    "real-app smoke test",
    "make the release files",
    "rc tag and prerelease",
    "delete the candidate branch",
    "candidate branch unchanged",
    "decide on a candidate",
    "push the candidate branch",
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


def check_next(version, tags):
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ReleaseError(f"version '{version}' is not X.Y.Z")
    tag = f"v{version}"
    if tag in tags:
        raise ReleaseError(f"{tag} is already released; raise the version in Cargo.toml")
    latest = latest_release(tags)
    if latest and version_key(tag) <= version_key(latest):
        raise ReleaseError(f"{tag} is not above the latest release {latest}")
    return tag


def should_candidate(version, tags, notes_exist):
    try:
        check_next(version, tags)
    except ReleaseError:
        return False
    return notes_exist


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


def check_candidate(branch, head, on_main, others):
    m = re.fullmatch(r"candidate/([0-9a-f]{12})", branch)
    if not m:
        raise ReleaseError(f"{branch} is not candidate/<12-character sha>")
    if not head.startswith(m.group(1)):
        raise ReleaseError(f"{branch} names {m.group(1)}, not HEAD {head}")
    if not on_main:
        raise ReleaseError(f"{head} is not on origin/main")
    if others:
        raise ReleaseError(f"{', '.join(others)} is left over; Kade deletes it first")


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
    final = check_next(version_of(open("Cargo.toml", encoding="utf-8").read()), release_tags())
    here = [t for t in git("tag", "--points-at", "HEAD").split("\n") if CANDIDATE.match(t)]
    if here:
        raise ReleaseError(f"{git('rev-parse', 'HEAD')} is already a candidate as {here[0]}")
    return final


def should_candidate_here():
    """Prints true or false; the reason for a false goes to stderr."""
    version = version_of(open("Cargo.toml", encoding="utf-8").read())
    try:
        final = next_tag()
    except ReleaseError as e:
        print(f"release: {e}", file=sys.stderr)
        return False
    notes = f"release-notes/{final}.md"
    if not os.path.isfile(notes):
        print(f"release: {notes} is missing", file=sys.stderr)
        return False
    return should_candidate(version, release_tags(), True)


def check_candidate_here(branch):
    head = git("rev-parse", "HEAD")
    on_main = subprocess.run(["git", "merge-base", "--is-ancestor", head, "origin/main"],
                             capture_output=True).returncode == 0
    check_candidate(branch, head, on_main, [b for b in origin_branches("candidate/*") if b != branch])


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
        elif cmd == "check-candidate":
            check_candidate_here(arg)
        elif cmd == "should-candidate":
            print("true" if should_candidate_here() else "false")
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
