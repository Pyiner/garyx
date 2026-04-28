export function buildDesktopElectronLaunchEnv(overrides = {}) {
  return {
    ...process.env,
    // Playwright's Electron launcher allocates its own DevTools port. Disabling
    // the desktop app's fixed CDP listener avoids collisions with a separately
    // running Garyx instance during smoke runs.
    GARYX_DESKTOP_DISABLE_FIXED_CDP: '1',
    ...overrides,
  };
}
