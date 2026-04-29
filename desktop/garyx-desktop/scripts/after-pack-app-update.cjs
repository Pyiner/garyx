const { mkdir, readFile, writeFile } = require("node:fs/promises");
const { dirname, join, resolve } = require("node:path");

function yamlScalar(value) {
  if (typeof value === "string") {
    return JSON.stringify(value);
  }
  if (typeof value === "boolean" || typeof value === "number") {
    return String(value);
  }
  return JSON.stringify(value);
}

function updateConfigYaml(config) {
  const preferredOrder = [
    "provider",
    "owner",
    "repo",
    "channel",
    "updaterCacheDirName",
  ];
  const keys = [
    ...preferredOrder.filter((key) => Object.prototype.hasOwnProperty.call(config, key)),
    ...Object.keys(config)
      .filter((key) => !preferredOrder.includes(key))
      .sort(),
  ];
  return `${keys.map((key) => `${key}: ${yamlScalar(config[key])}`).join("\n")}\n`;
}

function firstPublishConfig(buildConfig) {
  const publish = buildConfig?.publish;
  if (Array.isArray(publish)) {
    return publish.find((entry) => entry && typeof entry === "object") || null;
  }
  if (publish && typeof publish === "object") {
    return publish;
  }
  return null;
}

async function ensureAppUpdateConfig(context = {}) {
  if (context.electronPlatformName && context.electronPlatformName !== "darwin") {
    return;
  }

  const appOutDir = context.appOutDir;
  if (!appOutDir) {
    throw new Error("after-pack-app-update requires context.appOutDir");
  }

  const projectDir = context.packager?.projectDir || resolve(__dirname, "..");
  const packageJson = JSON.parse(
    await readFile(join(projectDir, "package.json"), "utf8"),
  );
  const publish = firstPublishConfig(context.packager?.config || packageJson.build);
  if (!publish) {
    return;
  }

  const resourcesDir = join(appOutDir, "Contents", "Resources");
  const updaterCacheDirName =
    context.packager?.appInfo?.updaterCacheDirName
    || `${String(packageJson.name || "garyx-desktop").toLowerCase()}-updater`;
  const updateConfig = {
    ...publish,
    updaterCacheDirName,
  };

  delete updateConfig.publishAutoUpdate;
  await mkdir(resourcesDir, { recursive: true });
  await writeFile(
    join(resourcesDir, "app-update.yml"),
    updateConfigYaml(updateConfig),
    "utf8",
  );
}

module.exports = ensureAppUpdateConfig;
