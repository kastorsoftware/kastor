import assert from "node:assert/strict";
import test from "node:test";
import { checkVersion } from "./check-version.mjs";

const before = "a".repeat(40);
const currentJson = '{"version":"1.1.0"}';
const options = { event: "push", before, currentJson };

test("compares available versions", () => {
  assert.equal(checkVersion({ ...options, git: () => '{"version":"1.0.9"}' }).changed, true);
  assert.equal(checkVersion({ ...options, git: () => currentJson }).changed, false);
});

test("fetches the detached previous tip after force-push", () => {
  const calls = [];
  const result = checkVersion({ ...options, git: (args) => {
    calls.push(args);
    if (calls.length === 1) throw new Error("unknown revision");
    return args[0] === "fetch" ? "" : '{"version":"1.0.9"}';
  } });
  assert.equal(result.changed, true);
  assert.deepEqual(calls[1], ["fetch", "--no-tags", "--depth=1", "origin", before]);
  assert.equal(calls.length, 3);
});

test("manual and initial pushes need no prior version", () => {
  const git = () => { throw new Error("must not call git"); };
  assert.equal(checkVersion({ ...options, event: "workflow_dispatch", before: undefined, git }).changed, true);
  assert.equal(checkVersion({ ...options, before: "0".repeat(40), git }).changed, true);
});

test("unavailable prior history is an explicit error, not an unchanged version", () => {
  assert.throws(() => checkVersion({ ...options, git: () => { throw new Error("not found"); } }), /Cannot load previous version/);
});

test("empty or malformed JSON and invalid SHAs fail clearly", () => {
  for (const json of ["", "{", "{}", '{"version":null}']) {
    assert.throws(() => checkVersion({ ...options, git: () => json }), /parse app version|Missing app version/);
  }
  assert.throws(() => checkVersion({ ...options, before: "bad", git: () => currentJson }), /valid previous commit/);
  assert.throws(() => checkVersion({ ...options, currentJson: "", git: () => currentJson }), /parse app version/);
});
