import { existsSync, readdirSync, rmSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const PRODUCT_NAME = "Garyx.app";
const APPLICATIONS_TARGET = join("/Applications", PRODUCT_NAME);
const LEGACY_APPLICATION_TARGETS = [
  join("/Applications", "Garyx Desktop.app"),
  join("/Applications", "Gary Desktop.app"),
];
const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const projectRoot = resolve(scriptDirectory, "..");
const distRoot = join(projectRoot, "dist-release");

function isPackagedAppBundle(appPath) {
  return (
    existsSync(join(appPath, "Contents", "Resources", "app.asar")) ||
    existsSync(join(appPath, "Contents", "Resources", "app", "package.json"))
  );
}

if (process.platform !== "darwin") {
  console.log("Skipping app install because this host is not macOS.");
  process.exit(0);
}

if (process.env.SKIP_INSTALL_TO_APPLICATIONS === "1") {
  console.log("Skipping app install because SKIP_INSTALL_TO_APPLICATIONS=1.");
  process.exit(0);
}

function findBuiltApp() {
  if (!existsSync(distRoot)) {
    throw new Error(`Build output directory does not exist: ${distRoot}`);
  }

  const buildRoots = readdirSync(distRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && !entry.name.includes(".bak-"))
    .map((entry) => join(distRoot, entry.name));
  const preferredCandidates = buildRoots
    .map((root) => join(root, PRODUCT_NAME))
    .filter(
      (candidate) => existsSync(candidate) && isPackagedAppBundle(candidate),
    )
    .sort((left, right) => statSync(right).mtimeMs - statSync(left).mtimeMs);

  if (preferredCandidates.length) {
    return preferredCandidates[0];
  }

  const fallbackCandidates = buildRoots
    .flatMap((root) =>
      readdirSync(root, { withFileTypes: true })
        .filter((entry) => entry.isDirectory() && entry.name.endsWith(".app"))
        .map((entry) => join(root, entry.name)),
    )
    .filter((candidate) => isPackagedAppBundle(candidate))
    .sort((left, right) => statSync(right).mtimeMs - statSync(left).mtimeMs);

  if (!fallbackCandidates.length) {
    throw new Error(`Could not find ${PRODUCT_NAME} under ${distRoot}`);
  }

  console.warn(
    `Falling back to ${fallbackCandidates[0]} and installing it as ${PRODUCT_NAME}.`,
  );
  return fallbackCandidates[0];
}

const sourceApp = findBuiltApp();

for (const target of [APPLICATIONS_TARGET, ...LEGACY_APPLICATION_TARGETS]) {
  rmSync(target, { recursive: true, force: true });
}
const installResult = spawnSync("ditto", [sourceApp, APPLICATIONS_TARGET], {
  stdio: "inherit",
});

if (installResult.status !== 0) {
  throw new Error(
    `ditto failed with exit code ${installResult.status ?? "unknown"}`,
  );
}

console.log(`Installed ${PRODUCT_NAME} to ${APPLICATIONS_TARGET}`);
