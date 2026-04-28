import { existsSync, readdirSync, statSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(scriptDirectory, "..");
const distRoot = join(projectRoot, "dist-release");
const providedAppPath = process.argv[2] ? resolve(process.argv[2]) : null;
const PRODUCT_NAME = "Garyx.app";

function run(command, args) {
  const result = spawnSync(command, args, { stdio: "inherit" });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function findLatestBuiltApp() {
  if (!existsSync(distRoot)) {
    throw new Error(`Build output directory does not exist: ${distRoot}`);
  }

  const candidates = readdirSync(distRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => join(distRoot, entry.name, PRODUCT_NAME))
    .filter((candidate) => existsSync(candidate))
    .sort((left, right) => statSync(right).mtimeMs - statSync(left).mtimeMs);

  if (candidates.length === 0) {
    throw new Error(`Could not find ${PRODUCT_NAME} under ${distRoot}`);
  }

  return candidates[0];
}

const appPath = providedAppPath ?? findLatestBuiltApp();

console.log(`Verifying ${appPath}`);
run("codesign", ["-dv", "--verbose=4", appPath]);
run("spctl", ["-a", "-vvv", appPath]);
run("xcrun", ["stapler", "validate", appPath]);
