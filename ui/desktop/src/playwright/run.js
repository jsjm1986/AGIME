#!/usr/bin/env node
/**
 * AGIME Playwright MCP Runner
 * Automatically selects the correct Node.js binary for the current platform
 * and launches the @playwright/mcp server.
 */

const path = require('path');
const { spawn } = require('child_process');
const os = require('os');

// Determine platform
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
  console.error('Please run Playwright MCP using npx instead.');
  process.exit(1);
}

// Paths
const nodeDir = path.join(__dirname, 'node', platformDir);
const nodeExe = platform === 'win32' ? 'node.exe' : 'node';
const nodePath = path.join(nodeDir, nodeExe);
const mcpCli = path.join(__dirname, 'mcp', 'node_modules', '@playwright', 'mcp', 'cli.js');

// Verify paths exist
const fs = require('fs');
if (!fs.existsSync(nodePath)) {
  console.error('Node.js binary not found:', nodePath);
  process.exit(1);
}
if (!fs.existsSync(mcpCli)) {
  console.error('Playwright MCP not found:', mcpCli);
  process.exit(1);
}

// Create isolated environment to avoid conflicts with user's Node.js
// Remove Node.js related environment variables that could cause conflicts
const isolatedEnv = { ...process.env };

// Remove variables that could interfere with embedded Node.js
const varsToRemove = [
  // Node.js core variables
  'NODE_PATH',
  'NODE_OPTIONS',
  'NODE_EXTRA_CA_CERTS',
  'NODE_REDIRECT_WARNINGS',
  'NODE_REPL_HISTORY',
  // npm/yarn/pnpm variables
  'npm_config_prefix',
  'npm_config_globalconfig',
  'npm_config_userconfig',
  'npm_config_cache',
  'npm_config_registry',
  'npm_execpath',
  'npm_node_execpath',
  'npm_package_json',
  // Version managers
  'NVM_DIR',
  'NVM_BIN',
  'NVM_INC',
  'NVM_CD_FLAGS',
  'FNM_DIR',
  'FNM_MULTISHELL_PATH',
  'VOLTA_HOME',
  'N_PREFIX',
  // Other potential conflicts
  'NODE_ICU_DATA',
  'UV_THREADPOOL_SIZE',
  // Debug and logging variables that could interfere
  'NODE_DEBUG',
  'NODE_DEBUG_NATIVE',
  'NODE_NO_WARNINGS',
  'NODE_PENDING_DEPRECATION',
  // Security related
  'NODE_TLS_REJECT_UNAUTHORIZED',
  'NODE_SKIP_PLATFORM_CHECK',
  // Module system
  'NODE_PRESERVE_SYMLINKS',
  'NODE_PENDING_PIPE_INSTANCES',
  // Coverage and profiling
  'NODE_V8_COVERAGE',
  // Other package managers
  'YARN_IGNORE_PATH',
  'PNPM_HOME',
];

varsToRemove.forEach(key => {
  delete isolatedEnv[key];
});

// Also remove any npm_* prefixed variables
Object.keys(isolatedEnv).forEach(key => {
  if (key.startsWith('npm_')) {
    delete isolatedEnv[key];
  }
});

// Set our own controlled environment
isolatedEnv.NODE_ENV = isolatedEnv.NODE_ENV || 'production';
isolatedEnv.PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD = '1';

// Launch MCP server with isolated environment
const args = [mcpCli, ...process.argv.slice(2)];
const child = spawn(nodePath, args, {
  stdio: 'inherit',
  env: isolatedEnv,
  cwd: path.dirname(mcpCli) // Set working directory to mcp package
});

child.on('error', (err) => {
  console.error('Failed to start Playwright MCP:', err);
  process.exit(1);
});

child.on('exit', (code) => {
  process.exit(code || 0);
});

// Handle termination signals to properly clean up child process
// This prevents browser processes from becoming orphans
process.on('SIGTERM', () => {
  child.kill('SIGTERM');
});

process.on('SIGINT', () => {
  child.kill('SIGINT');
});

// Handle parent process disconnect (e.g., when Rust process terminates)
process.on('disconnect', () => {
  child.kill('SIGTERM');
});
