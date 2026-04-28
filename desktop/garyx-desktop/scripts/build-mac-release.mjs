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

function assertValidVersion(value) {
  if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(value)) {
    throw new Error(
      `GARYX_DESKTOP_VERSION must be semver-like (received ${JSON.stringify(value)})`,
    );
  }
}

const originalPackageJson = readFileSync(packageJsonPath, "utf8");

if (requestedVersion) {
  assertValidVersion(requestedVersion);
  const packageJson = JSON.parse(originalPackageJson);
  packageJson.version = requestedVersion;
  writeFileSync(`${packageJsonPath}`, `${JSON.stringify(packageJson, null, 2)}\n`);
  console.log(`Temporarily set Garyx version to ${requestedVersion}`);
}

try {
  const args = [
    builderCliPath,
    ...(extraArgs.length === 0 ? defaultArgs : extraArgs),
  ];
  const result = spawnSync(process.execPath, args, {
    cwd: projectRoot,
    stdio: "inherit",
    env: process.env,
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.exitCode = result.status ?? 1;
  }
} finally {
  if (requestedVersion) {
    writeFileSync(packageJsonPath, originalPackageJson);
  }
}
