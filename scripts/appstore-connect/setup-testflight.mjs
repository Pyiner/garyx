#!/usr/bin/env node
import { createPrivateKey, sign as signBytes } from "node:crypto";
import { readFile } from "node:fs/promises";

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

async function ascRequest(method, path, body, options = {}) {
  token ??= await createJwt();
  const response = await fetch(`${ASC_BASE_URL}${path}`, {
    method,
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: body ? JSON.stringify(body) : undefined,
  });
  if (options.allowStatuses?.includes(response.status)) {
    return null;
  }
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

async function findFirst(path) {
  const response = await ascRequest("GET", path);
  return response.data?.[0] ?? null;
}

function encodeFilter(value) {
  return encodeURIComponent(value);
}

function sanitizeForLog(value) {
  return String(value).replace(
    /[A-Z0-9._%+-]+@[A-Z0-9.-]+/gi,
    "[email]",
  );
}

async function ensureBundleId({ appName, bundleId }) {
  const existing = await findFirst(
    `/v1/bundleIds?filter[identifier]=${encodeFilter(bundleId)}&limit=1`,
  );
  if (existing) {
    console.log(`Bundle ID exists: ${bundleId}`);
    return existing;
  }
  const created = await ascRequest("POST", "/v1/bundleIds", {
    data: {
      type: "bundleIds",
      attributes: {
        identifier: bundleId,
        name: appName,
        platform: "IOS",
      },
    },
  });
  console.log(`Created Bundle ID: ${bundleId}`);
  return created.data;
}

async function ensureApp({ appName, bundleId, sku, primaryLocale }) {
  const byBundleId = await findFirst(
    `/v1/apps?filter[bundleId]=${encodeFilter(bundleId)}&limit=1`,
  );
  if (byBundleId) {
    console.log(`App exists: ${appName} (${bundleId})`);
    return byBundleId;
  }

  const bySku = await findFirst(
    `/v1/apps?filter[sku]=${encodeFilter(sku)}&limit=1`,
  );
  if (bySku) {
    console.log(`App exists with SKU: ${sku}`);
    return bySku;
  }

  const created = await ascRequest("POST", "/v1/apps", {
    data: {
      type: "apps",
      attributes: {
        bundleId,
        name: appName,
        primaryLocale,
        sku,
      },
    },
  });
  console.log(`Created app: ${appName}`);
  return created.data;
}

async function ensureInternalBetaGroup({ app, groupName }) {
  const groups = await ascRequest(
    "GET",
    `/v1/apps/${app.id}/betaGroups?limit=200`,
  );
  const existing = groups.data?.find(
    (group) =>
      group.attributes?.name === groupName &&
      group.attributes?.isInternalGroup === true,
  );
  if (existing) {
    console.log(`Internal beta group exists: ${existing.attributes.name}`);
    return existing;
  }

  const fallback = groups.data?.find(
    (group) => group.attributes?.isInternalGroup === true,
  );
  if (fallback) {
    console.log(`Internal beta group exists: ${fallback.attributes.name}`);
    return fallback;
  }

  const created = await ascRequest("POST", "/v1/betaGroups", {
    data: {
      type: "betaGroups",
      attributes: {
        feedbackEnabled: true,
        isInternalGroup: true,
        name: groupName,
      },
      relationships: {
        app: {
          data: {
            id: app.id,
            type: "apps",
          },
        },
      },
    },
  });
  console.log(`Created internal beta group: ${groupName}`);
  return created.data;
}

async function ensureTester({ email, group }) {
  const existing = await findFirst(
    `/v1/betaTesters?filter[email]=${encodeFilter(email)}&limit=1`,
  );
  if (existing) {
    console.log("Beta tester exists: [email]");
    return existing;
  }

  const created = await ascRequest("POST", "/v1/betaTesters", {
    data: {
      type: "betaTesters",
      attributes: {
        email,
        firstName: "Garyx",
        lastName: "Tester",
      },
      relationships: {
        betaGroups: {
          data: [
            {
              id: group.id,
              type: "betaGroups",
            },
          ],
        },
      },
    },
  });
  console.log("Created beta tester: [email]");
  return created.data;
}

async function addTesterToGroup({ tester, group }) {
  const groups = await ascRequest(
    "GET",
    `/v1/betaTesters/${tester.id}/relationships/betaGroups?limit=200`,
  );
  if (groups.data?.some((existing) => existing.id === group.id)) {
    console.log("Beta tester already in internal group: [email]");
    return;
  }

  try {
    await ascRequest(
      "POST",
      `/v1/betaGroups/${group.id}/relationships/betaTesters`,
      {
        data: [
          {
            id: tester.id,
            type: "betaTesters",
          },
        ],
      },
    );
  } catch (error) {
    if (
      error instanceof Error &&
      error.message.includes("Tester(s) cannot be assigned")
    ) {
      console.warn("Skipped beta tester that Apple cannot assign: [email]");
      return;
    }
    throw error;
  }
  console.log("Added beta tester to internal group: [email]");
}

function testerEmails() {
  return optionalEnv("TESTFLIGHT_TESTER_EMAILS", "")
    .split(",")
    .map((email) => email.trim())
    .filter(Boolean);
}

async function main() {
  const appName = optionalEnv("GARYX_APP_NAME", "Garyx");
  const bundleId = optionalEnv("IOS_BUNDLE_ID", "com.garyx.mobile");
  const sku = optionalEnv("GARYX_APP_SKU", "garyx-ios");
  const primaryLocale = optionalEnv("APP_STORE_PRIMARY_LOCALE", "en-US");
  const groupName = optionalEnv("TESTFLIGHT_GROUP_NAME", "Garyx Experimental");

  await ensureBundleId({ appName, bundleId });
  const app = await ensureApp({ appName, bundleId, sku, primaryLocale });
  const group = await ensureInternalBetaGroup({ app, groupName });

  for (const email of testerEmails()) {
    const tester = await ensureTester({ email, group });
    await addTesterToGroup({ tester, group });
  }

  console.log("App Store Connect TestFlight setup complete.");
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
