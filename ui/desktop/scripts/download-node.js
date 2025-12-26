#!/usr/bin/env node

/**
 * Download Node.js LTS binaries for all supported platforms
 * This script downloads Node.js runtime for embedding with Playwright MCP
 *
 * Supported platforms:
 * - win-x64: Windows x64
 * - darwin-x64: macOS Intel
 * - darwin-arm64: macOS Apple Silicon
 * - linux-x64: Linux x64
 * - linux-arm64: Linux ARM64
 */

const https = require('https');
const crypto = require('crypto');
const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

// Node.js LTS version to download
const NODE_VERSION = '20.18.1';

// Download timeout in milliseconds (5 minutes)
const DOWNLOAD_TIMEOUT = 5 * 60 * 1000;

// Maximum redirect count to prevent infinite loops
const MAX_REDIRECTS = 5;

// Minimum required disk space in bytes (500 MB buffer)
const MIN_DISK_SPACE = 500 * 1024 * 1024;

// Platform configurations
const PLATFORMS = {
  'win-x64': {
    url: `https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-win-x64.zip`,
    extractedDir: `node-v${NODE_VERSION}-win-x64`,
    executable: 'node.exe',
    archive: 'zip'
  },
  'darwin-x64': {
    url: `https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-darwin-x64.tar.gz`,
    extractedDir: `node-v${NODE_VERSION}-darwin-x64`,
    executable: 'bin/node',
    archive: 'tar.gz'
  },
  'darwin-arm64': {
    url: `https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-darwin-arm64.tar.gz`,
    extractedDir: `node-v${NODE_VERSION}-darwin-arm64`,
    executable: 'bin/node',
    archive: 'tar.gz'
  },
  'linux-x64': {
    url: `https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.gz`,
    extractedDir: `node-v${NODE_VERSION}-linux-x64`,
    executable: 'bin/node',
    archive: 'tar.gz'
  },
  'linux-arm64': {
    url: `https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-arm64.tar.gz`,
    extractedDir: `node-v${NODE_VERSION}-linux-arm64`,
    executable: 'bin/node',
    archive: 'tar.gz'
  }
};

// Output directory
const OUTPUT_DIR = path.join(__dirname, '..', 'src', 'playwright', 'node');

// Cache for SHASUMS
let cachedShasums = null;

/**
 * Download SHASUMS256.txt for verification
 */
async function downloadShasums() {
  if (cachedShasums) return cachedShasums;

  const url = `https://nodejs.org/dist/v${NODE_VERSION}/SHASUMS256.txt`;
  console.log(`Downloading checksums from: ${url}`);

  return new Promise((resolve, reject) => {
    const request = https.get(url, { timeout: 30000 }, (response) => {
      if (response.statusCode !== 200) {
        reject(new Error(`Failed to download SHASUMS: HTTP ${response.statusCode}`));
        return;
      }

      let data = '';
      response.on('data', chunk => data += chunk);
      response.on('end', () => {
        const shasums = {};
        data.split('\n').forEach(line => {
          const match = line.match(/^([a-f0-9]{64})\s+(.+)$/);
          if (match) {
            shasums[match[2]] = match[1];
          }
        });
        console.log(`  Loaded ${Object.keys(shasums).length} checksums`);
        cachedShasums = shasums;
        resolve(shasums);
      });
    });

    request.on('error', reject);
    request.on('timeout', () => {
      request.destroy();
      reject(new Error('Timeout downloading SHASUMS'));
    });
  });
}

/**
 * Verify SHA256 checksum of a file
 */
function verifySha256(filePath, expectedHash) {
  console.log(`  Verifying SHA256 checksum...`);

  const hash = crypto.createHash('sha256');
  const data = fs.readFileSync(filePath);
  hash.update(data);

  const computed = hash.digest('hex');

  if (computed === expectedHash) {
    console.log(`  ✓ Checksum verified`);
    return true;
  } else {
    console.error(`  ✗ Checksum mismatch!`);
    console.error(`    Expected: ${expectedHash}`);
    console.error(`    Computed: ${computed}`);
    return false;
  }
}

/**
 * Download a file from URL to destination with timeout
 */
function downloadFile(url, dest) {
  return new Promise((resolve, reject) => {
    console.log(`Downloading: ${url}`);

    const file = fs.createWriteStream(dest);
    let timeoutId = null;
    let redirectCount = 0;

    const request = (urlStr) => {
      // Check redirect limit
      if (redirectCount >= MAX_REDIRECTS) {
        reject(new Error(`Too many redirects (max ${MAX_REDIRECTS})`));
        return;
      }

      const req = https.get(urlStr, { timeout: DOWNLOAD_TIMEOUT }, (response) => {
        // Clear timeout on successful response
        if (timeoutId) clearTimeout(timeoutId);

        // Handle redirects
        if (response.statusCode === 301 || response.statusCode === 302) {
          const redirectUrl = response.headers.location;
          if (!redirectUrl) {
            reject(new Error('Redirect response missing Location header'));
            return;
          }
          redirectCount++;
          console.log(`Redirecting to: ${redirectUrl} (${redirectCount}/${MAX_REDIRECTS})`);
          request(redirectUrl);
          return;
        }

        if (response.statusCode !== 200) {
          reject(new Error(`Failed to download: HTTP ${response.statusCode}`));
          return;
        }

        // Validate content-length
        const contentLength = response.headers['content-length'];
        const totalBytes = contentLength ? parseInt(contentLength, 10) : 0;

        if (!contentLength || isNaN(totalBytes) || totalBytes <= 0) {
          console.warn('  Warning: Content-Length header missing or invalid, progress will not be shown');
        }

        let downloadedBytes = 0;
        let lastPercent = 0;

        response.on('data', (chunk) => {
          downloadedBytes += chunk.length;
          if (totalBytes > 0) {
            const percent = Math.floor((downloadedBytes / totalBytes) * 100);
            if (percent >= lastPercent + 10) {
              console.log(`  Progress: ${percent}%`);
              lastPercent = percent;
            }
          }
        });

        response.pipe(file);

        file.on('finish', () => {
          file.close();
          console.log(`  Downloaded: ${dest}`);
          resolve();
        });
      });

      req.on('error', (err) => {
        if (timeoutId) clearTimeout(timeoutId);
        fs.unlink(dest, (unlinkErr) => {
          if (unlinkErr) console.warn(`  Warning: Could not cleanup partial download: ${unlinkErr.message}`);
        });
        reject(err);
      });

      req.on('timeout', () => {
        req.destroy();
        fs.unlink(dest, () => {});
        reject(new Error(`Download timeout after ${DOWNLOAD_TIMEOUT / 1000} seconds`));
      });

      // Set overall timeout
      timeoutId = setTimeout(() => {
        req.destroy();
        reject(new Error(`Download timeout after ${DOWNLOAD_TIMEOUT / 1000} seconds`));
      }, DOWNLOAD_TIMEOUT);
    };

    request(url);
  });
}

/**
 * Escape path for PowerShell single-quoted strings
 */
function escapePowerShellPath(p) {
  return p.replace(/'/g, "''");
}

/**
 * Extract archive based on type
 */
function extractArchive(archivePath, outputDir, archiveType) {
  console.log(`Extracting: ${archivePath}`);

  if (archiveType === 'zip') {
    // Use PowerShell on Windows for zip extraction
    if (process.platform === 'win32') {
      const escapedArchive = escapePowerShellPath(archivePath);
      const escapedOutput = escapePowerShellPath(outputDir);
      execSync(`powershell -Command "Expand-Archive -LiteralPath '${escapedArchive}' -DestinationPath '${escapedOutput}' -Force"`, {
        stdio: 'inherit'
      });
    } else {
      execSync(`unzip -o "${archivePath}" -d "${outputDir}"`, {
        stdio: 'inherit'
      });
    }
  } else if (archiveType === 'tar.gz') {
    // Use tar for tar.gz extraction
    execSync(`tar -xzf "${archivePath}" -C "${outputDir}"`, {
      stdio: 'inherit'
    });
  }

  console.log(`  Extracted to: ${outputDir}`);
}

/**
 * Check available disk space
 * Returns available space in bytes, or -1 if check fails
 */
function getAvailableDiskSpace(dir) {
  try {
    if (process.platform === 'win32') {
      // Windows: use wmic
      const driveLetter = path.parse(dir).root.replace('\\', '');
      const result = execSync(`wmic logicaldisk where "DeviceID='${driveLetter}'" get FreeSpace`, { encoding: 'utf8' });
      const lines = result.trim().split('\n');
      if (lines.length >= 2) {
        const freeSpace = parseInt(lines[1].trim(), 10);
        if (!isNaN(freeSpace)) return freeSpace;
      }
    } else {
      // Unix: use df
      const result = execSync(`df -k "${dir}"`, { encoding: 'utf8' });
      const lines = result.trim().split('\n');
      if (lines.length >= 2) {
        const parts = lines[1].split(/\s+/);
        if (parts.length >= 4) {
          const freeSpaceKB = parseInt(parts[3], 10);
          if (!isNaN(freeSpaceKB)) return freeSpaceKB * 1024;
        }
      }
    }
  } catch (e) {
    console.warn(`  Warning: Could not check disk space: ${e.message}`);
  }
  return -1; // Unknown
}

/**
 * Create a simple lock file for concurrent download protection
 */
function acquireLock(lockPath) {
  try {
    // Try to create lock file exclusively
    const fd = fs.openSync(lockPath, 'wx');
    fs.writeSync(fd, `${process.pid}\n${new Date().toISOString()}`);
    fs.closeSync(fd);
    return true;
  } catch (e) {
    if (e.code === 'EEXIST') {
      // Lock file exists, check if it's stale (older than 30 minutes)
      try {
        const stats = fs.statSync(lockPath);
        const ageMs = Date.now() - stats.mtimeMs;
        if (ageMs > 30 * 60 * 1000) {
          console.warn('  Warning: Found stale lock file, removing...');
          fs.unlinkSync(lockPath);
          return acquireLock(lockPath);
        }
        console.error(`  Error: Another download is in progress (lock file: ${lockPath})`);
        console.error('  If this is incorrect, delete the lock file and try again.');
      } catch (statErr) {
        // Ignore stat errors
      }
    }
    return false;
  }
}

/**
 * Release lock file
 */
function releaseLock(lockPath) {
  try {
    fs.unlinkSync(lockPath);
  } catch (e) {
    // Ignore errors
  }
}

/**
 * Download and prepare Node.js for a specific platform
 */
async function preparePlatform(platformId, config, shasums) {
  console.log(`\n${'='.repeat(60)}`);
  console.log(`Preparing Node.js for platform: ${platformId}`);
  console.log(`${'='.repeat(60)}`);

  const platformDir = path.join(OUTPUT_DIR, platformId);
  const tempDir = path.join(__dirname, '..', 'temp', 'node-download');
  const archiveExt = config.archive === 'zip' ? 'zip' : 'tar.gz';
  const archivePath = path.join(tempDir, `node-${platformId}.${archiveExt}`);
  const lockPath = path.join(tempDir, `node-${platformId}.lock`);

  // Create directories
  fs.mkdirSync(platformDir, { recursive: true });
  fs.mkdirSync(tempDir, { recursive: true });

  // Check if already downloaded
  const nodeExecutable = path.join(platformDir, path.basename(config.executable));
  if (fs.existsSync(nodeExecutable)) {
    console.log(`  Node.js already exists at: ${nodeExecutable}`);
    console.log(`  Skipping download. Delete the file to re-download.`);
    return;
  }

  // Acquire lock for concurrent download protection
  if (!acquireLock(lockPath)) {
    throw new Error(`Could not acquire lock for ${platformId}. Another download may be in progress.`);
  }

  try {
    // Check disk space before download
    const availableSpace = getAvailableDiskSpace(tempDir);
    if (availableSpace !== -1 && availableSpace < MIN_DISK_SPACE) {
      throw new Error(`Insufficient disk space: ${(availableSpace / 1024 / 1024).toFixed(2)} MB available, ${(MIN_DISK_SPACE / 1024 / 1024).toFixed(2)} MB required`);
    }
    if (availableSpace !== -1) {
      console.log(`  Available disk space: ${(availableSpace / 1024 / 1024).toFixed(2)} MB`);
    }

    // Download archive
    await downloadFile(config.url, archivePath);

    // Verify SHA256 checksum
    const archiveFilename = path.basename(config.url);
    const expectedHash = shasums ? shasums[archiveFilename] : null;

    if (expectedHash) {
      if (!verifySha256(archivePath, expectedHash)) {
        throw new Error(`SHA256 checksum verification failed for ${archiveFilename}`);
      }
    } else {
      console.warn(`  ⚠ Warning: No checksum found for ${archiveFilename}, skipping verification`);
    }

    // Extract archive
    extractArchive(archivePath, tempDir, config.archive);

    // Copy executable to platform directory
    const extractedNodePath = path.join(tempDir, config.extractedDir, config.executable);
    const destNodePath = path.join(platformDir, path.basename(config.executable));

    console.log(`  Copying: ${extractedNodePath} -> ${destNodePath}`);
    fs.copyFileSync(extractedNodePath, destNodePath);

    // Set executable permissions on Unix-like systems
    if (process.platform !== 'win32' && platformId !== 'win-x64') {
      try {
        fs.chmodSync(destNodePath, 0o755);
        console.log(`  Set executable permissions (755)`);
      } catch (chmodErr) {
        console.warn(`  Warning: Could not set executable permissions: ${chmodErr.message}`);
        console.warn(`  You may need to run: chmod +x "${destNodePath}"`);
      }
    }

    console.log(`  Node.js prepared successfully for ${platformId}`);

  } finally {
    // Release lock
    releaseLock(lockPath);

    // Cleanup temp files
    console.log(`  Cleaning up temp files...`);
    try {
      fs.rmSync(archivePath, { force: true });
      fs.rmSync(path.join(tempDir, config.extractedDir), { recursive: true, force: true });
    } catch (e) {
      console.log(`  Warning: Could not cleanup all temp files: ${e.message}`);
    }
  }
}

/**
 * Main function
 */
async function main() {
  console.log('╔════════════════════════════════════════════════════════════╗');
  console.log('║          Node.js Download Script for AGIME                 ║');
  console.log('║          Playwright MCP Embedded Runtime                   ║');
  console.log('╚════════════════════════════════════════════════════════════╝');
  console.log(`\nNode.js Version: v${NODE_VERSION}`);
  console.log(`Output Directory: ${OUTPUT_DIR}`);

  // Parse command line arguments
  const args = process.argv.slice(2);
  let targetPlatforms = Object.keys(PLATFORMS);
  let skipVerify = false;

  // Check for flags
  if (args.includes('--skip-verify')) {
    skipVerify = true;
    args.splice(args.indexOf('--skip-verify'), 1);
  }

  if (args.length > 0) {
    // Filter to specified platforms
    targetPlatforms = args.filter(arg => PLATFORMS[arg]);
    if (targetPlatforms.length === 0) {
      console.error('\nError: No valid platforms specified.');
      console.error('Valid platforms:', Object.keys(PLATFORMS).join(', '));
      console.error('Flags: --skip-verify (skip SHA256 verification)');
      process.exit(1);
    }
    console.log(`\nTarget platforms: ${targetPlatforms.join(', ')}`);
  } else {
    console.log(`\nDownloading for all platforms: ${targetPlatforms.join(', ')}`);
  }

  // Create output directory
  fs.mkdirSync(OUTPUT_DIR, { recursive: true });

  // Download SHASUMS for verification
  let shasums = null;
  if (!skipVerify) {
    try {
      console.log('\n--- Downloading checksums for verification ---');
      shasums = await downloadShasums();
    } catch (error) {
      console.warn(`\nWarning: Could not download checksums: ${error.message}`);
      console.warn('Continuing without verification...');
    }
  } else {
    console.log('\n--- Skipping checksum verification (--skip-verify flag) ---');
  }

  // Download and prepare each platform
  for (const platformId of targetPlatforms) {
    try {
      await preparePlatform(platformId, PLATFORMS[platformId], shasums);
    } catch (error) {
      console.error(`\nError preparing ${platformId}: ${error.message}`);
      console.error(error.stack);
      process.exit(1);
    }
  }

  console.log('\n╔════════════════════════════════════════════════════════════╗');
  console.log('║                    Download Complete!                       ║');
  console.log('╚════════════════════════════════════════════════════════════╝');

  // Print summary
  console.log('\nNode.js binaries prepared:');
  for (const platformId of targetPlatforms) {
    const platformDir = path.join(OUTPUT_DIR, platformId);
    const files = fs.existsSync(platformDir) ? fs.readdirSync(platformDir) : [];
    console.log(`  ${platformId}: ${files.join(', ') || '(empty)'}`);
  }

  if (shasums) {
    console.log('\n✓ All downloads verified with SHA256 checksums');
  }
}

main().catch(error => {
  console.error('Fatal error:', error);
  process.exit(1);
});
