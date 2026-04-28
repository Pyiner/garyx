import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
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

function resolveApiKeyFile(rawValue) {
  if (!rawValue) {
    return { path: null, cleanup: null };
  }

  if (existsSync(rawValue)) {
    return { path: rawValue, cleanup: null };
  }

  let decoded = rawValue;
  if (!decoded.includes("BEGIN PRIVATE KEY")) {
    try {
      decoded = Buffer.from(rawValue, "base64").toString("utf8");
    } catch {
      decoded = rawValue;
    }
  }

  if (!decoded.includes("BEGIN PRIVATE KEY")) {
    throw new Error(
      "APPLE_API_KEY must be an absolute path, raw .p8 contents, or base64-encoded .p8 contents",
    );
  }

  const tempDirectory = mkdtempSync(join(tmpdir(), "garyx-desktop-notary-"));
  const apiKeyPath = join(tempDirectory, "AuthKey_GARY_DESKTOP.p8");
  writeFileSync(apiKeyPath, decoded, { mode: 0o600 });
  return {
    path: apiKeyPath,
    cleanup: () => rmSync(tempDirectory, { recursive: true, force: true }),
  };
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

  const keychainProfile = nonEmptyEnv("APPLE_KEYCHAIN_PROFILE");
  const appleId = nonEmptyEnv("APPLE_ID");
  const appleIdPassword = nonEmptyEnv("APPLE_APP_SPECIFIC_PASSWORD");
  const teamId = nonEmptyEnv("APPLE_TEAM_ID");
  const appleApiKeyId = nonEmptyEnv("APPLE_API_KEY_ID");
  const appleApiIssuer = nonEmptyEnv("APPLE_API_ISSUER");
  const appleApiKeyValue = nonEmptyEnv("APPLE_API_KEY");

  let options = null;
  let cleanup = null;

  if (keychainProfile) {
    options = { keychainProfile };
  } else if (appleApiKeyId && appleApiIssuer && appleApiKeyValue) {
    const resolved = resolveApiKeyFile(appleApiKeyValue);
    cleanup = resolved.cleanup;
    options = {
      appleApiKey: resolved.path,
      appleApiKeyId,
      appleApiIssuer,
    };
  } else if (appleId && appleIdPassword && teamId) {
    options = {
      appleId,
      appleIdPassword,
      teamId,
    };
  }

  if (!options) {
    const message =
      "Skipping notarization because no valid Apple notarization credentials were configured.";
    if (requireNotarization()) {
      throw new Error(
        `${message} Configure APPLE_KEYCHAIN_PROFILE, or APPLE_API_KEY + APPLE_API_KEY_ID + APPLE_API_ISSUER, or APPLE_ID + APPLE_APP_SPECIFIC_PASSWORD + APPLE_TEAM_ID.`,
      );
    }
    console.log(message);
    return;
  }

  try {
    console.log(`Submitting ${appPath} for notarization...`);
    await notarize({
      appPath,
      ...options,
    });
    console.log(`Notarization completed for ${appPath}`);
  } finally {
    cleanup?.();
  }
}
