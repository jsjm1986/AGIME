#!/usr/bin/env node

/**
 * Prepare @playwright/mcp package for embedding
 * This script installs @playwright/mcp and its dependencies for bundling
 *
 * The MCP package is platform-independent (JavaScript), so we only need one copy
 * that will work with all platform-specific Node.js binaries.
 */

const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

// @playwright/mcp version to install
const PLAYWRIGHT_MCP_VERSION = '0.0.53';

// Output directory
const OUTPUT_DIR = path.join(__dirname, '..', 'src', 'playwright', 'mcp');

// Temporary working directory
const TEMP_DIR = path.join(__dirname, '..', 'temp', 'playwright-mcp-install');

/**
 * Create package.json for the MCP package
 */
function createPackageJson() {
  const packageJson = {
    name: "agime-playwright-mcp",
    version: "1.0.0",
    description: "Embedded @playwright/mcp for AGIME",
    private: true,
    main: "node_modules/@playwright/mcp/cli.js",
    dependencies: {
      // Use exact version (no ^ prefix) for reproducible builds
      "@playwright/mcp": PLAYWRIGHT_MCP_VERSION
    }
  };

  return JSON.stringify(packageJson, null, 2);
}

/**
 * Clean directory
 */
function cleanDirectory(dir) {
  if (fs.existsSync(dir)) {
    console.log(`Cleaning directory: ${dir}`);
    fs.rmSync(dir, { recursive: true, force: true });
  }
  fs.mkdirSync(dir, { recursive: true });
}

/**
 * Install npm packages
 */
function installPackages(workDir) {
  console.log('\nInstalling @playwright/mcp and dependencies...');
  console.log(`Working directory: ${workDir}`);

  try {
    // Use npm install with production flag to minimize size
    execSync('npm install --production --no-optional --ignore-scripts', {
      cwd: workDir,
      stdio: 'inherit',
      env: {
        ...process.env,
        // Disable Playwright browser download during npm install
        PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD: '1'
      }
    });

    console.log('npm install completed successfully');
  } catch (error) {
    console.error('npm install failed:', error.message);
    throw error;
  }
}

/**
 * Copy installed packages to output directory
 */
function copyToOutput(sourceDir, outputDir) {
  console.log(`\nCopying installed packages to: ${outputDir}`);

  // Clean output directory
  cleanDirectory(outputDir);

  // Copy package.json
  const packageJsonSrc = path.join(sourceDir, 'package.json');
  const packageJsonDest = path.join(outputDir, 'package.json');
  fs.copyFileSync(packageJsonSrc, packageJsonDest);
  console.log('  Copied package.json');

  // Copy package-lock.json if exists
  const packageLockSrc = path.join(sourceDir, 'package-lock.json');
  if (fs.existsSync(packageLockSrc)) {
    const packageLockDest = path.join(outputDir, 'package-lock.json');
    fs.copyFileSync(packageLockSrc, packageLockDest);
    console.log('  Copied package-lock.json');
  }

  // Copy node_modules
  const nodeModulesSrc = path.join(sourceDir, 'node_modules');
  const nodeModulesDest = path.join(outputDir, 'node_modules');

  console.log('  Copying node_modules (this may take a moment)...');
  copyDirectoryRecursive(nodeModulesSrc, nodeModulesDest);
  console.log('  Copied node_modules');
}

/**
 * Recursively copy directory
 */
function copyDirectoryRecursive(src, dest) {
  fs.mkdirSync(dest, { recursive: true });

  const entries = fs.readdirSync(src, { withFileTypes: true });

  for (const entry of entries) {
    const srcPath = path.join(src, entry.name);
    const destPath = path.join(dest, entry.name);

    if (entry.isDirectory()) {
      copyDirectoryRecursive(srcPath, destPath);
    } else if (entry.isSymbolicLink()) {
      // Handle symlinks (common in node_modules)
      try {
        const linkTarget = fs.readlinkSync(srcPath);
        fs.symlinkSync(linkTarget, destPath);
      } catch (e) {
        // If symlink fails, copy the file instead
        fs.copyFileSync(srcPath, destPath);
      }
    } else {
      fs.copyFileSync(srcPath, destPath);
    }
  }
}

/**
 * Calculate directory size
 */
function getDirectorySize(dir) {
  let totalSize = 0;

  function walkDir(currentPath) {
    const entries = fs.readdirSync(currentPath, { withFileTypes: true });
    for (const entry of entries) {
      const fullPath = path.join(currentPath, entry.name);
      if (entry.isDirectory()) {
        walkDir(fullPath);
      } else {
        try {
          const stats = fs.statSync(fullPath);
          totalSize += stats.size;
        } catch (e) {
          // Ignore errors for symlinks, etc.
        }
      }
    }
  }

  if (fs.existsSync(dir)) {
    walkDir(dir);
  }

  return totalSize;
}

/**
 * Format bytes to human readable
 */
function formatBytes(bytes) {
  if (bytes < 1024) return bytes + ' B';
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(2) + ' KB';
  return (bytes / (1024 * 1024)).toFixed(2) + ' MB';
}

/**
 * Remove unnecessary files from node_modules to reduce package size
 * This includes TypeScript definitions, README files, LICENSE files, etc.
 */
function cleanupUnnecessaryFiles(outputDir) {
  console.log('\nCleaning up unnecessary files to reduce package size...');

  const nodeModulesDir = path.join(outputDir, 'node_modules');
  if (!fs.existsSync(nodeModulesDir)) {
    return 0;
  }

  // Patterns of files to remove (case-insensitive)
  const patternsToRemove = [
    /\.d\.ts$/i,           // TypeScript definition files
    /\.d\.ts\.map$/i,      // TypeScript definition source maps
    /\.ts\.map$/i,         // TypeScript source maps
    /^readme\.md$/i,       // README files
    /^changelog\.md$/i,    // Changelog files
    /^history\.md$/i,      // History files
    /^authors$/i,          // Authors files
    /^contributors$/i,     // Contributors files
    /\.markdown$/i,        // Markdown files (but not .md in general)
    /^makefile$/i,         // Makefiles
    /^gulpfile\.js$/i,     // Gulp files
    /^gruntfile\.js$/i,    // Grunt files
    /\.tsbuildinfo$/i,     // TypeScript build info
  ];

  let removedCount = 0;
  let removedSize = 0;

  function walkAndClean(dir) {
    try {
      const entries = fs.readdirSync(dir, { withFileTypes: true });

      for (const entry of entries) {
        const fullPath = path.join(dir, entry.name);

        if (entry.isDirectory()) {
          // Skip some directories entirely
          if (entry.name === '.git' || entry.name === 'test' || entry.name === 'tests' ||
              entry.name === '__tests__' || entry.name === 'docs' || entry.name === 'doc') {
            try {
              const stats = fs.statSync(fullPath);
              removedSize += getDirectorySize(fullPath);
              fs.rmSync(fullPath, { recursive: true, force: true });
              removedCount++;
            } catch (e) {
              // Ignore errors
            }
          } else {
            walkAndClean(fullPath);
          }
        } else {
          // Check if file matches any pattern to remove
          const shouldRemove = patternsToRemove.some(pattern => pattern.test(entry.name));
          if (shouldRemove) {
            try {
              const stats = fs.statSync(fullPath);
              removedSize += stats.size;
              fs.unlinkSync(fullPath);
              removedCount++;
            } catch (e) {
              // Ignore errors
            }
          }
        }
      }
    } catch (e) {
      // Ignore directory read errors
    }
  }

  walkAndClean(nodeModulesDir);

  console.log(`  Removed ${removedCount} unnecessary files/directories`);
  console.log(`  Saved approximately ${formatBytes(removedSize)}`);

  return removedSize;
}

/**
 * Verify installation
 */
function verifyInstallation(outputDir) {
  console.log('\nVerifying installation...');

  const mcpCliPath = path.join(outputDir, 'node_modules', '@playwright', 'mcp', 'cli.js');
  const mcpPackageJsonPath = path.join(outputDir, 'node_modules', '@playwright', 'mcp', 'package.json');

  if (!fs.existsSync(mcpCliPath)) {
    throw new Error(`@playwright/mcp cli.js not found at: ${mcpCliPath}`);
  }

  if (!fs.existsSync(mcpPackageJsonPath)) {
    throw new Error(`@playwright/mcp package.json not found at: ${mcpPackageJsonPath}`);
  }

  // Read and verify version
  const mcpPackageJson = JSON.parse(fs.readFileSync(mcpPackageJsonPath, 'utf8'));
  console.log(`  @playwright/mcp version: ${mcpPackageJson.version}`);
  console.log(`  Entry point: ${mcpCliPath}`);

  // Calculate and display size
  const totalSize = getDirectorySize(outputDir);
  console.log(`  Total size: ${formatBytes(totalSize)}`);

  console.log('\nInstallation verified successfully!');
}

/**
 * Create launcher script for cross-platform execution
 */
function createLauncherScript(outputDir) {
  console.log('\nCreating launcher script...');

  // Create a simple launcher that can be called by the Rust backend
  const launcherContent = `#!/usr/bin/env node
// AGIME Playwright MCP Launcher
// This script launches @playwright/mcp with the embedded Node.js runtime

const path = require('path');
const { spawn } = require('child_process');

// Path to the MCP CLI
const mcpCli = path.join(__dirname, 'node_modules', '@playwright', 'mcp', 'cli.js');

// Forward all arguments to the MCP CLI
const args = [mcpCli, ...process.argv.slice(2)];

// Start the MCP server
const child = spawn(process.execPath, args, {
  stdio: 'inherit',
  env: {
    ...process.env,
    // Ensure Playwright doesn't try to download browsers at runtime
    PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD: '1'
  }
});

child.on('exit', (code) => {
  process.exit(code || 0);
});
`;

  const launcherPath = path.join(outputDir, 'launcher.js');
  fs.writeFileSync(launcherPath, launcherContent);
  console.log(`  Created: ${launcherPath}`);
}

/**
 * Main function
 */
async function main() {
  console.log('╔════════════════════════════════════════════════════════════╗');
  console.log('║       Playwright MCP Preparation Script for AGIME          ║');
  console.log('╚════════════════════════════════════════════════════════════╝');
  console.log(`\n@playwright/mcp Version: ${PLAYWRIGHT_MCP_VERSION}`);
  console.log(`Output Directory: ${OUTPUT_DIR}`);

  // Check if npm is available
  try {
    const npmVersion = execSync('npm --version', { encoding: 'utf8' }).trim();
    console.log(`npm version: ${npmVersion}`);
  } catch (error) {
    console.error('Error: npm is not available. Please install Node.js first.');
    process.exit(1);
  }

  // Check if output already exists and is valid
  const mcpCliPath = path.join(OUTPUT_DIR, 'node_modules', '@playwright', 'mcp', 'cli.js');
  if (fs.existsSync(mcpCliPath)) {
    console.log('\n@playwright/mcp already installed. Checking version...');
    try {
      const mcpPackageJsonPath = path.join(OUTPUT_DIR, 'node_modules', '@playwright', 'mcp', 'package.json');
      const mcpPackageJson = JSON.parse(fs.readFileSync(mcpPackageJsonPath, 'utf8'));

      if (mcpPackageJson.version === PLAYWRIGHT_MCP_VERSION) {
        console.log(`Version ${PLAYWRIGHT_MCP_VERSION} is already installed.`);
        console.log('Use --force flag to reinstall.');

        if (!process.argv.includes('--force')) {
          return;
        }
        console.log('--force flag detected, reinstalling...');
      } else {
        console.log(`Installed version ${mcpPackageJson.version} differs from target ${PLAYWRIGHT_MCP_VERSION}`);
        console.log('Reinstalling...');
      }
    } catch (e) {
      console.log('Could not verify existing installation, reinstalling...');
    }
  }

  let tempDirCreated = false;

  try {
    // Step 1: Clean and create temp directory
    console.log('\n--- Step 1: Preparing workspace ---');
    cleanDirectory(TEMP_DIR);
    tempDirCreated = true;

    // Step 2: Create package.json
    console.log('\n--- Step 2: Creating package.json ---');
    const packageJsonPath = path.join(TEMP_DIR, 'package.json');
    fs.writeFileSync(packageJsonPath, createPackageJson());
    console.log(`Created: ${packageJsonPath}`);

    // Step 3: Install packages
    console.log('\n--- Step 3: Installing packages ---');
    installPackages(TEMP_DIR);

    // Step 4: Copy to output directory
    console.log('\n--- Step 4: Copying to output directory ---');
    copyToOutput(TEMP_DIR, OUTPUT_DIR);

    // Step 5: Cleanup unnecessary files to reduce package size
    console.log('\n--- Step 5: Cleaning up unnecessary files ---');
    cleanupUnnecessaryFiles(OUTPUT_DIR);

    // Step 6: Create launcher script
    console.log('\n--- Step 6: Creating launcher script ---');
    createLauncherScript(OUTPUT_DIR);

    // Step 7: Verify installation
    console.log('\n--- Step 7: Verifying installation ---');
    verifyInstallation(OUTPUT_DIR);

    console.log('\n╔════════════════════════════════════════════════════════════╗');
    console.log('║              Preparation Complete!                          ║');
    console.log('╚════════════════════════════════════════════════════════════╝');

  } catch (error) {
    console.error('\nError during preparation:', error.message);
    console.error(error.stack);
    process.exit(1);
  } finally {
    // Always cleanup temp directory, even on failure
    if (tempDirCreated) {
      console.log('\n--- Cleanup ---');
      try {
        fs.rmSync(TEMP_DIR, { recursive: true, force: true });
        console.log('Cleaned up temp directory');
      } catch (e) {
        console.warn(`Warning: Could not cleanup temp directory: ${e.message}`);
      }
    }
  }
}

main().catch(error => {
  console.error('Fatal error:', error);
  process.exit(1);
});
