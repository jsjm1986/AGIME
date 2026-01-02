import { spawn, ChildProcess, execSync } from 'child_process';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';
import https from 'node:https';
import http from 'node:http';
import log from './logger';

// Official Cloudflare GitHub releases (primary source)
const CLOUDFLARED_URLS: Record<string, string> = {
  'win32-x64': 'https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-windows-amd64.exe',
  'win32-arm64': 'https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-windows-arm64.exe',
  'darwin-x64': 'https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-darwin-amd64.tgz',
  'darwin-arm64': 'https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-darwin-arm64.tgz',
  'linux-x64': 'https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64',
  'linux-arm64': 'https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-arm64',
};

// Mirror/proxy URLs for faster download in China (backup sources)
const MIRROR_PREFIXES = [
  '', // No prefix = direct GitHub (try first)
  'https://mirror.ghproxy.com/', // ghproxy mirror
  'https://ghproxy.net/', // ghproxy.net mirror
];

export interface CloudflaredTunnelInfo {
  state: 'idle' | 'starting' | 'running' | 'error' | 'disabled';
  url: string;
  hostname: string;
  secret: string;
  error?: string;
}

let cloudflaredProcess: ChildProcess | null = null;
let currentTunnelInfo: CloudflaredTunnelInfo = {
  state: 'idle',
  url: '',
  hostname: '',
  secret: '',
};

// Get the cloudflared binary path
export function getCloudflaredPath(): string {
  const platform = process.platform;
  const binName = platform === 'win32' ? 'cloudflared.exe' : 'cloudflared';

  // Check in app data directory first
  const appDataPath = path.join(
    platform === 'win32'
      ? process.env.LOCALAPPDATA || path.join(os.homedir(), 'AppData', 'Local')
      : platform === 'darwin'
        ? path.join(os.homedir(), 'Library', 'Application Support')
        : path.join(os.homedir(), '.local', 'share'),
    'AGIME',
    'bin',
    binName
  );

  if (fs.existsSync(appDataPath)) {
    return appDataPath;
  }

  // Check in system PATH
  const pathDirs = (process.env.PATH || '').split(path.delimiter);
  for (const dir of pathDirs) {
    const fullPath = path.join(dir, binName);
    if (fs.existsSync(fullPath)) {
      return fullPath;
    }
  }

  return appDataPath; // Return expected path even if not exists
}

// Check if cloudflared is installed
export function isCloudflaredInstalled(): boolean {
  const cloudflaredPath = getCloudflaredPath();
  return fs.existsSync(cloudflaredPath);
}

// Minimum expected file size for cloudflared binary (in bytes)
// Windows exe is typically ~65MB, macOS/Linux is ~40MB
const MIN_CLOUDFLARED_SIZE = 10 * 1024 * 1024; // 10MB minimum

// Validate that the cloudflared binary is a valid executable
export function validateCloudflaredBinary(): { valid: boolean; error?: string } {
  const cloudflaredPath = getCloudflaredPath();

  if (!fs.existsSync(cloudflaredPath)) {
    return { valid: false, error: 'File not found' };
  }

  try {
    const stats = fs.statSync(cloudflaredPath);

    // Check file size - if it's too small, it's likely corrupted or an error page
    if (stats.size < MIN_CLOUDFLARED_SIZE) {
      log.warn(`Cloudflared binary too small: ${stats.size} bytes (expected > ${MIN_CLOUDFLARED_SIZE})`);
      return { valid: false, error: `File too small (${Math.round(stats.size / 1024)}KB), likely corrupted` };
    }

    // Read magic bytes to validate executable format
    const fd = fs.openSync(cloudflaredPath, 'r');
    const buffer = Buffer.alloc(4);
    fs.readSync(fd, buffer, 0, 4, 0);
    fs.closeSync(fd);

    const platform = process.platform;

    if (platform === 'win32') {
      // Windows PE executable: starts with "MZ"
      if (buffer.slice(0, 2).toString('ascii') !== 'MZ') {
        log.warn('Cloudflared binary is not a valid Windows executable (missing MZ header)');
        return { valid: false, error: 'Not a valid Windows executable' };
      }
    } else if (platform === 'darwin') {
      // macOS Mach-O executable: starts with 0xFEEDFACE (32-bit), 0xFEEDFACF (64-bit),
      // or 0xCAFEBABE (universal binary)
      const magic = buffer.readUInt32BE(0);
      const magicLE = buffer.readUInt32LE(0);
      const validMachO = [
        0xFEEDFACE, 0xFEEDFACF, // Mach-O 32/64 big endian
        0xCEFAEDFE, 0xCFFAEDFE, // Mach-O 32/64 little endian
        0xCAFEBABE, 0xBEBAFECA, // Universal binary
      ];
      if (!validMachO.includes(magic) && !validMachO.includes(magicLE)) {
        log.warn(`Cloudflared binary is not a valid macOS executable (magic: 0x${buffer.toString('hex')})`);
        return { valid: false, error: 'Not a valid macOS executable' };
      }
    } else {
      // Linux ELF executable: starts with 0x7F 'E' 'L' 'F'
      if (buffer[0] !== 0x7F || buffer.slice(1, 4).toString('ascii') !== 'ELF') {
        log.warn(`Cloudflared binary is not a valid Linux executable (magic: 0x${buffer.toString('hex')})`);
        return { valid: false, error: 'Not a valid Linux executable' };
      }
    }

    log.info(`Cloudflared binary validated successfully (${Math.round(stats.size / 1024 / 1024)}MB)`);
    return { valid: true };
  } catch (err) {
    log.error('Failed to validate cloudflared binary:', err);
    return { valid: false, error: err instanceof Error ? err.message : 'Validation failed' };
  }
}

// Remove corrupted cloudflared binary
export function removeCloudflaredBinary(): boolean {
  const cloudflaredPath = getCloudflaredPath();
  try {
    if (fs.existsSync(cloudflaredPath)) {
      fs.unlinkSync(cloudflaredPath);
      log.info(`Removed cloudflared binary at ${cloudflaredPath}`);
      return true;
    }
    return false;
  } catch (err) {
    log.error('Failed to remove cloudflared binary:', err);
    return false;
  }
}

// Download cloudflared binary with mirror fallback support
export async function downloadCloudflared(
  onProgress?: (percent: number) => void
): Promise<string> {
  const platform = process.platform;
  const arch = process.arch;
  const key = `${platform}-${arch}`;

  const baseUrl = CLOUDFLARED_URLS[key];
  if (!baseUrl) {
    throw new Error(`Unsupported platform: ${platform}-${arch}`);
  }

  const binName = platform === 'win32' ? 'cloudflared.exe' : 'cloudflared';
  const binDir = path.join(
    platform === 'win32'
      ? process.env.LOCALAPPDATA || path.join(os.homedir(), 'AppData', 'Local')
      : platform === 'darwin'
        ? path.join(os.homedir(), 'Library', 'Application Support')
        : path.join(os.homedir(), '.local', 'share'),
    'AGIME',
    'bin'
  );

  // Ensure directory exists
  fs.mkdirSync(binDir, { recursive: true });

  const binPath = path.join(binDir, binName);
  const isTgz = baseUrl.endsWith('.tgz');
  const downloadPath = isTgz ? path.join(binDir, 'cloudflared.tgz') : binPath;

  // Try each mirror source until one succeeds
  const errors: string[] = [];

  for (const mirrorPrefix of MIRROR_PREFIXES) {
    const downloadUrl = mirrorPrefix + baseUrl;
    log.info(`Trying to download cloudflared from: ${downloadUrl}`);

    try {
      await downloadFromUrl(downloadUrl, downloadPath, onProgress);

      // Post-download processing
      if (isTgz) {
        log.info(`Extracting ${downloadPath} to ${binDir}`);
        execSync(`tar -xzf "${downloadPath}" -C "${binDir}"`, { stdio: 'ignore' });
        fs.unlinkSync(downloadPath);
        log.info(`Extracted and removed ${downloadPath}`);
      }

      // Make executable on Unix systems
      if (platform !== 'win32') {
        try {
          fs.chmodSync(binPath, 0o755);
        } catch (chmodErr) {
          log.warn('Failed to set executable permission:', chmodErr);
        }
      }

      // Verify the binary exists
      if (!fs.existsSync(binPath)) {
        throw new Error(`Binary not found at ${binPath} after download`);
      }

      // Validate the downloaded binary
      const validation = validateCloudflaredBinary();
      if (!validation.valid) {
        try { fs.unlinkSync(binPath); } catch {}
        throw new Error(`Downloaded file is invalid: ${validation.error}`);
      }

      log.info(`Cloudflared downloaded and validated successfully from ${downloadUrl}`);
      return binPath;

    } catch (err) {
      const errMsg = err instanceof Error ? err.message : String(err);
      errors.push(`${mirrorPrefix || 'GitHub'}: ${errMsg}`);
      log.warn(`Download failed from ${downloadUrl}: ${errMsg}`);

      // Clean up partial downloads
      try { fs.unlinkSync(downloadPath); } catch {}
      try { if (fs.existsSync(binPath)) fs.unlinkSync(binPath); } catch {}

      // Continue to next mirror
    }
  }

  // All mirrors failed
  throw new Error(`Failed to download cloudflared from all sources:\n${errors.join('\n')}`);
}

// Helper function to download from a single URL with timeout
function downloadFromUrl(
  url: string,
  destPath: string,
  onProgress?: (percent: number) => void,
  timeoutMs: number = 60000
): Promise<void> {
  return new Promise((resolve, reject) => {
    let timeoutId: NodeJS.Timeout | null = null;
    let req: http.ClientRequest | null = null;

    const cleanup = () => {
      if (timeoutId) clearTimeout(timeoutId);
      if (req) req.destroy();
    };

    const followRedirect = (currentUrl: string, redirectCount = 0) => {
      if (redirectCount > 10) {
        cleanup();
        reject(new Error('Too many redirects'));
        return;
      }

      const currentIsHttps = currentUrl.startsWith('https://');
      const currentHttpModule = currentIsHttps ? https : http;

      // Set connection timeout
      timeoutId = setTimeout(() => {
        cleanup();
        reject(new Error('Connection timeout'));
      }, timeoutMs);

      req = currentHttpModule.get(currentUrl, (response) => {
        if (timeoutId) clearTimeout(timeoutId);

        // Handle redirects
        if (response.statusCode && response.statusCode >= 300 && response.statusCode < 400) {
          const redirectUrl = response.headers.location;
          if (redirectUrl) {
            log.info(`Redirect ${response.statusCode} -> ${redirectUrl}`);
            followRedirect(redirectUrl, redirectCount + 1);
            return;
          }
        }

        if (response.statusCode !== 200) {
          cleanup();
          reject(new Error(`HTTP ${response.statusCode}`));
          return;
        }

        const totalSize = parseInt(response.headers['content-length'] || '0', 10);
        let downloadedSize = 0;

        const fileStream = fs.createWriteStream(destPath);

        response.on('data', (chunk) => {
          downloadedSize += chunk.length;
          if (totalSize > 0 && onProgress) {
            onProgress(Math.round((downloadedSize / totalSize) * 100));
          }
        });

        response.pipe(fileStream);

        fileStream.on('finish', () => {
          fileStream.close();
          resolve();
        });

        fileStream.on('error', (err) => {
          cleanup();
          fs.unlink(destPath, () => {});
          reject(err);
        });
      });

      req.on('error', (err) => {
        cleanup();
        reject(err);
      });
    };

    followRedirect(url);
  });
}

// Start cloudflared tunnel
export async function startCloudflaredTunnel(
  localPort: number,
  serverSecret: string
): Promise<CloudflaredTunnelInfo> {
  if (cloudflaredProcess) {
    log.warn('Cloudflared tunnel is already running');
    return currentTunnelInfo;
  }

  const cloudflaredPath = getCloudflaredPath();

  if (!fs.existsSync(cloudflaredPath)) {
    throw new Error('Cloudflared not installed. Please download it first.');
  }

  // Validate the binary before trying to spawn
  const validation = validateCloudflaredBinary();
  if (!validation.valid) {
    log.error(`Cloudflared binary validation failed: ${validation.error}`);
    // Remove the corrupted binary so user can re-download
    removeCloudflaredBinary();
    throw new Error(`Cloudflared binary is invalid: ${validation.error}. Please re-download.`);
  }

  currentTunnelInfo = {
    state: 'starting',
    url: '',
    hostname: '',
    secret: serverSecret,
  };

  return new Promise((resolve, reject) => {
    const args = [
      'tunnel',
      '--url', `http://127.0.0.1:${localPort}`,
      '--no-autoupdate',
    ];

    log.info(`Starting cloudflared: ${cloudflaredPath} ${args.join(' ')}`);

    cloudflaredProcess = spawn(cloudflaredPath, args, {
      windowsHide: true,
      stdio: ['ignore', 'pipe', 'pipe'],
    });

    let urlFound = false;
    const timeoutId = setTimeout(() => {
      if (!urlFound) {
        stopCloudflaredTunnel();
        reject(new Error('Timeout waiting for tunnel URL'));
      }
    }, 60000); // 60 second timeout

    const handleOutput = (data: Buffer) => {
      const output = data.toString();
      log.info(`cloudflared: ${output}`);

      // Look for the tunnel URL in the output
      // Format: "https://xxxxx.trycloudflare.com"
      const urlMatch = output.match(/https:\/\/[a-zA-Z0-9-]+\.trycloudflare\.com/);
      if (urlMatch && !urlFound) {
        urlFound = true;
        clearTimeout(timeoutId);

        const tunnelUrl = urlMatch[0];
        const hostname = new URL(tunnelUrl).hostname;

        currentTunnelInfo = {
          state: 'running',
          url: tunnelUrl,
          hostname: hostname,
          secret: serverSecret,
        };

        log.info(`Cloudflared tunnel started: ${tunnelUrl}`);
        resolve(currentTunnelInfo);
      }
    };

    cloudflaredProcess.stdout?.on('data', handleOutput);
    cloudflaredProcess.stderr?.on('data', handleOutput);

    cloudflaredProcess.on('error', (err) => {
      log.error('Cloudflared process error:', err);
      clearTimeout(timeoutId);
      currentTunnelInfo = {
        state: 'error',
        url: '',
        hostname: '',
        secret: '',
        error: err.message,
      };
      cloudflaredProcess = null;
      if (!urlFound) {
        reject(err);
      }
    });

    cloudflaredProcess.on('close', (code) => {
      log.info(`Cloudflared process exited with code ${code}`);
      clearTimeout(timeoutId);
      if (currentTunnelInfo.state === 'running') {
        currentTunnelInfo = {
          state: 'idle',
          url: '',
          hostname: '',
          secret: '',
        };
      }
      cloudflaredProcess = null;
    });
  });
}

// Stop cloudflared tunnel
export function stopCloudflaredTunnel(): void {
  if (cloudflaredProcess) {
    log.info('Stopping cloudflared tunnel');

    if (process.platform === 'win32') {
      // On Windows, use taskkill to ensure the process is terminated
      const pid = cloudflaredProcess.pid;
      if (pid) {
        spawn('taskkill', ['/pid', pid.toString(), '/T', '/F'], {
          windowsHide: true,
        });
      }
    } else {
      cloudflaredProcess.kill('SIGTERM');
    }

    cloudflaredProcess = null;
  }

  currentTunnelInfo = {
    state: 'idle',
    url: '',
    hostname: '',
    secret: '',
  };
}

// Get current tunnel status
export function getCloudflaredTunnelStatus(): CloudflaredTunnelInfo {
  return { ...currentTunnelInfo };
}

// Check if tunnel is running
export function isCloudflaredTunnelRunning(): boolean {
  return cloudflaredProcess !== null && currentTunnelInfo.state === 'running';
}
