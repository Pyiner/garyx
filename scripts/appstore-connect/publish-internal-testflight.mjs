#!/usr/bin/env node
import { createPrivateKey, sign as signBytes } from "node:crypto";
import { readFile } from "node:fs/promises";

const ASC_BASE_URL = "https://api.appstoreconnect.apple.com";
const DEFAULT_TIMEOUT_MS = 30 * 60 * 1000;
const DEFAULT_POLL_MS = 30 * 1000;

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

function optionalNumberEnv(name, fallback) {
  const value = process.env[name];
  if (!value || !value.trim()) {
    return fallback;
  }
  const parsed = Number(value);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
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

async function findOne(path) {
  const response = await ascRequest("GET", path);
  return response.data ?? null;
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

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function findApp({ bundleId }) {
  const app = await findFirst(
    `/v1/apps?filter[bundleId]=${encodeFilter(bundleId)}&limit=1`,
  );
  if (!app) {
    throw new Error(`App not found for bundle ID: ${bundleId}`);
  }
  return app;
}

async function findInternalGroup({ app, groupName }) {
  const groups = await ascRequest(
    "GET",
    `/v1/apps/${app.id}/betaGroups?limit=200`,
  );
  const group =
    groups.data?.find(
      (candidate) =>
        candidate.attributes?.name === groupName &&
        candidate.attributes?.isInternalGroup === true,
    ) ??
    groups.data?.find(
      (candidate) => candidate.attributes?.isInternalGroup === true,
    );
  if (!group) {
    throw new Error(`Internal TestFlight group not found: ${groupName}`);
  }
  return group;
}

async function listBuilds({ app, buildNumber }) {
  const versionFilter = buildNumber
    ? `&filter[version]=${encodeFilter(buildNumber)}`
    : "";
  const response = await ascRequest(
    "GET",
    `/v1/builds?filter[app]=${app.id}${versionFilter}&sort=-uploadedDate&limit=10&include=buildBetaDetail,betaGroups`,
  );
  return response.data ?? [];
}

async function waitForValidBuild({ app, buildNumber, timeoutMs, pollMs }) {
  const startedAt = Date.now();
  let latest = null;
  while (Date.now() - startedAt < timeoutMs) {
    const builds = await listBuilds({ app, buildNumber });
    latest = builds[0] ?? null;
    if (latest?.attributes?.processingState === "VALID") {
      return latest;
    }
    const state = latest?.attributes?.processingState ?? "missing";
    console.log(`Waiting for TestFlight build ${buildNumber}: ${state}`);
    await sleep(pollMs);
  }
  throw new Error(
    `Timed out waiting for TestFlight build ${buildNumber} to become VALID.`,
  );
}

async function addBuildToGroup({ build, group }) {
  const existing = await ascRequest(
    "GET",
    `/v1/betaGroups/${group.id}/relationships/builds?limit=200`,
  );
  if (existing.data?.some((candidate) => candidate.id === build.id)) {
    console.log("Build already assigned to internal TestFlight group.");
    return;
  }

  await ascRequest("POST", `/v1/betaGroups/${group.id}/relationships/builds`, {
    data: [
      {
        id: build.id,
        type: "builds",
      },
    ],
  });
  console.log("Assigned build to internal TestFlight group.");
}

async function main() {
  const bundleId = optionalEnv("IOS_BUNDLE_ID", "com.garyx.mobile");
  const groupName = optionalEnv("TESTFLIGHT_GROUP_NAME", "Garyx Experimental");
  const buildNumber = optionalEnv("GARYX_IOS_BUILD_NUMBER", "");
  const timeoutMs = optionalNumberEnv(
    "TESTFLIGHT_BUILD_WAIT_TIMEOUT_MS",
    DEFAULT_TIMEOUT_MS,
  );
  const pollMs = optionalNumberEnv(
    "TESTFLIGHT_BUILD_WAIT_POLL_MS",
    DEFAULT_POLL_MS,
  );

  const app = await findApp({ bundleId });
  const group = await findInternalGroup({ app, groupName });
  const build = await waitForValidBuild({
    app,
    buildNumber,
    timeoutMs,
    pollMs,
  });
  await addBuildToGroup({ build, group });

  const betaDetail = build.relationships?.buildBetaDetail?.data?.id
    ? await findOne(
        `/v1/buildBetaDetails/${build.relationships.buildBetaDetail.data.id}`,
      )
    : null;
  console.log(
    [
      "Internal TestFlight build ready:",
      `version=${build.attributes?.version}`,
      `processing=${build.attributes?.processingState}`,
      `internal=${betaDetail?.attributes?.internalBuildState ?? "unknown"}`,
    ].join(" "),
  );
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
});
