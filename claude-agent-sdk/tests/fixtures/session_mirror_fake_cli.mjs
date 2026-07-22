import { appendFileSync, existsSync, readFileSync, readdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { createInterface } from 'node:readline'

function optionValue(name) {
  const exact = `--${name}`
  const prefix = `${exact}=`
  const withEquals = process.argv.find(argument => argument.startsWith(prefix))
  if (withEquals) return withEquals.slice(prefix.length)
  const index = process.argv.indexOf(exact)
  return index >= 0 ? process.argv[index + 1] : undefined
}

function emit(value) {
  process.stdout.write(`${JSON.stringify(value)}\n`)
}

const sessionId = optionValue('resume')
if (!sessionId) {
  process.stderr.write('fake CLI requires --resume\n')
  process.exit(2)
}

const configDir = process.env.CLAUDE_CONFIG_DIR
if (!configDir) {
  process.stderr.write('fake CLI requires CLAUDE_CONFIG_DIR\n')
  process.exit(3)
}

const projectsRoot = join(configDir, 'projects')
const projectKey = readdirSync(projectsRoot, { withFileTypes: true }).find(
  entry => entry.isDirectory(),
)?.name
if (!projectKey) {
  process.stderr.write('resume transcript was not materialized before spawn\n')
  process.exit(4)
}

const mainPath = join(projectsRoot, projectKey, `${sessionId}.jsonl`)
if (!existsSync(mainPath)) {
  process.stderr.write('resume transcript was not materialized before spawn\n')
  process.exit(5)
}

const initialEntries = readFileSync(mainPath, 'utf8')
  .split('\n')
  .filter(Boolean)
  .map(line => JSON.parse(line))
const subagentPath = join(
  projectsRoot,
  projectKey,
  sessionId,
  'subagents',
  'agent-probe.jsonl',
)
const metadataPath = join(
  projectsRoot,
  projectKey,
  sessionId,
  'subagents',
  'agent-probe.meta.json',
)
const subagentEntries = existsSync(subagentPath)
  ? readFileSync(subagentPath, 'utf8').split('\n').filter(Boolean)
  : []
const metadata = existsSync(metadataPath)
  ? JSON.parse(readFileSync(metadataPath, 'utf8'))
  : null

let completed = false
const input = createInterface({ input: process.stdin, crlfDelay: Infinity })
input.on('line', line => {
  const message = JSON.parse(line)
  if (message.type === 'control_request') {
    emit({
      type: 'control_response',
      response: {
        subtype: 'success',
        request_id: message.request_id,
        response: {},
      },
    })
    return
  }
  if (message.type !== 'user' || completed) return
  completed = true
  const scenario = process.env.PROBE_SCENARIO ?? 'resume'
  if (scenario !== 'resume') {
    const mirror = (filePath, entries) => {
      emit({ type: 'transcript_mirror', filePath, entries })
    }
    const marked = marker => ({ type: 'probe', marker })
    const emitScenarioResult = () => {
      emit({
        type: 'result',
        subtype: 'success',
        is_error: false,
        duration_ms: 1,
        duration_api_ms: 1,
        num_turns: 1,
        total_cost_usd: 0,
        session_id: sessionId,
        result: JSON.stringify({
          scenario,
          mirrorFlag: process.argv.includes('--session-mirror'),
        }),
      })
    }
    const subagentMirrorPath = join(
      projectsRoot,
      projectKey,
      sessionId,
      'subagents',
      'agent-batch.jsonl',
    )

    if (scenario === 'batched') {
      mirror(
        mainPath,
        Array.from({ length: 250 }, (_, index) => marked(index + 1)),
      )
      mirror(
        mainPath,
        Array.from({ length: 250 }, (_, index) => marked(index + 251)),
      )
      mirror(mainPath, [marked(501)])
      mirror(subagentMirrorPath, [marked('sub-1'), marked('sub-2')])
      mirror(mainPath, [marked(502)])
    } else if (scenario === 'eager') {
      mirror(mainPath, [marked(1)])
      mirror(mainPath, [marked(2)])
      mirror(subagentMirrorPath, [marked(3)])
    } else if (scenario === 'eager-background') {
      mirror(mainPath, [marked('background')])
      setTimeout(() => {
        emit({
          type: 'assistant',
          session_id: sessionId,
          message: {
            model: 'probe',
            content: [{ type: 'text', text: 'release background append' }],
          },
        })
        emitScenarioResult()
      }, 50)
      return
    } else if (scenario === 'eager-failure-partial-line') {
      mirror(mainPath, [marked('failure-partial-line')])
      const assistantLine = JSON.stringify({
        type: 'assistant',
        session_id: sessionId,
        message: {
          model: 'probe',
          content: [{ type: 'text', text: 'partial line survived' }],
        },
      })
      const splitIndex = assistantLine.indexOf('"content"')
      setTimeout(() => {
        process.stdout.write(assistantLine.slice(0, splitIndex))
      }, 100)
      setTimeout(() => {
        process.stdout.write(`${assistantLine.slice(splitIndex)}\n`)
        emitScenarioResult()
      }, 1500)
      return
    } else if (scenario === 'bytes') {
      const empty = [{ type: 'probe', marker: 'bytes', text: '' }]
      const padding = 1024 * 1024 - JSON.stringify(empty).length
      mirror(mainPath, [
        { type: 'probe', marker: 'bytes', text: 'x'.repeat(padding) },
      ])
      mirror(mainPath, [marked('after-bytes')])
    } else if (scenario === 'unsafe') {
      mirror(join(dirname(projectsRoot), 'outside.jsonl'), [marked('outside')])
      mirror(join(projectsRoot, projectKey, sessionId, 'shallow.jsonl'), [
        marked('shallow'),
      ])
      mirror(mainPath, [marked('valid')])
    } else if (scenario === 'retry' || scenario === 'failure') {
      mirror(mainPath, [marked('retry')])
    } else {
      process.stderr.write(`unknown probe scenario: ${scenario}\n`)
      process.exitCode = 6
      input.close()
      return
    }

    emitScenarioResult()
    return
  }

  const entry = {
    type: 'user',
    uuid: process.env.PROBE_ENTRY_UUID,
    sessionId,
    message: { role: 'user', content: 'mirrored continuation' },
  }
  appendFileSync(mainPath, `${JSON.stringify(entry)}\n`)
  emit({ type: 'transcript_mirror', filePath: mainPath, entries: [entry] })
  emit({
    type: 'result',
    subtype: 'success',
    is_error: false,
    duration_ms: 1,
    duration_api_ms: 1,
    num_turns: 1,
    total_cost_usd: 0,
    session_id: sessionId,
    result: JSON.stringify({
      sessionId,
      mirrorFlag: process.argv.includes('--session-mirror'),
      initialCount: initialEntries.length,
      subagentCount: subagentEntries.length,
      metadataToolUseId: metadata?.toolUseId ?? null,
    }),
  })
})
