import { existsSync } from "node:fs";
import { join } from "node:path";
import { notarize } from "@electron/notarize";

function nonEmptyEnv(name) {
  const value = process.env[name]?.trim();
  return value ? value : null;
}

function requireNotarization() {
  return process.env.REQUIRE_MACOS_NOTARIZATION === "1";
}

function skipNotarization() {
  return process.env.SKIP_NOTARIZATION === "1";
}

export default async function afterSign(context) {
  if (context.electronPlatformName !== "darwin") {
    return;
  }

  if (skipNotarization()) {
    console.log("Skipping notarization because SKIP_NOTARIZATION=1.");
    return;
  }

  const appName = context.packager.appInfo.productFilename;
  const appPath = join(context.appOutDir, `${appName}.app`);
  if (!existsSync(appPath)) {
    throw new Error(`Expected packaged app at ${appPath}`);
  }

  const appleId = nonEmptyEnv("APPLE_ID");
  const appleIdPassword = nonEmptyEnv("APPLE_APP_SPECIFIC_PASSWORD");
  const teamId = nonEmptyEnv("APPLE_TEAM_ID");

  if (!(appleId && appleIdPassword && teamId)) {
    const message =
      "Skipping notarization because Apple notarization credentials are missing.";
    if (requireNotarization()) {
      throw new Error(
        `${message} Set APPLE_ID, APPLE_APP_SPECIFIC_PASSWORD, and APPLE_TEAM_ID.`,
      );
    }
    console.log(message);
    return;
  }

  console.log(`Submitting ${appPath} for notarization...`);
  await notarize({ appPath, appleId, appleIdPassword, teamId });
  console.log(`Notarization completed for ${appPath}`);
}
