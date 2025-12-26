#!/usr/bin/env node

/**
 * Build Playwright Embedded Runtime for AGIME
 *
 * This is the main orchestrator script that:
 * 1. Downloads Node.js runtimes for target platforms
 * 2. Installs @playwright/mcp with dependencies
 * 3. Prepares the final embedded package structure
 *
 * Usage:
 *   node build-playwright.js              # Build for current platform
 *   node build-playwright.js --all        # Build for all platforms
 *   node build-playwright.js --platform win-x64  # Build for specific platform
 *   node build-playwright.js --clean      # Clean all build artifacts
 */

const fs = require('fs');
const path = require('path');
const { execSync, spawn } = require('child_process');

// Configuration
const SCRIPTS_DIR = __dirname;
const PLAYWRIGHT_DIR = path.join(__dirname, '..', 'src', 'playwright');
const NODE_DIR = path.join(PLAYWRIGHT_DIR, 'node');
const MCP_DIR = path.join(PLAYWRIGHT_DIR, 'mcp');

// Platform mapping
const PLATFORM_MAP = {
  'win32-x64': 'win-x64',
  'darwin-x64': 'darwin-x64',
  'darwin-arm64': 'darwin-arm64',
  'linux-x64': 'linux-x64',
  'linux-arm64': 'linux-arm64'
};

const ALL_PLATFORMS = ['win-x64', 'darwin-x64', 'darwin-arm64', 'linux-x64', 'linux-arm64'];

/**
 * Get current platform identifier
 */
function getCurrentPlatform() {
  const key = `${process.platform}-${process.arch}`;
  return PLATFORM_MAP[key] || null;
}

/**
 * Run a script synchronously
 */
function runScript(scriptPath, args = []) {
  console.log(`\n${'─'.repeat(60)}`);
  console.log(`Running: node ${path.basename(scriptPath)} ${args.join(' ')}`);
  console.log('─'.repeat(60));

  execSync(`node "${scriptPath}" ${args.join(' ')}`, {
    stdio: 'inherit',
    cwd: SCRIPTS_DIR
  });
}

/**
 * Clean all build artifacts
 */
function cleanAll() {
  console.log('\nCleaning Playwright build artifacts...');

  const dirsToClean = [
    PLAYWRIGHT_DIR,
    path.join(__dirname, '..', 'temp', 'node-download'),
    path.join(__dirname, '..', 'temp', 'playwright-mcp-install')
  ];

  for (const dir of dirsToClean) {
    if (fs.existsSync(dir)) {
      console.log(`  Removing: ${dir}`);
      fs.rmSync(dir, { recursive: true, force: true });
    }
  }

  console.log('Clean complete.');
}

/**
 * Verify the build output
 */
function verifyBuild(platforms) {
  console.log('\n' + '═'.repeat(60));
  console.log('Build Verification');
  console.log('═'.repeat(60));

  let success = true;

  // Check MCP installation
  const mcpCliPath = path.join(MCP_DIR, 'node_modules', '@playwright', 'mcp', 'cli.js');
  if (fs.existsSync(mcpCliPath)) {
    console.log('✓ @playwright/mcp installed');
  } else {
    console.log('✗ @playwright/mcp NOT found');
    success = false;
  }

  // Check Node.js binaries for each platform
  for (const platform of platforms) {
    const nodeExe = platform === 'win-x64' ? 'node.exe' : 'node';
    const nodePath = path.join(NODE_DIR, platform, nodeExe);

    if (fs.existsSync(nodePath)) {
      const stats = fs.statSync(nodePath);
      const sizeMB = (stats.size / (1024 * 1024)).toFixed(2);
      console.log(`✓ Node.js for ${platform} (${sizeMB} MB)`);
    } else {
      console.log(`✗ Node.js for ${platform} NOT found`);
      success = false;
    }
  }

  // Calculate total size
  if (fs.existsSync(PLAYWRIGHT_DIR)) {
    let totalSize = 0;

    function walkDir(dir) {
      const entries = fs.readdirSync(dir, { withFileTypes: true });
      for (const entry of entries) {
        const fullPath = path.join(dir, entry.name);
        if (entry.isDirectory()) {
          walkDir(fullPath);
        } else {
          try {
            totalSize += fs.statSync(fullPath).size;
          } catch (e) {}
        }
      }
    }

    walkDir(PLAYWRIGHT_DIR);
    const totalMB = (totalSize / (1024 * 1024)).toFixed(2);
    console.log(`\nTotal embedded size: ${totalMB} MB`);
  }

  return success;
}

/**
 * Create the runner script that will be used by Rust backend
 * NOTE: Only creates if run.js doesn't exist, as it may have been manually customized
 */
function createRunnerScript() {
  console.log('\nChecking platform runner scripts...');

  const runnerPath = path.join(PLAYWRIGHT_DIR, 'run.js');

  // Check if run.js already exists (manually maintained)
  if (fs.existsSync(runnerPath)) {
    console.log(`  run.js already exists (manually maintained), skipping generation`);
  } else {
    console.log(`  run.js not found, creating default version...`);
    console.log(`  WARNING: For full functionality (signal handling, env isolation), use the manually maintained run.js`);

    // Create a basic script that determines the right Node.js binary at runtime
    const runnerContent = `#!/usr/bin/env node
/**
 * AGIME Playwright MCP Runner (Auto-generated basic version)
 * For full functionality, use the manually maintained run.js from source control.
 */

const path = require('path');
const { spawn } = require('child_process');
const os = require('os');

const platform = os.platform();
const arch = os.arch();

let platformDir;
if (platform === 'win32' && arch === 'x64') {
  platformDir = 'win-x64';
} else if (platform === 'darwin' && arch === 'x64') {
  platformDir = 'darwin-x64';
} else if (platform === 'darwin' && arch === 'arm64') {
  platformDir = 'darwin-arm64';
} else if (platform === 'linux' && arch === 'x64') {
  platformDir = 'linux-x64';
} else if (platform === 'linux' && arch === 'arm64') {
  platformDir = 'linux-arm64';
} else {
  console.error('Unsupported platform:', platform, arch);
  process.exit(1);
}

const nodeDir = path.join(__dirname, 'node', platformDir);
const nodeExe = platform === 'win32' ? 'node.exe' : 'node';
const nodePath = path.join(nodeDir, nodeExe);
const mcpCli = path.join(__dirname, 'mcp', 'node_modules', '@playwright', 'mcp', 'cli.js');

const fs = require('fs');
if (!fs.existsSync(nodePath)) {
  console.error('Node.js binary not found:', nodePath);
  process.exit(1);
}
if (!fs.existsSync(mcpCli)) {
  console.error('Playwright MCP not found:', mcpCli);
  process.exit(1);
}

const args = [mcpCli, ...process.argv.slice(2)];
const child = spawn(nodePath, args, {
  stdio: 'inherit',
  env: { ...process.env, PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD: '1' }
});

child.on('error', (err) => {
  console.error('Failed to start Playwright MCP:', err);
  process.exit(1);
});

child.on('exit', (code) => {
  process.exit(code || 0);
});
`;

    fs.mkdirSync(PLAYWRIGHT_DIR, { recursive: true });
    fs.writeFileSync(runnerPath, runnerContent);
    console.log(`  Created: ${runnerPath}`);
  }

  // Also create a shell script for Unix systems (if not exists)
  const shRunnerPath = path.join(PLAYWRIGHT_DIR, 'run.sh');
  if (!fs.existsSync(shRunnerPath)) {
    const shRunner = `#!/bin/sh
# AGIME Playwright MCP Runner (Shell)
DIR="$(cd "$(dirname "$0")" && pwd)"
exec node "$DIR/run.js" "$@"
`;
    fs.writeFileSync(shRunnerPath, shRunner);
    if (process.platform !== 'win32') {
      fs.chmodSync(shRunnerPath, 0o755);
    }
    console.log(`  Created: ${shRunnerPath}`);
  } else {
    console.log(`  run.sh already exists, skipping`);
  }

  // Create batch file for Windows (if not exists)
  const batRunnerPath = path.join(PLAYWRIGHT_DIR, 'run.bat');
  if (!fs.existsSync(batRunnerPath)) {
    const batRunner = `@echo off
REM AGIME Playwright MCP Runner (Batch)
node "%~dp0run.js" %*
`;
    fs.writeFileSync(batRunnerPath, batRunner);
    console.log(`  Created: ${batRunnerPath}`);
  } else {
    console.log(`  run.bat already exists, skipping`);
  }
}

/**
 * Main function
 */
async function main() {
  console.log('╔════════════════════════════════════════════════════════════╗');
  console.log('║        AGIME Playwright Embedded Runtime Builder           ║');
  console.log('╚════════════════════════════════════════════════════════════╝');

  // Parse arguments
  const args = process.argv.slice(2);
  let targetPlatforms = [];
  let shouldClean = false;

  for (let i = 0; i < args.length; i++) {
    const arg = args[i];
    if (arg === '--clean') {
      shouldClean = true;
    } else if (arg === '--all') {
      targetPlatforms = ALL_PLATFORMS;
    } else if (arg === '--platform' && args[i + 1]) {
      targetPlatforms.push(args[i + 1]);
      i++;
    } else if (ALL_PLATFORMS.includes(arg)) {
      targetPlatforms.push(arg);
    }
  }

  // Handle clean
  if (shouldClean) {
    cleanAll();
    if (targetPlatforms.length === 0) {
      return;
    }
  }

  // Default to current platform if none specified
  if (targetPlatforms.length === 0) {
    const currentPlatform = getCurrentPlatform();
    if (currentPlatform) {
      targetPlatforms = [currentPlatform];
      console.log(`\nNo platform specified, using current: ${currentPlatform}`);
    } else {
      console.log('\nNo platform specified. Building for all platforms.');
      targetPlatforms = ALL_PLATFORMS;
    }
  }

  console.log(`\nTarget platforms: ${targetPlatforms.join(', ')}`);
  console.log(`Output directory: ${PLAYWRIGHT_DIR}`);

  // Step 1: Download Node.js
  console.log('\n' + '═'.repeat(60));
  console.log('Phase 1: Downloading Node.js Runtimes');
  console.log('═'.repeat(60));

  const downloadNodeScript = path.join(SCRIPTS_DIR, 'download-node.js');
  runScript(downloadNodeScript, targetPlatforms);

  // Step 2: Install @playwright/mcp
  console.log('\n' + '═'.repeat(60));
  console.log('Phase 2: Installing @playwright/mcp');
  console.log('═'.repeat(60));

  const prepareMcpScript = path.join(SCRIPTS_DIR, 'prepare-playwright-mcp.js');
  runScript(prepareMcpScript);

  // Step 3: Create runner scripts
  console.log('\n' + '═'.repeat(60));
  console.log('Phase 3: Creating Runner Scripts');
  console.log('═'.repeat(60));

  createRunnerScript();

  // Step 4: Verify build
  const buildSuccess = verifyBuild(targetPlatforms);

  // Final summary
  console.log('\n' + '═'.repeat(60));
  if (buildSuccess) {
    console.log('BUILD SUCCESSFUL');
    console.log('═'.repeat(60));
    console.log(`\nPlaywright embedded runtime is ready at:`);
    console.log(`  ${PLAYWRIGHT_DIR}`);
    console.log(`\nTo test, run:`);
    console.log(`  node "${path.join(PLAYWRIGHT_DIR, 'run.js')}"`);
  } else {
    console.log('BUILD FAILED - Some components are missing');
    console.log('═'.repeat(60));
    process.exit(1);
  }
}

main().catch(error => {
  console.error('Fatal error:', error);
  process.exit(1);
});
