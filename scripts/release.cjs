#!/usr/bin/env node
// release: the commands the release workflows run (release-flow skill; kade-agent/agent-config
// docs/release-flow-plan.md, sections 3 to 5a). A release is Candidate (.github/workflows/candidate.yml),
// which tags the main commit v<version>-rc.N and makes the branch rc/<rc tag>, then Promote
// (promote.yml), run on that branch, which adds v<version> on the same commit. The version is the
// one in Cargo.toml [workspace.package]. Same commands as agent-config's release.cjs, less its
// date tag and AGENTS.md stamp.
//
//   next              the final tag of Cargo.toml's version (v<X.Y.Z>); refuses a version that is
//                     released or not above the latest release, and a HEAD that is a candidate already
//   rc T              the next candidate tag of the final tag T: T-rc.1, T-rc.2, ...
//   latest            the highest release tag, or nothing
//   check-candidate B refuses unless B is candidate/<12-character sha of HEAD>, HEAD is on
//                     origin/main and origin has no other candidate/* branch
//   checks-passed ID  reads a commit's check runs (the API's JSON) on stdin; refuses unless one
//                     ran, all finished and none failed, leaving out run ID's own jobs and the
//                     candidate workflows' jobs from any run
//   check-promote R   refuses unless R is the highest rc tag of a new version whose commit's
//                     Cargo.toml has that version; prints the final tag
//   rc-branch B       the rc tag of a branch rc/<rc tag>, which Candidate makes at the tag's
//                     commit for Promote to run from (the GitHub app lists no tags)
//   superseded R      origin's rc branches of R's version below R, space-separated
//   released T        origin's rc branches of version T and every older one, space-separated
"use strict";

const fs = require("fs");
const { execFileSync } = require("child_process");

const RELEASE = /^v\d+\.\d+\.\d+$/;
const CANDIDATE = /^(v\d+\.\d+\.\d+)-rc\.(\d+)$/;
const RC_BRANCH = /^rc\/(v\d+\.\d+\.\d+-rc\.\d+)$/;

function compareVersions(a, b) {
  const pa = a.slice(1).split(".").map(Number);
  const pb = b.slice(1).split(".").map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] || 0) - (pb[i] || 0);
    if (d) return d;
  }
  return 0;
}

function latestRelease(tags) {
  const releases = tags.filter((t) => RELEASE.test(t));
  return releases.length ? releases.sort(compareVersions).at(-1) : null;
}

// The jobs of candidate.yml and candidate-guard.yml: their check runs on a main commit, from this
// run or an earlier candidate there, are not the commit's CI. A called workflow's jobs are named
// "<caller job> / <job>".
const CANDIDATE_JOBS = new Set([
  "check the candidate",
  "build and sign the exe (Windows)",
  "real-app smoke test",
  "rc tag and prerelease",
  "delete the candidate branch",
  "candidate branch unchanged",
]);
const FAILED = new Set(["failure", "timed_out", "cancelled", "action_required", "stale", "startup_failure"]);

function isCandidateJob(name) {
  return CANDIDATE_JOBS.has(name) || [...CANDIDATE_JOBS].some((j) => String(name).startsWith(`${j} / `));
}

function versionOf(cargoToml) {
  const m = /^\[workspace\.package\][^[]*?^version\s*=\s*"([^"]*)"/ms.exec(cargoToml);
  if (!m) throw new Error("no [workspace.package] version in Cargo.toml");
  return m[1];
}

function checkNext(version, tags) {
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error(`version '${version}' is not X.Y.Z`);
  const tag = `v${version}`;
  if (tags.includes(tag)) throw new Error(`${tag} is already released; raise the version in Cargo.toml`);
  const latest = latestRelease(tags);
  if (latest && compareVersions(tag, latest) <= 0) throw new Error(`${tag} is not above the latest release ${latest}`);
  return tag;
}

function candidates(final, tags) {
  return tags
    .map((t) => CANDIDATE.exec(t))
    .filter((m) => m && m[1] === final)
    .map((m) => Number(m[2]));
}

function rcTag(final, tags) {
  if (!final || !RELEASE.test(final)) throw new Error(`${final || "(none)"} is not a release tag`);
  return `${final}-rc.${Math.max(0, ...candidates(final, tags)) + 1}`;
}

function rcOfBranch(branch) {
  const m = RC_BRANCH.exec(branch || "");
  if (!m) throw new Error(`${branch || "(none)"} is not rc/<rc tag>`);
  return m[1];
}

function rcBranches(branches) {
  return branches
    .map((b) => [b, RC_BRANCH.exec(b)])
    .filter(([, m]) => m)
    .map(([b, m]) => [b, CANDIDATE.exec(m[1])]);
}

function supersededBranches(branches, rc) {
  const [, final, n] = CANDIDATE.exec(rc);
  return rcBranches(branches)
    .filter(([, m]) => m[1] === final && Number(m[2]) < Number(n))
    .map(([b]) => b);
}

function releasedBranches(branches, final) {
  return rcBranches(branches)
    .filter(([, m]) => compareVersions(m[1], final) <= 0)
    .map(([b]) => b);
}

function checkCandidate(branch, head, onMain, others) {
  const m = /^candidate\/([0-9a-f]{12})$/.exec(branch);
  if (!m) throw new Error(`${branch} is not candidate/<12-character sha>`);
  if (!head.startsWith(m[1])) throw new Error(`${branch} names ${m[1]}, not HEAD ${head}`);
  if (!onMain) throw new Error(`${head} is not on origin/main`);
  if (others.length) throw new Error(`${others.join(", ")} is left over; Kade deletes it first`);
}

function checksPassed(runs, runId) {
  const own = `/actions/runs/${runId}/`;
  const others = runs.filter((c) => !String(c.details_url || "").includes(own) && !isCandidateJob(c.name));
  if (!others.length) throw new Error("no check run on this commit besides this run");
  const bad = others
    .filter((c) => c.status !== "completed" || FAILED.has(c.conclusion))
    .map((c) => `${c.name}: ${c.status === "completed" ? c.conclusion : c.status}`);
  if (bad.length) throw new Error(`checks not passed: ${bad.join("; ")}`);
}

function checkPromote(rc, tags, version) {
  const m = CANDIDATE.exec(rc || "");
  if (!m) throw new Error(`${rc || "(none)"} is not a candidate tag vX.Y.Z-rc.N`);
  const final = m[1];
  const highest = Math.max(...candidates(final, tags), Number(m[2]));
  if (highest > Number(m[2])) throw new Error(`${final}-rc.${highest} is newer than ${rc}`);
  if (tags.includes(final)) throw new Error(`${final} is already released`);
  const latest = latestRelease(tags);
  if (latest && compareVersions(final, latest) <= 0) {
    throw new Error(`${final} is not above the latest release ${latest}`);
  }
  if (`v${version}` !== final) throw new Error(`Cargo.toml at ${rc} is ${version}, not ${final.slice(1)}`);
  return final;
}

function git(...args) {
  return execFileSync("git", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
}

function releaseTags() {
  return git("tag", "-l", "v*").split("\n").filter(Boolean);
}

function next() {
  const tags = releaseTags();
  const final = checkNext(versionOf(fs.readFileSync("Cargo.toml", "utf8")), tags);
  const here = git("tag", "--points-at", "HEAD").split("\n").filter((t) => CANDIDATE.test(t));
  if (here.length) throw new Error(`${git("rev-parse", "HEAD")} is already a candidate as ${here[0]}`);
  return final;
}

function originBranches(pattern) {
  return git("ls-remote", "--heads", "origin", pattern)
    .split("\n")
    .filter(Boolean)
    .map((line) => line.split("refs/heads/")[1]);
}

function checkCandidateHere(branch) {
  const head = git("rev-parse", "HEAD");
  let onMain = true;
  try {
    git("merge-base", "--is-ancestor", head, "origin/main");
  } catch {
    onMain = false;
  }
  const others = originBranches("candidate/*").filter((b) => b !== branch);
  checkCandidate(branch, head, onMain, others);
}

if (require.main === module) {
  const [cmd, ...rest] = process.argv.slice(2);
  try {
    process.chdir(git("rev-parse", "--show-toplevel"));
    if (cmd === "next") console.log(next());
    else if (cmd === "rc") console.log(rcTag(rest[0], releaseTags()));
    else if (cmd === "latest") console.log(latestRelease(releaseTags()) || "");
    else if (cmd === "check-candidate") checkCandidateHere(rest[0] || "");
    else if (cmd === "checks-passed") checksPassed(JSON.parse(fs.readFileSync(0, "utf8")).check_runs || [], rest[0]);
    else if (cmd === "check-promote") {
      console.log(checkPromote(rest[0], releaseTags(), versionOf(git("show", `${rest[0]}^{commit}:Cargo.toml`))));
    } else if (cmd === "rc-branch") console.log(rcOfBranch(rest[0]));
    else if (cmd === "superseded") console.log(supersededBranches(originBranches("rc/*"), rcOfBranch(`rc/${rest[0]}`)).join(" "));
    else if (cmd === "released") {
      if (!rest[0] || !RELEASE.test(rest[0])) throw new Error(`${rest[0] || "(none)"} is not a release tag`);
      console.log(releasedBranches(originBranches("rc/*"), rest[0]).join(" "));
    } else {
      throw new Error("usage: release.cjs next | rc <tag> | latest | check-candidate <branch> | checks-passed <run id> | check-promote <rc tag> | rc-branch <branch> | superseded <rc tag> | released <tag>");
    }
  } catch (e) {
    console.error(`release: ${e.message}`);
    process.exit(1);
  }
}

module.exports = { versionOf, checkNext, rcTag, checkCandidate, checksPassed, checkPromote, CANDIDATE_JOBS, rcOfBranch, supersededBranches, releasedBranches };
