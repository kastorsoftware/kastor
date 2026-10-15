import { execFileSync } from "node:child_process";
import { appendFileSync, readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const configPath = "src-tauri/tauri.conf.json";

function readVersion(json, label) {
  let version;
  try {
    version = JSON.parse(json).version;
  } catch {
    throw new Error(`Cannot parse app version from ${label}.`);
  }
  if (typeof version !== "string" || !version.trim()) {
    throw new Error(`Missing app version in ${label}.`);
  }
  return version;
}

export function checkVersion({ event, before, currentJson, git }) {
  const current = readVersion(currentJson, "working tree");
  if (event === "workflow_dispatch") return { changed: true, current };
  if (!/^[0-9a-f]{40}$/i.test(before || "")) {
    throw new Error("Push event has no valid previous commit SHA.");
  }
  if (/^0{40}$/.test(before)) return { changed: true, current };
  const previousSpec = `${before}:${configPath}`;
  let previousJson;
  try {
    previousJson = git(["show", previousSpec]);
  } catch {
    // A force-push can detach the previous tip from all fetched branches.
    try {
      git(["fetch", "--no-tags", "--depth=1", "origin", before]);
      previousJson = git(["show", previousSpec]);
    } catch {
      throw new Error(`Cannot load previous version at ${before}. Use workflow_dispatch to publish explicitly.`);
    }
  }
  const previous = readVersion(previousJson, `commit ${before}`);
  return { changed: current !== previous, current, previous };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const result = checkVersion({
      event: process.env.EVENT_NAME,
      before: process.env.BEFORE_SHA,
      currentJson: readFileSync(configPath, "utf8"),
      git: (args) => execFileSync("git", args, { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }),
    });
    console.log(`App version: ${result.previous ?? "explicit/initial run"} -> ${result.current}; changed=${result.changed}`);
    appendFileSync(process.env.GITHUB_OUTPUT, `changed=${result.changed}\n`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
