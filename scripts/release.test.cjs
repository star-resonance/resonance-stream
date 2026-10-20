// Run: node --test scripts/release.test.cjs
"use strict";

const test = require("node:test");
const assert = require("node:assert");
const fs = require("fs");
const os = require("os");
const path = require("path");
const { execFileSync } = require("child_process");
const r = require("./release.cjs");

const CARGO = (version) => `[workspace]\nmembers = ["a"]\n\n[workspace.package]\nversion = "${version}"\nedition = "2021"\n`;

test("versionOf reads [workspace.package] version of Cargo.toml", () => {
  assert.strictEqual(r.versionOf(CARGO("0.6.2")), "0.6.2");
  assert.strictEqual(r.versionOf('[package]\nversion = "9.9.9"\n[workspace.package]\nversion = "1.2.3"\n'), "1.2.3");
  assert.throws(() => r.versionOf('[package]\nversion = "9.9.9"\n'), /no \[workspace\.package\] version/);
});

test("checkNext wants a version X.Y.Z that no release has and that is above the latest release", () => {
  assert.strictEqual(r.checkNext("0.6.3", ["v0.6.2"]), "v0.6.3");
  assert.strictEqual(r.checkNext("0.6.3", []), "v0.6.3");
  assert.strictEqual(r.checkNext("0.6.3", ["v0.6.2", "v0.6.3-rc.1", "v0.6.0-rc.main"]), "v0.6.3", "rc tags are not releases");
  assert.throws(() => r.checkNext("0.6.2", ["v0.6.2"]), /v0\.6\.2 is already released/);
  assert.throws(() => r.checkNext("0.6.1", ["v0.6.2"]), /not above the latest release v0\.6\.2/);
  assert.strictEqual(r.checkNext("0.10.0", ["v0.9.9"]), "v0.10.0", "numeric, not text, order");
});

test("checkNext refuses a version that is not X.Y.Z", () => {
  for (const v of ["0.6", "0.6.2-rc.1", "v0.6.2", "", "0.6.x"]) {
    assert.throws(() => r.checkNext(v, []), /not X\.Y\.Z/, v);
  }
});

test("rcTag is the next candidate number of a version", () => {
  assert.strictEqual(r.rcTag("v0.6.3", []), "v0.6.3-rc.1");
  assert.strictEqual(r.rcTag("v0.6.3", ["v0.6.3-rc.1", "v0.6.3-rc.2", "v0.6.30-rc.5", "v0.6.3-rc.main"]), "v0.6.3-rc.3");
  assert.throws(() => r.rcTag("v0.6.3-rc.1", []), /not a release tag/);
});

test("checkCandidate wants candidate/<12-character sha of HEAD>, on main, and no other candidate", () => {
  const head = "0123456789abcdef0123456789abcdef01234567";
  assert.doesNotThrow(() => r.checkCandidate("candidate/0123456789ab", head, true, []));
  assert.throws(() => r.checkCandidate("candidate/v0.6.3", head, true, []), /candidate\/<12-character sha>/);
  assert.throws(() => r.checkCandidate("candidate/fedcba987654", head, true, []), /not HEAD/);
  assert.throws(() => r.checkCandidate("candidate/0123456789ab", head, false, []), /not on origin\/main/);
  assert.throws(() => r.checkCandidate("candidate/0123456789ab", head, true, ["candidate/aaaaaaaaaaaa"]), /candidate\/aaaaaaaaaaaa is left over/);
});

test("checksPassed wants at least one finished check run, none failed, this run's own left out", () => {
  const run = (name, status, conclusion, id = 1) => ({ name, status, conclusion, details_url: `https://github.com/o/r/actions/runs/${id}/job/9` });
  assert.doesNotThrow(() => r.checksPassed([run("test", "completed", "success"), run("guard", "completed", "skipped")], "7"));
  assert.doesNotThrow(() => r.checksPassed([run("test", "completed", "success"), run("something", "in_progress", null, 7)], "7"));
  assert.throws(() => r.checksPassed([run("something", "in_progress", null, 7)], "7"), /no check run/);
  assert.throws(() => r.checksPassed([run("test", "in_progress", null)], "7"), /test: in_progress/);
  for (const c of ["failure", "timed_out", "cancelled", "action_required"]) {
    assert.throws(() => r.checksPassed([run("test", "completed", c)], "7"), new RegExp(`test: ${c}`));
  }
});

// A candidate that failed leaves its check runs on the main commit; a new candidate branch at the
// same commit must not be refused for them, nor for a guard run of an earlier push there.
test("checksPassed leaves out the candidate workflows' own jobs from any run, called workflows too", () => {
  const run = (name, conclusion, id) => ({ name, status: "completed", conclusion, details_url: `https://github.com/o/r/actions/runs/${id}/job/9` });
  const own = [...r.CANDIDATE_JOBS];
  assert.doesNotThrow(() => r.checksPassed([
    run("test", "success", 1),
    ...own.map((name) => run(name, "failure", 2)),
    run("real-app smoke test / smoke (Windows)", "failure", 2),
  ], "7"));
  assert.throws(() => r.checksPassed([run(own[0], "success", 2)], "7"), /no check run/);
});

test("the job names checksPassed leaves out are the candidate workflows' own jobs", () => {
  const text = ["candidate.yml", "candidate-guard.yml"]
    .map((f) => fs.readFileSync(path.join(__dirname, "..", ".github", "workflows", f), "utf8"))
    .join("\n");
  const names = [...text.matchAll(/^ {4}name: (.+)$/gm)].map((m) => m[1]);
  assert.deepStrictEqual([...r.CANDIDATE_JOBS].sort(), names.sort());
});

test("checkPromote wants the highest rc of a new version whose commit carries that version", () => {
  const tags = ["v0.6.2", "v0.6.3-rc.1", "v0.6.3-rc.2"];
  assert.strictEqual(r.checkPromote("v0.6.3-rc.2", tags, "0.6.3"), "v0.6.3");
  assert.throws(() => r.checkPromote("v0.6.3", tags, "0.6.3"), /not a candidate tag/);
  assert.throws(() => r.checkPromote("v0.6.3-rc.1", tags, "0.6.3"), /v0\.6\.3-rc\.2 is newer/);
  assert.throws(() => r.checkPromote("v0.6.3-rc.2", [...tags, "v0.6.3"], "0.6.3"), /already released/);
  assert.throws(() => r.checkPromote("v0.6.3-rc.2", [...tags, "v0.7.0"], "0.6.3"), /not above the latest release v0\.7\.0/);
  assert.throws(() => r.checkPromote("v0.6.3-rc.2", tags, "0.6.4"), /Cargo\.toml at v0\.6\.3-rc\.2 is 0\.6\.4, not 0\.6\.3/);
});

// Promote runs from a branch rc/<rc tag>, which Candidate makes at the rc tag's commit: the
// GitHub app lists branches only. The tag stays the record; the branch is a handle.
test("rcOfBranch reads the rc tag out of rc/<rc tag> and refuses anything else", () => {
  assert.strictEqual(r.rcOfBranch("rc/v0.6.3-rc.1"), "v0.6.3-rc.1");
  assert.strictEqual(r.rcOfBranch("rc/v0.0.1-rc.12"), "v0.0.1-rc.12");
  for (const b of ["candidate/v0.6.3-rc.1", "candidate/0123456789ab", "rc/v0.6.3", "main", "rc", "rc/v1-rc.x", "rc/v0.6.0-rc.main"]) {
    assert.throws(() => r.rcOfBranch(b), /not rc\/<rc tag>/, b);
  }
});

test("supersededBranches are the same version's rc branches below the new rc", () => {
  const branches = ["rc/v1.2.0-rc.1", "rc/v1.2.0-rc.2", "rc/v1.1.0-rc.4", "candidate/0123456789ab", "rc/v1.2.0-rc.3"];
  assert.deepStrictEqual(r.supersededBranches(branches, "v1.2.0-rc.3"), ["rc/v1.2.0-rc.1", "rc/v1.2.0-rc.2"]);
  assert.deepStrictEqual(r.supersededBranches([], "v1.2.0-rc.1"), []);
});

test("releasedBranches are the rc branches of the released version and every older one", () => {
  const branches = ["rc/v1.2.0-rc.1", "rc/v1.2.0-rc.2", "rc/v1.1.0-rc.4", "rc/v1.3.0-rc.1", "rc/v1.10.0-rc.1", "candidate/0123456789ab", "main"];
  assert.deepStrictEqual(r.releasedBranches(branches, "v1.2.0"), ["rc/v1.2.0-rc.1", "rc/v1.2.0-rc.2", "rc/v1.1.0-rc.4"]);
});

function git(cwd, ...args) {
  return execFileSync("git", ["-C", cwd, ...args], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
}

function release(cwd, ...args) {
  return execFileSync(process.execPath, [path.join(__dirname, "release.cjs"), ...args], {
    cwd,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

// One origin, one clone on main, Cargo.toml at 0.6.2, released as v0.6.2.
function released() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "release-"));
  const origin = path.join(root, "origin.git");
  const dev = path.join(root, "dev");
  git(root, "init", "-q", "--bare", "-b", "main", origin);
  git(root, "clone", "-q", origin, dev);
  git(dev, "symbolic-ref", "HEAD", "refs/heads/main");
  git(dev, "config", "user.email", "t@t");
  git(dev, "config", "user.name", "t");
  fs.writeFileSync(path.join(dev, "Cargo.toml"), CARGO("0.6.2"));
  git(dev, "add", "-A");
  git(dev, "commit", "-q", "-m", "first");
  git(dev, "tag", "v0.6.2");
  git(dev, "push", "-q", "origin", "main", "v0.6.2");
  return { origin, dev };
}

function bump(dev, version) {
  fs.writeFileSync(path.join(dev, "Cargo.toml"), CARGO(version));
  git(dev, "commit", "-q", "-am", `version ${version}`);
}

test("next prints the tag of the Cargo.toml version and refuses a version already released", () => {
  const { dev } = released();
  assert.throws(() => release(dev, "next"), /v0\.6\.2 is already released/);
  bump(dev, "0.6.3");
  assert.strictEqual(release(dev, "next").trim(), "v0.6.3");
});

test("next refuses a commit that already carries a candidate tag", () => {
  const { dev } = released();
  bump(dev, "0.6.3");
  git(dev, "tag", "v0.6.3-rc.1");
  assert.throws(() => release(dev, "next"), /already a candidate as v0\.6\.3-rc\.1/);
});

test("latest prints the highest release tag, ignoring rc and other v tags", () => {
  const { dev } = released();
  git(dev, "tag", "v0.0.0-probe");
  git(dev, "tag", "v0.6.10");
  git(dev, "tag", "v0.6.11-rc.1");
  assert.strictEqual(release(dev, "latest").trim(), "v0.6.10");
});

test("rc and check-promote read the repository's tags and Cargo.toml at the rc tag", () => {
  const { dev } = released();
  bump(dev, "0.6.3");
  assert.strictEqual(release(dev, "rc", "v0.6.3").trim(), "v0.6.3-rc.1");
  git(dev, "tag", "v0.6.3-rc.1");
  assert.strictEqual(release(dev, "rc", "v0.6.3").trim(), "v0.6.3-rc.2");
  assert.strictEqual(release(dev, "check-promote", "v0.6.3-rc.1").trim(), "v0.6.3");
  bump(dev, "0.6.4");
  git(dev, "tag", "v0.6.3-rc.2");
  assert.throws(() => release(dev, "check-promote", "v0.6.3-rc.2"), /is 0\.6\.4, not 0\.6\.3/);
});

test("check-candidate reads HEAD, origin/main and the origin's candidate branches", () => {
  const { dev } = released();
  const short = git(dev, "rev-parse", "HEAD").slice(0, 12);
  git(dev, "fetch", "-q", "origin");
  git(dev, "push", "-q", "origin", `HEAD:refs/heads/candidate/${short}`);
  assert.doesNotThrow(() => release(dev, "check-candidate", `candidate/${short}`));
  git(dev, "push", "-q", "origin", "HEAD:refs/heads/candidate/aaaaaaaaaaaa");
  assert.throws(() => release(dev, "check-candidate", `candidate/${short}`), /left over/);
  git(dev, "push", "-q", "origin", "--delete", "candidate/aaaaaaaaaaaa");
  bump(dev, "0.6.3");
  const off = git(dev, "rev-parse", "HEAD").slice(0, 12);
  assert.throws(() => release(dev, "check-candidate", `candidate/${off}`), /not on origin\/main/);
});

test("rc-branch, superseded and released read origin's rc/ branches; check-candidate ignores them", () => {
  const { dev } = released();
  git(dev, "fetch", "-q", "origin");
  git(dev, "push", "-q", "origin", "HEAD:refs/heads/rc/v0.6.3-rc.1", "HEAD:refs/heads/rc/v0.6.2-rc.1", "HEAD:refs/heads/rc/v0.7.0-rc.1");
  assert.strictEqual(release(dev, "rc-branch", "rc/v0.6.3-rc.1").trim(), "v0.6.3-rc.1");
  assert.strictEqual(release(dev, "superseded", "v0.6.3-rc.2").trim(), "rc/v0.6.3-rc.1");
  assert.strictEqual(release(dev, "released", "v0.6.3").trim(), "rc/v0.6.2-rc.1 rc/v0.6.3-rc.1");
  const short = git(dev, "rev-parse", "HEAD").slice(0, 12);
  assert.doesNotThrow(() => release(dev, "check-candidate", `candidate/${short}`));
});

test("the commands of the old flow and of agent-config's stamp are gone", () => {
  const { dev } = released();
  for (const cmd of ["stamp", "prepare", "tag"]) {
    assert.throws(() => release(dev, cmd, "x"), /usage: release\.cjs next \| rc <tag> \| latest \|/, cmd);
  }
});
