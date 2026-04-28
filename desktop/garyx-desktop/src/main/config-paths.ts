import { constants } from 'node:fs';
import { access, copyFile, mkdir, readFile, rename, unlink } from 'node:fs/promises';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';

export const DEFAULT_GARY_HOME_DIR = join(homedir(), '.garyx');
export const DEFAULT_CONFIG_PATH = join(DEFAULT_GARY_HOME_DIR, 'garyx.json');
const LEGACY_GARY_CONFIG_PATH = join(homedir(), '.gary', 'gary.json');
const LEGACY_CONFIG_PATH = join(homedir(), 'gary', 'gary.json');

async function fileExists(path: string): Promise<boolean> {
  try {
    await access(path, constants.R_OK);
    return true;
  } catch {
    return false;
  }
}

export async function desktopConfigFileExists(): Promise<boolean> {
  const configPath = await resolveDesktopConfigPath();
  return fileExists(configPath);
}

async function moveFile(sourcePath: string, targetPath: string): Promise<void> {
  try {
    await rename(sourcePath, targetPath);
  } catch (error) {
    const code = (error as NodeJS.ErrnoException | undefined)?.code;
    if (code !== 'EXDEV') {
      throw error;
    }
    await copyFile(sourcePath, targetPath);
    await unlink(sourcePath);
  }
}

export async function resolveDesktopConfigPath(): Promise<string> {
  if (await fileExists(DEFAULT_CONFIG_PATH)) {
    return DEFAULT_CONFIG_PATH;
  }

  // Try legacy paths in order: ~/.gary/gary.json, ~/gary/gary.json
  const legacyPaths = [LEGACY_GARY_CONFIG_PATH, LEGACY_CONFIG_PATH];
  for (const legacyPath of legacyPaths) {
    if (await fileExists(legacyPath)) {
      try {
        await mkdir(dirname(DEFAULT_CONFIG_PATH), { recursive: true });
        await moveFile(legacyPath, DEFAULT_CONFIG_PATH);
        return DEFAULT_CONFIG_PATH;
      } catch {
        return legacyPath;
      }
    }
  }

  return DEFAULT_CONFIG_PATH;
}

export async function readResolvedDesktopConfig(): Promise<string> {
  const configPath = await resolveDesktopConfigPath();
  return readFile(configPath, 'utf8');
}
