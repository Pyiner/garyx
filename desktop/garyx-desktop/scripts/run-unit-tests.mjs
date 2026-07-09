#!/usr/bin/env node
import { readdirSync, statSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectDir = path.resolve(scriptDir, '..');
const sourceDir = path.join(projectDir, 'src');
const testFilePattern = /\.(test|spec)\.mjs$/;

function usage() {
  console.log(`Usage:
  npm run test:unit
  npm run test:unit -- --list
  npm run test:unit -- src/renderer/src/render-view-model.test.mjs
  npm run test:unit -- --test-name-pattern "renders tool rows"

When no files are passed, all src/**/*.test.mjs and src/**/*.spec.mjs files run.`);
}

function walk(directory, files) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      walk(entryPath, files);
      continue;
    }

    if (entry.isFile() && testFilePattern.test(entry.name)) {
      files.push(entryPath);
    }
  }
}

function discoverTestFiles() {
  const files = [];
  walk(sourceDir, files);
  return files.sort((a, b) => a.localeCompare(b));
}

function maybeResolveExistingFile(arg) {
  if (arg.startsWith('-')) {
    return null;
  }

  const absolutePath = path.isAbsolute(arg)
    ? arg
    : path.resolve(projectDir, arg);
  if (!testFilePattern.test(absolutePath)) {
    return null;
  }
  if (!statSync(absolutePath, { throwIfNoEntry: false })?.isFile()) {
    return null;
  }
  return absolutePath;
}

const args = process.argv.slice(2);
if (args.includes('--help') || args.includes('-h')) {
  usage();
  process.exit(0);
}

const discoveredFiles = discoverTestFiles();
if (args.includes('--list')) {
  for (const file of discoveredFiles) {
    console.log(path.relative(projectDir, file));
  }
  process.exit(0);
}

const selectedFiles = [];
const testRunnerArgs = [];
for (const arg of args) {
  const file = maybeResolveExistingFile(arg);
  if (file) {
    selectedFiles.push(file);
  } else {
    testRunnerArgs.push(arg);
  }
}

const testFiles = selectedFiles.length > 0 ? selectedFiles : discoveredFiles;
if (testFiles.length === 0) {
  console.error('No unit test files found under src.');
  process.exit(1);
}

const child = spawnSync(
  process.execPath,
  ['--experimental-strip-types', '--test', ...testRunnerArgs, ...testFiles],
  { cwd: projectDir, stdio: 'inherit' },
);

if (child.error) {
  console.error(child.error.message);
  process.exit(1);
}

process.exit(child.status ?? 1);
