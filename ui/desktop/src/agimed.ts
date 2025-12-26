import Electron from 'electron';
import fs from 'node:fs';
import { spawn, ChildProcess } from 'child_process';
import { createServer } from 'net';
import os from 'node:os';
import path from 'node:path';
import log from './utils/logger';
import { App } from 'electron';
import { Buffer } from 'node:buffer';

import { status } from './api';
import { Client } from './api/client';
import { ExternalAgimedConfig } from './utils/settings';
import { getEnvCompat } from './utils/envCompat';

export const findAvailablePort = (): Promise<number> => {
  return new Promise((resolve, _reject) => {
    const server = createServer();

    server.listen(0, '127.0.0.1', () => {
      const { port } = server.address() as { port: number };
      server.close(() => {
        log.info(`Found available port: ${port}`);
        resolve(port);
      });
    });
  });
};

// Check if agimed server is ready by polling the status endpoint
export const checkServerStatus = async (client: Client, errorLog: string[]): Promise<boolean> => {
  const interval = 100; // ms
  const maxAttempts = 100; // 10s

  const fatal = (line: string) => {
    const trimmed = line.trim().toLowerCase();
    return trimmed.startsWith("thread 'main' panicked at") || trimmed.startsWith('error:');
  };

  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    if (errorLog.some(fatal)) {
      log.error('Detected fatal error in server logs');
      return false;
    }
    try {
      await status({ client, throwOnError: true });
      return true;
    } catch {
      if (attempt === maxAttempts) {
        log.error(`Server failed to respond after ${(interval * maxAttempts) / 1000} seconds`);
      }
    }
    await new Promise((resolve) => setTimeout(resolve, interval));
  }
  return false;
};

export interface AgimedResult {
  baseUrl: string;
  workingDir: string;
  process: ChildProcess;
  errorLog: string[];
  kill: () => void;
}

const connectToExternalBackend = (workingDir: string, url: string): AgimedResult => {
  log.info(`Using external agimed backend at ${url}`);

  const mockProcess = {
    pid: undefined,
    kill: () => {
      log.info(`Not killing external process that is managed externally`);
    },
  } as ChildProcess;

  return {
    baseUrl: url,
    workingDir,
    process: mockProcess,
    errorLog: [],
    kill: () => {
      log.info(`Not killing external process that is managed externally`);
    },
  };
};

interface AgimedProcessEnv {
  [key: string]: string | undefined;

  HOME: string;
  USERPROFILE?: string;
  APPDATA?: string;
  LOCALAPPDATA?: string;
  PATH: string;
  AGIME_PORT?: string;
  AGIME_SERVER__SECRET_KEY?: string;
  // Legacy GOOSE_ keys for backward compatibility
  GOOSE_PORT?: string;
  GOOSE_SERVER__SECRET_KEY?: string;
  // UTF-8 encoding environment variables
  LANG?: string;
  LC_ALL?: string;
  PYTHONIOENCODING?: string;
  CHCP?: string;
}

export interface StartAgimedOptions {
  app: App;
  serverSecret: string;
  dir: string;
  env?: Partial<AgimedProcessEnv>;
  externalAgimed?: ExternalAgimedConfig;
}

export const startAgimed = async (options: StartAgimedOptions): Promise<AgimedResult> => {
  const { app, serverSecret, dir: inputDir, env = {}, externalAgimed } = options;
  const isWindows = process.platform === 'win32';
  const homeDir = os.homedir();
  const dir = path.resolve(path.normalize(inputDir));

  if (externalAgimed?.enabled && externalAgimed.url) {
    return connectToExternalBackend(dir, externalAgimed.url);
  }

  if (getEnvCompat('EXTERNAL_BACKEND')) {
    return connectToExternalBackend(dir, 'http://127.0.0.1:3000');
  }

  let agimedPath = getAgimedBinaryPath(app);

  const resolvedAgimedPath = path.resolve(agimedPath);

  const port = await findAvailablePort();
  const stderrLines: string[] = [];

  log.info(`Starting agimed from: ${resolvedAgimedPath} on port ${port} in dir ${dir}`);

  // Base environment variables for all platforms
  const additionalEnv: AgimedProcessEnv = {
    HOME: homeDir,
    PATH: `${path.dirname(resolvedAgimedPath)}${path.delimiter}${process.env.PATH || ''}`,
    // New AGIME_ prefixed environment variables (preferred)
    AGIME_PORT: String(port),
    AGIME_SERVER__SECRET_KEY: serverSecret,
    // Legacy GOOSE_ prefixed environment variables (for backward compatibility)
    GOOSE_PORT: String(port),
    GOOSE_SERVER__SECRET_KEY: serverSecret,
    // Ensure UTF-8 encoding for all output
    LANG: 'en_US.UTF-8',
    LC_ALL: 'en_US.UTF-8',
    PYTHONIOENCODING: 'utf-8',
    ...env,
  };

  // Add Windows-specific environment variables
  if (isWindows) {
    additionalEnv.USERPROFILE = homeDir;
    additionalEnv.APPDATA = process.env.APPDATA || path.join(homeDir, 'AppData', 'Roaming');
    additionalEnv.LOCALAPPDATA =
      process.env.LOCALAPPDATA || path.join(homeDir, 'AppData', 'Local');
    // Windows-specific: Set console code page to UTF-8
    additionalEnv.CHCP = '65001';
  }

  const processEnv: AgimedProcessEnv = { ...process.env, ...additionalEnv } as AgimedProcessEnv;

  if (isWindows && !resolvedAgimedPath.toLowerCase().endsWith('.exe')) {
    agimedPath = resolvedAgimedPath + '.exe';
  } else {
    agimedPath = resolvedAgimedPath;
  }
  log.info(`Binary path resolved to: ${agimedPath}`);

  const spawnOptions = {
    cwd: dir,
    env: processEnv,
    stdio: ['ignore', 'pipe', 'pipe'] as ['ignore', 'pipe', 'pipe'],
    windowsHide: true,
    detached: isWindows,
    shell: false,
  };

  const safeSpawnOptions = {
    ...spawnOptions,
    env: Object.keys(spawnOptions.env || {}).reduce(
      (acc, key) => {
        if (key.includes('SECRET') || key.includes('PASSWORD') || key.includes('TOKEN')) {
          acc[key] = '[REDACTED]';
        } else {
          acc[key] = spawnOptions.env![key] || '';
        }
        return acc;
      },
      {} as Record<string, string>
    ),
  };
  log.info('Spawn options:', JSON.stringify(safeSpawnOptions, null, 2));

  const safeArgs = ['agent'];

  const agimedProcess: ChildProcess = spawn(agimedPath, safeArgs, spawnOptions);

  if (isWindows && agimedProcess.unref) {
    agimedProcess.unref();
  }

  agimedProcess.stdout?.on('data', (data: Buffer) => {
    log.info(`agimed stdout for port ${port} and dir ${dir}: ${data.toString()}`);
  });

  agimedProcess.stderr?.on('data', (data: Buffer) => {
    const lines = data
      .toString()
      .split('\n')
      .filter((l) => l.trim());
    lines.forEach((line) => {
      log.error(`agimed stderr for port ${port} and dir ${dir}: ${line}`);
      stderrLines.push(line);
    });
  });

  agimedProcess.on('close', (code: number | null) => {
    log.info(`agimed process exited with code ${code} for port ${port} and dir ${dir}`);
  });

  agimedProcess.on('error', (err: Error) => {
    log.error(`Failed to start agimed on port ${port} and dir ${dir}`, err);
    throw err;
  });

  // Track if process has been killed to prevent duplicate termination
  let killed = false;

  const try_kill_agime = () => {
    // Prevent duplicate termination attempts
    if (killed) {
      log.info('Process already terminated, skipping kill');
      return;
    }

    // Check PID validity
    if (!agimedProcess.pid) {
      log.warn('No valid PID available, cannot kill process');
      return;
    }

    killed = true;

    try {
      if (isWindows) {
        const pid = agimedProcess.pid.toString();
        log.info(`Terminating agimed process with PID ${pid} using taskkill`);
        spawn('taskkill', ['/pid', pid, '/T', '/F'], { shell: false, windowsHide: true });
      } else {
        agimedProcess.kill?.();
      }
    } catch (error) {
      log.error('Error while terminating agimed process:', error);
    }
  };

  // Use once to prevent duplicate listeners when multiple windows are created
  app.once('will-quit', () => {
    log.info('App quitting, terminating agimed server');
    try_kill_agime();
  });

  log.info(`Agimed server successfully started on port ${port}`);
  return {
    baseUrl: `http://127.0.0.1:${port}`,
    workingDir: dir,
    process: agimedProcess,
    errorLog: stderrLines,
    kill: try_kill_agime,
  };
};

const getAgimedBinaryPath = (app: Electron.App): string => {
  let executableName = process.platform === 'win32' ? 'agimed.exe' : 'agimed';

  let possiblePaths: string[];
  if (!app.isPackaged) {
    possiblePaths = [
      path.join(process.cwd(), 'src', 'bin', executableName),
      path.join(process.cwd(), 'bin', executableName),
      path.join(process.cwd(), '..', '..', 'target', 'debug', executableName),
      path.join(process.cwd(), '..', '..', 'target', 'release', executableName),
    ];
  } else {
    possiblePaths = [path.join(process.resourcesPath, 'bin', executableName)];
  }

  for (const binPath of possiblePaths) {
    try {
      const resolvedPath = path.resolve(binPath);

      if (fs.existsSync(resolvedPath)) {
        const stats = fs.statSync(resolvedPath);
        if (stats.isFile()) {
          return resolvedPath;
        } else {
          log.error(`Path exists but is not a regular file: ${resolvedPath}`);
        }
      }
    } catch (error) {
      log.error(`Error checking path ${binPath}:`, error);
    }
  }

  throw new Error(
    `Could not find ${executableName} binary in any of the expected locations: ${possiblePaths.join(
      ', '
    )}`
  );
};
