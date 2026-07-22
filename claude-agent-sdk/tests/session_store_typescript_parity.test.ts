import { afterAll, describe, expect, test } from 'bun:test'
import { mkdtempSync, mkdirSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { pathToFileURL } from 'node:url'

const officialRepo = process.env.CLAUDE_TS_SDK_REPO
const officialModule = process.env.CLAUDE_TS_SDK_MODULE
const rustOracle = process.env.CLAUDE_RUST_SESSION_STORE_ORACLE
const rustResumeProbe = process.env.CLAUDE_RUST_SESSION_STORE_RESUME_PROBE
const rustBatchProbe = process.env.CLAUDE_RUST_SESSION_STORE_BATCH_PROBE
const fakeCli = process.env.CLAUDE_SESSION_STORE_FAKE_CLI
const nodeBinary = process.env.CLAUDE_SESSION_STORE_NODE

if (
  !officialRepo ||
  !officialModule ||
  !rustOracle ||
  !rustResumeProbe ||
  !rustBatchProbe ||
  !fakeCli ||
  !nodeBinary
) {
  throw new Error(
    'all Claude SessionStore parity environment variables are required',
  )
}

const { runSessionStoreConformance } = await import(
  pathToFileURL(
    join(officialRepo, 'examples/session-stores/shared/conformance.ts'),
  ).href
)
const officialSdk = await import(pathToFileURL(officialModule).href)
const scratch = mkdtempSync(join(tmpdir(), 'garyx-session-store-parity-'))
let storeCounter = 0

afterAll(() => rmSync(scratch, { recursive: true, force: true }))

type SessionKey = {
  projectKey: string
  sessionId: string
  subpath?: string
}

type SessionStoreEntry = { type: string; [key: string]: unknown }

async function rustCall(root: string, operation: unknown): Promise<unknown> {
  const child = Bun.spawn([rustOracle, root, JSON.stringify(operation)], {
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const [stdout, stderr, exitCode] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ])
  if (exitCode !== 0) {
    throw new Error(`Rust SessionStore oracle failed (${exitCode}): ${stderr}`)
  }
  return JSON.parse(stdout)
}

class RustLocalDirectorySessionStore {
  readonly root: string

  constructor() {
    this.root = join(scratch, `store-${storeCounter++}`)
  }

  async append(key: SessionKey, entries: SessionStoreEntry[]): Promise<void> {
    await rustCall(this.root, { op: 'append', key, entries })
  }

  async load(key: SessionKey): Promise<SessionStoreEntry[] | null> {
    return (await rustCall(this.root, { op: 'load', key })) as
      | SessionStoreEntry[]
      | null
  }

  async listSessions(
    projectKey: string,
  ): Promise<Array<{ sessionId: string; mtime: number }>> {
    return (await rustCall(this.root, {
      op: 'listSessions',
      projectKey,
    })) as Array<{ sessionId: string; mtime: number }>
  }

  async delete(key: SessionKey): Promise<void> {
    await rustCall(this.root, { op: 'delete', key })
  }

  async listSubkeys(key: {
    projectKey: string
    sessionId: string
  }): Promise<string[]> {
    return (await rustCall(this.root, { op: 'listSubkeys', key })) as string[]
  }
}

describe('Rust LocalDirectorySessionStore vs official TS v0.3.217 contract', () => {
  runSessionStoreConformance(() => new RustLocalDirectorySessionStore())
})

describe('project key differential', () => {
  for (const [name, relative] of [
    ['ascii', ['workspace']],
    ['utf16', ['工作区-🧪']],
    ['long', Array.from({ length: 32 }, (_, index) => `segment-${index}`)],
  ] as const) {
    test(name, async () => {
      const path = join(scratch, 'project-keys', ...relative)
      mkdirSync(path, { recursive: true })
      let typescriptKey: string | undefined
      const captureStore = {
        async append() {},
        async load() {
          return null
        },
        async listSessions(projectKey: string) {
          typescriptKey = projectKey
          return []
        },
      }
      await officialSdk.listSessions({ dir: path, sessionStore: captureStore })
      const rustKey = await rustCall(scratch, { op: 'projectKey', path })
      expect(rustKey).toBe(typescriptKey)
    })
  }
})

describe('resume + transcript mirror differential', () => {
  test('materializes the same native session before spawn and mirrors before result', async () => {
    const cwd = join(scratch, 'resume-workspace')
    const profile = join(scratch, 'typescript-profile')
    mkdirSync(cwd, { recursive: true })
    mkdirSync(profile, { recursive: true })

    let projectKey: string | undefined
    await officialSdk.listSessions({
      dir: cwd,
      sessionStore: {
        async append() {},
        async load() {
          return null
        },
        async listSessions(key: string) {
          projectKey = key
          return []
        },
      },
    })
    if (!projectKey) throw new Error('official SDK did not resolve projectKey')

    const sessionId = '11111111-2222-4333-8444-555555555555'
    const entryUuid = '72f50f98-c34b-4e4e-a586-58179c5536f1'
    const store = new officialSdk.InMemorySessionStore()
    const mainKey = { projectKey, sessionId }
    await store.append(mainKey, [
      {
        type: 'user',
        uuid: '0d02bd0d-f6cf-4f87-81c6-849acac8712b',
        sessionId,
        message: { role: 'user', content: 'original turn' },
      },
    ])
    await store.append(
      { ...mainKey, subpath: 'subagents/agent-probe' },
      [
        {
          type: 'assistant',
          uuid: '63f73d62-5ca2-409a-96fe-bf3b36f1ba31',
        },
        { type: 'agent_metadata', toolUseId: 'tool-probe' },
      ],
    )

    let resultValue: unknown
    for await (const message of officialSdk.query({
      prompt: 'continue',
      options: {
        resume: sessionId,
        cwd,
        pathToClaudeCodeExecutable: fakeCli,
        env: {
          ...process.env,
          CLAUDE_CONFIG_DIR: profile,
          PROBE_ENTRY_UUID: entryUuid,
          ANTHROPIC_API_KEY: 'parity-probe',
        },
        sessionStore: store,
      },
    })) {
      if (message.type === 'result') {
        resultValue = JSON.parse(message.result)
      }
    }
    const typescript = {
      result: resultValue,
      entries: await store.load(mainKey),
    }

    const rustScratch = join(scratch, 'rust-resume')
    mkdirSync(rustScratch, { recursive: true })
    const child = Bun.spawn(
      [rustResumeProbe, rustScratch, nodeBinary, fakeCli],
      { stdout: 'pipe', stderr: 'pipe' },
    )
    const [stdout, stderr, exitCode] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
      child.exited,
    ])
    if (exitCode !== 0) {
      throw new Error(`Rust resume probe failed (${exitCode}): ${stderr}`)
    }
    const rust = JSON.parse(stdout)
    expect(rust).toEqual(typescript)
    expect(rust.result).toEqual({
      sessionId,
      mirrorFlag: true,
      initialCount: 1,
      subagentCount: 1,
      metadataToolUseId: 'tool-probe',
    })
    expect(rust.entries).toHaveLength(2)
  })
})

type AppendTrace = {
  key: SessionKey
  entryCount: number
  firstMarker?: unknown
  lastMarker?: unknown
  outcome: 'ok' | 'error'
}

class RecordingSessionStore {
  readonly calls: AppendTrace[] = []
  readonly mainKey: SessionKey
  readonly seed: SessionStoreEntry[]
  failuresRemaining: number

  constructor(mainKey: SessionKey, failures: number) {
    this.mainKey = mainKey
    this.seed = [{ type: 'seed' }]
    this.failuresRemaining = failures
  }

  async append(key: SessionKey, entries: SessionStoreEntry[]): Promise<void> {
    const failed = this.failuresRemaining > 0
    if (failed) this.failuresRemaining -= 1
    this.calls.push({
      key: structuredClone(key),
      entryCount: entries.length,
      firstMarker: entries.at(0)?.marker,
      lastMarker: entries.at(-1)?.marker,
      outcome: failed ? 'error' : 'ok',
    })
    if (failed) throw new Error('intentional parity probe failure')
  }

  async load(key: SessionKey): Promise<SessionStoreEntry[] | null> {
    if (
      key.projectKey === this.mainKey.projectKey &&
      key.sessionId === this.mainKey.sessionId &&
      key.subpath === undefined
    ) {
      return structuredClone(this.seed)
    }
    return null
  }

  async listSubkeys(): Promise<string[]> {
    return []
  }
}

async function officialMirrorTrace(
  scenario: string,
): Promise<{ result: unknown; calls: AppendTrace[]; mirrorErrors: number }> {
  const cwd = join(scratch, `batch-${scenario}-workspace`)
  const profile = join(scratch, `batch-${scenario}-typescript-profile`)
  mkdirSync(cwd, { recursive: true })
  mkdirSync(profile, { recursive: true })

  let projectKey: string | undefined
  await officialSdk.listSessions({
    dir: cwd,
    sessionStore: {
      async append() {},
      async load() {
        return null
      },
      async listSessions(key: string) {
        projectKey = key
        return []
      },
    },
  })
  if (!projectKey) throw new Error('official SDK did not resolve projectKey')

  const sessionId = '11111111-2222-4333-8444-555555555555'
  const store = new RecordingSessionStore(
    { projectKey, sessionId },
    scenario === 'retry' ? 2 : scenario === 'failure' ? 3 : 0,
  )
  let resultValue: unknown
  let mirrorErrors = 0
  for await (const message of officialSdk.query({
    prompt: 'continue',
    options: {
      resume: sessionId,
      cwd,
      pathToClaudeCodeExecutable: fakeCli,
      env: {
        ...process.env,
        CLAUDE_CONFIG_DIR: profile,
        PROBE_SCENARIO: scenario,
        ANTHROPIC_API_KEY: 'parity-probe',
      },
      sessionStore: store,
      sessionStoreFlush: scenario === 'eager' ? 'eager' : 'batched',
    },
  })) {
    if (message.type === 'result') resultValue = JSON.parse(message.result)
    if (message.type === 'system' && message.subtype === 'mirror_error') {
      mirrorErrors += 1
    }
  }
  return { result: resultValue, calls: store.calls, mirrorErrors }
}

async function rustMirrorTrace(scenario: string): Promise<unknown> {
  const rustScratch = join(scratch, `batch-${scenario}-rust`)
  const sharedCwd = join(scratch, `batch-${scenario}-workspace`)
  mkdirSync(rustScratch, { recursive: true })
  const child = Bun.spawn(
    [rustBatchProbe, rustScratch, nodeBinary, fakeCli, scenario, sharedCwd],
    { stdout: 'pipe', stderr: 'pipe' },
  )
  const [stdout, stderr, exitCode] = await Promise.all([
    new Response(child.stdout).text(),
    new Response(child.stderr).text(),
    child.exited,
  ])
  if (exitCode !== 0) {
    throw new Error(`Rust batch probe failed (${exitCode}): ${stderr}`)
  }
  return JSON.parse(stdout)
}

describe('transcript mirror batch differential', () => {
  for (const scenario of [
    'batched',
    'eager',
    'bytes',
    'unsafe',
    'retry',
    'failure',
  ]) {
    test(scenario, async () => {
      const typescript = await officialMirrorTrace(scenario)
      const rust = await rustMirrorTrace(scenario)
      expect(rust).toEqual(typescript)

      if (scenario === 'batched') {
        expect(typescript.calls.map(call => call.entryCount)).toEqual([
          501, 2, 1,
        ])
      } else if (scenario === 'eager') {
        expect(typescript.calls.map(call => call.entryCount)).toEqual([1, 1, 1])
      } else if (scenario === 'bytes') {
        expect(typescript.calls.map(call => call.entryCount)).toEqual([2])
      } else if (scenario === 'unsafe') {
        expect(typescript.calls.map(call => call.firstMarker)).toEqual(['valid'])
      } else if (scenario === 'retry') {
        expect(typescript.calls.map(call => call.outcome)).toEqual([
          'error',
          'error',
          'ok',
        ])
      } else if (scenario === 'failure') {
        expect(typescript.calls.map(call => call.outcome)).toEqual([
          'error',
          'error',
          'error',
        ])
        expect(typescript.mirrorErrors).toBe(1)
      }
    }, 20_000)
  }
})
