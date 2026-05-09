import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(scriptDirectory, "..");
const packageJsonPath = resolve(projectRoot, "package.json");
const builderCliPath = resolve(projectRoot, "node_modules", "electron-builder", "cli.js");
const requestedVersion = process.env.GARYX_DESKTOP_VERSION?.trim() || null;
const extraArgs = process.argv.slice(2);
const defaultArgs = ["--mac", "dmg", "zip", "--universal", "--publish", "never"];
const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";

function assertValidVersion(value) {
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(value)) {
    throw new Error(
      `GARYX_DESKTOP_VERSION must be semver-like (received ${JSON.stringify(value)})`,
    );
  }
}

const originalPackageJson = readFileSync(packageJsonPath, "utf8");

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: projectRoot,
    stdio: "inherit",
    env: process.env,
  });
  if (result.error) {
    throw result.error;
  }
  return result.status ?? 1;
}

if (requestedVersion) {
  assertValidVersion(requestedVersion);
  const packageJson = JSON.parse(originalPackageJson);
  packageJson.version = requestedVersion;
  writeFileSync(`${packageJsonPath}`, `${JSON.stringify(packageJson, null, 2)}\n`);
  console.log(`Temporarily set Garyx version to ${requestedVersion}`);
}

let exitCode = 0;

try {
  if (process.env.GARYX_DESKTOP_SKIP_BUILD === "1") {
    console.log("Skipping UI build because GARYX_DESKTOP_SKIP_BUILD=1.");
  } else {
    exitCode = run(npmCommand, ["run", "build:packaged"]);
  }

  const args = [
    builderCliPath,
    ...(extraArgs.length === 0 ? defaultArgs : extraArgs),
  ];
  if (exitCode === 0) {
    exitCode = run(process.execPath, args);
  }
} finally {
  if (requestedVersion) {
    writeFileSync(packageJsonPath, originalPackageJson);
  }
}

if (exitCode !== 0) {
  process.exit(exitCode);
}
