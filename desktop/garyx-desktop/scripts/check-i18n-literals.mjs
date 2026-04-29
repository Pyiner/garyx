#!/usr/bin/env node
import { readdirSync, readFileSync, statSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const projectDir = path.resolve(scriptDir, '..');
const ignoredDirectories = new Set([
  'node_modules',
  'out',
  'dist',
  'dist-release',
  'test-artifacts',
]);
const checkedExtensions = new Set([
  '.cjs',
  '.css',
  '.html',
  '.js',
  '.json',
  '.jsx',
  '.mjs',
  '.ts',
  '.tsx',
]);
const allowedPrefix = path.join(projectDir, 'src/renderer/src/i18n') + path.sep;
const hanPattern = /\p{Script=Han}/u;

function isAllowed(filePath) {
  return filePath.startsWith(allowedPrefix);
}

function findHanLines(filePath) {
  const text = readFileSync(filePath, 'utf8');
  return text
    .split(/\r?\n/)
    .map((line, index) => ({ line, lineNumber: index + 1 }))
    .filter(({ line }) => hanPattern.test(line));
}

function walk(directory, violations) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.isDirectory()) {
      if (!ignoredDirectories.has(entry.name)) {
        walk(path.join(directory, entry.name), violations);
      }
      continue;
    }

    const filePath = path.join(directory, entry.name);
    if (isAllowed(filePath) || !checkedExtensions.has(path.extname(entry.name))) {
      continue;
    }
    if (!statSync(filePath).isFile()) {
      continue;
    }

    for (const { line, lineNumber } of findHanLines(filePath)) {
      violations.push({
        filePath: path.relative(projectDir, filePath),
        lineNumber,
        line: line.trim(),
      });
    }
  }
}

const violations = [];
walk(projectDir, violations);

if (violations.length > 0) {
  console.error('Non-i18n Han literals found:');
  for (const violation of violations) {
    console.error(`${violation.filePath}:${violation.lineNumber}: ${violation.line}`);
  }
  process.exit(1);
}

console.log('No Han literals found outside src/renderer/src/i18n.');
