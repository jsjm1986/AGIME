import { spawn, ChildProcess, execSync } from 'child_process';
import path from 'node:path';
import fs from 'node:fs';
import os from 'node:os';
import https from 'node:https';
import zlib from 'node:zlib';
import { pipeline } from 'node:stream/promises';
import log from './logger';

// Cloudflared download URLs for different platforms
const CLOUDFLARED_URLS: Record<string, string> = {
  'win32-x64': 'https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-windows-amd64.exe',
  'win32-arm64': 'https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-windows-arm64.exe',
  'darwin-x64': 'https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-darwin-amd64.tgz',
  'darwin-arm64': 'https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-darwin-arm64.tgz',
  'linux-x64': 'https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64',
  'linux-arm64': 'https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-arm64',
};

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
  const arch = process.arch;
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

// Download cloudflared binary
export async function downloadCloudflared(
  onProgress?: (percent: number) => void
): Promise<string> {
  const platform = process.platform;
  const arch = process.arch;
  const key = `${platform}-${arch}`;

  const downloadUrl = CLOUDFLARED_URLS[key];
  if (!downloadUrl) {
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
  const isTgz = downloadUrl.endsWith('.tgz');
  const downloadPath = isTgz ? path.join(binDir, 'cloudflared.tgz') : binPath;

  log.info(`Downloading cloudflared from ${downloadUrl} to ${downloadPath}`);

  return new Promise((resolve, reject) => {
    const followRedirect = (url: string, redirectCount = 0) => {
      // Prevent infinite redirects
      if (redirectCount > 10) {
        reject(new Error('Too many redirects'));
        return;
      }

      https.get(url, (response) => {
        // Handle redirects (301, 302, 303, 307, 308)
        if (response.statusCode && response.statusCode >= 300 && response.statusCode < 400) {
          const redirectUrl = response.headers.location;
          if (redirectUrl) {
            log.info(`Redirect ${response.statusCode} -> ${redirectUrl}`);
            followRedirect(redirectUrl, redirectCount + 1);
            return;
          }
        }

        if (response.statusCode !== 200) {
          reject(new Error(`Failed to download: HTTP ${response.statusCode}`));
          return;
        }

        const totalSize = parseInt(response.headers['content-length'] || '0', 10);
        let downloadedSize = 0;

        const fileStream = fs.createWriteStream(downloadPath);

        response.on('data', (chunk) => {
          downloadedSize += chunk.length;
          if (totalSize > 0 && onProgress) {
            onProgress(Math.round((downloadedSize / totalSize) * 100));
          }
        });

        response.pipe(fileStream);

        fileStream.on('finish', async () => {
          fileStream.close();

          try {
            // Handle .tgz extraction for macOS
            if (isTgz) {
              log.info(`Extracting ${downloadPath} to ${binDir}`);
              try {
                // Use tar command to extract (available on macOS)
                execSync(`tar -xzf "${downloadPath}" -C "${binDir}"`, { stdio: 'ignore' });
                // Remove the .tgz file after extraction
                fs.unlinkSync(downloadPath);
                log.info(`Extracted and removed ${downloadPath}`);
              } catch (extractError) {
                log.error('Failed to extract .tgz file:', extractError);
                // Clean up the downloaded file
                try {
                  fs.unlinkSync(downloadPath);
                } catch {}
                reject(new Error('Failed to extract cloudflared archive'));
                return;
              }
            }

            // Make executable on Unix systems
            if (platform !== 'win32') {
              fs.chmodSync(binPath, 0o755);
            }

            // Verify the binary exists
            if (!fs.existsSync(binPath)) {
              reject(new Error(`Binary not found at ${binPath} after download`));
              return;
            }

            log.info(`Cloudflared downloaded successfully to ${binPath}`);
            resolve(binPath);
          } catch (postProcessError) {
            log.error('Post-download processing failed:', postProcessError);
            reject(postProcessError);
          }
        });

        fileStream.on('error', (err) => {
          fs.unlink(downloadPath, () => {}); // Delete partial file
          reject(err);
        });
      }).on('error', reject);
    };

    followRedirect(downloadUrl);
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
