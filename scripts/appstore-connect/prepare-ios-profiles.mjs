#!/usr/bin/env node
import assert from "node:assert/strict";
import { createPrivateKey, sign as signBytes } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";

const ASC_BASE_URL = "https://api.appstoreconnect.apple.com";

function requiredEnv(name, aliases = []) {
  for (const key of [name, ...aliases]) {
    const value = process.env[key];
    if (value && value.trim()) {
      return value.trim();
    }
  }
  throw new Error(`Missing required environment variable: ${name}`);
}

function optionalEnv(name, fallback) {
  const value = process.env[name];
  return value && value.trim() ? value.trim() : fallback;
}

function base64url(value) {
  return Buffer.from(value).toString("base64url");
}

function readDerLength(bytes, offset) {
  let length = bytes[offset++];
  if (length < 0x80) {
    return { length, offset };
  }
  const octets = length & 0x7f;
  length = 0;
  for (let index = 0; index < octets; index += 1) {
    length = (length << 8) | bytes[offset++];
  }
  return { length, offset };
}

function derEcdsaSignatureToJose(signature) {
  let offset = 0;
  if (signature[offset++] !== 0x30) {
    throw new Error("Invalid ECDSA DER signature.");
  }
  ({ offset } = readDerLength(signature, offset));

  const parts = [];
  for (let partIndex = 0; partIndex < 2; partIndex += 1) {
    if (signature[offset++] !== 0x02) {
      throw new Error("Invalid ECDSA DER integer.");
    }
    const parsed = readDerLength(signature, offset);
    offset = parsed.offset;
    let value = signature.subarray(offset, offset + parsed.length);
    offset += parsed.length;
    while (value.length > 32 && value[0] === 0) {
      value = value.subarray(1);
    }
    if (value.length > 32) {
      throw new Error("ECDSA signature integer is too large for ES256.");
    }
    parts.push(Buffer.concat([Buffer.alloc(32 - value.length), value]));
  }
  return Buffer.concat(parts).toString("base64url");
}

async function loadPrivateKey() {
  if (process.env.APP_STORE_CONNECT_API_KEY_P8) {
    return process.env.APP_STORE_CONNECT_API_KEY_P8.replace(/\\n/g, "\n");
  }
  const keyPath = requiredEnv("APP_STORE_CONNECT_API_KEY_PATH", [
    "ASC_PRIVATE_KEY_PATH",
  ]);
  return readFile(keyPath, "utf8");
}

async function createJwt() {
  const keyId = requiredEnv("APP_STORE_CONNECT_API_KEY_ID", ["ASC_KEY_ID"]);
  const issuerId = requiredEnv("APP_STORE_CONNECT_API_ISSUER_ID", [
    "ASC_ISSUER_ID",
  ]);
  const privateKey = createPrivateKey(await loadPrivateKey());
  const now = Math.floor(Date.now() / 1000);
  const header = { alg: "ES256", kid: keyId, typ: "JWT" };
  const payload = {
    aud: "appstoreconnect-v1",
    exp: now + 20 * 60,
    iss: issuerId,
  };
  const signingInput = `${base64url(JSON.stringify(header))}.${base64url(
    JSON.stringify(payload),
  )}`;
  const derSignature = signBytes("sha256", Buffer.from(signingInput), privateKey);
  return `${signingInput}.${derEcdsaSignatureToJose(derSignature)}`;
}

let token;

function sanitizeForLog(value) {
  return String(value).replace(
    /[A-Z0-9._%+-]+@[A-Z0-9.-]+/gi,
    "[email]",
  );
}

async function ascRequest(method, path, body) {
  token ??= await createJwt();
  const response = await fetch(`${ASC_BASE_URL}${path}`, {
    method,
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (!response.ok) {
    const text = await response.text();
    throw new Error(
      `${method} ${path} failed with ${response.status}: ${sanitizeForLog(
        text,
      )}`,
    );
  }
  if (response.status === 204) {
    return null;
  }
  return response.json();
}

function encodeFilter(value) {
  return encodeURIComponent(value);
}

function normalizeSerial(value) {
  return String(value ?? "")
    .replace(/[^a-fA-F0-9]/g, "")
    .toUpperCase();
}

async function findBundleId(identifier) {
  const response = await ascRequest(
    "GET",
    `/v1/bundleIds?filter[identifier]=${encodeFilter(identifier)}&limit=200`,
  );
  const bundleId = response.data?.find(
    (candidate) => candidate.attributes?.identifier === identifier,
  );
  if (!bundleId) {
    throw new Error(`Bundle ID not found: ${identifier}`);
  }
  return bundleId;
}

export async function ensurePushNotificationsCapability(
  bundleId,
  request = ascRequest,
) {
  const response = await request(
    "GET",
    `/v1/bundleIds/${encodeURIComponent(
      bundleId.id,
    )}/bundleIdCapabilities?limit=200`,
  );
  const alreadyEnabled = response.data?.some(
    (capability) =>
      capability.attributes?.capabilityType === "PUSH_NOTIFICATIONS",
  );
  if (alreadyEnabled) {
    return { created: false };
  }

  await request("POST", "/v1/bundleIdCapabilities", {
    data: {
      type: "bundleIdCapabilities",
      attributes: {
        capabilityType: "PUSH_NOTIFICATIONS",
      },
      relationships: {
        bundleId: {
          data: {
            id: bundleId.id,
            type: "bundleIds",
          },
        },
      },
    },
  });
  return { created: true };
}

async function findDistributionCertificate(serial) {
  const normalizedSerial = normalizeSerial(serial);
  if (!normalizedSerial) {
    throw new Error("Distribution certificate serial is empty.");
  }

  const response = await ascRequest(
    "GET",
    "/v1/certificates?filter[certificateType]=IOS_DISTRIBUTION&limit=200",
  );
  const certificate = response.data?.find(
    (candidate) =>
      normalizeSerial(candidate.attributes?.serialNumber) === normalizedSerial,
  );
  if (!certificate) {
    throw new Error(
      "Imported distribution certificate was not found in App Store Connect.",
    );
  }
  return certificate;
}

async function createProfile({ name, bundleId, certificate }) {
  const response = await ascRequest("POST", "/v1/profiles", {
    data: {
      type: "profiles",
      attributes: {
        name,
        profileType: "IOS_APP_STORE",
      },
      relationships: {
        bundleId: {
          data: {
            id: bundleId.id,
            type: "bundleIds",
          },
        },
        certificates: {
          data: [
            {
              id: certificate.id,
              type: "certificates",
            },
          ],
        },
      },
    },
  });
  return response.data;
}

async function ensureProfile({
  label,
  bundleIdentifier,
  bundleId,
  certificate,
  outputDir,
}) {
  const runId = optionalEnv("GITHUB_RUN_ID", String(Date.now()));
  const attempt = optionalEnv("GITHUB_RUN_ATTEMPT", "1");
  const name = `Garyx ${label} App Store ${runId}.${attempt}`;
  const resolvedBundleId = bundleId ?? (await findBundleId(bundleIdentifier));
  const profile = await createProfile({
    name,
    bundleId: resolvedBundleId,
    certificate,
  });
  const content = profile.attributes?.profileContent;
  const uuid = profile.attributes?.uuid;
  if (!content || !uuid) {
    throw new Error(`App Store Connect did not return profile content for ${label}.`);
  }

  await mkdir(outputDir, { recursive: true });
  const profilePath = join(outputDir, `${uuid}.mobileprovision`);
  await writeFile(profilePath, Buffer.from(content, "base64"));
  console.log(`Created ${label} App Store provisioning profile: ${name}`);
  return { name, path: profilePath, uuid };
}

async function appendEnv(values) {
  const envPath = requiredEnv("GITHUB_ENV");
  const lines = Object.entries(values).map(([key, value]) => `${key}=${value}`);
  await mkdir(dirname(envPath), { recursive: true });
  await writeFile(envPath, `${lines.join("\n")}\n`, { flag: "a" });
}

async function main() {
  const appBundleIdentifier = optionalEnv("IOS_BUNDLE_ID", "com.garyx.mobile");
  const widgetBundleIdentifier = optionalEnv(
    "IOS_WIDGET_BUNDLE_ID",
    `${appBundleIdentifier}.RecentThreadsWidget`,
  );
  const outputDir = requiredEnv("IOS_PROFILE_INSTALL_DIR");
  const certificate = await findDistributionCertificate(
    requiredEnv("IOS_DISTRIBUTION_CERTIFICATE_SERIAL"),
  );

  const appBundleId = await findBundleId(appBundleIdentifier);
  const capability = await ensurePushNotificationsCapability(appBundleId);
  console.log(
    capability.created
      ? `Enabled PUSH_NOTIFICATIONS for ${appBundleIdentifier}.`
      : `PUSH_NOTIFICATIONS already enabled for ${appBundleIdentifier}.`,
  );
  const appProfile = await ensureProfile({
    label: "Mobile",
    bundleIdentifier: appBundleIdentifier,
    bundleId: appBundleId,
    certificate,
    outputDir,
  });
  // The widget intentionally does not receive a push capability.
  const widgetProfile = await ensureProfile({
    label: "Widget",
    bundleIdentifier: widgetBundleIdentifier,
    certificate,
    outputDir,
  });

  await appendEnv({
    IOS_APP_PROVISIONING_PROFILE_SPECIFIER: appProfile.name,
    IOS_APP_PROVISIONING_PROFILE_PATH: appProfile.path,
    IOS_WIDGET_PROVISIONING_PROFILE_SPECIFIER: widgetProfile.name,
    IOS_WIDGET_PROVISIONING_PROFILE_PATH: widgetProfile.path,
  });
}

async function dryRun() {
  const bundleId = {
    id: "TEST_BUNDLE_ID",
    type: "bundleIds",
    attributes: { identifier: "com.garyx.mobile" },
  };
  const existingCalls = [];
  const existing = await ensurePushNotificationsCapability(
    bundleId,
    async (method, path, body) => {
      existingCalls.push({ method, path, body });
      return {
        data: [
          {
            type: "bundleIdCapabilities",
            attributes: { capabilityType: "PUSH_NOTIFICATIONS" },
          },
        ],
      };
    },
  );
  assert.deepEqual(existing, { created: false });
  assert.equal(existingCalls.length, 1);
  assert.equal(existingCalls[0].method, "GET");

  const missingCalls = [];
  const missing = await ensurePushNotificationsCapability(
    bundleId,
    async (method, path, body) => {
      missingCalls.push({ method, path, body });
      return method === "GET" ? { data: [] } : { data: { id: "CAPABILITY" } };
    },
  );
  assert.deepEqual(missing, { created: true });
  assert.equal(missingCalls.length, 2);
  assert.equal(missingCalls[0].method, "GET");
  assert.deepEqual(missingCalls[1], {
    method: "POST",
    path: "/v1/bundleIdCapabilities",
    body: {
      data: {
        type: "bundleIdCapabilities",
        attributes: { capabilityType: "PUSH_NOTIFICATIONS" },
        relationships: {
          bundleId: {
            data: { id: "TEST_BUNDLE_ID", type: "bundleIds" },
          },
        },
      },
    },
  });
  console.log(
    "Dry run passed: existing capability is a no-op; missing capability is created exactly once for the app bundle.",
  );
}

if (process.argv.includes("--dry-run")) {
  await dryRun();
} else {
  await main();
}
