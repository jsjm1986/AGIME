/**
 * Download Node.js binaries for embedding
 *
 * This script downloads Node.js LTS binaries for different platforms
 * to be embedded with the AGIME desktop application.
 *
 * Usage:
 *   node download.js [platform]
 *
 * Examples:
 *   node download.js              # Download for current platform
 *   node download.js all          # Download for all platforms
 *   node download.js win-x64      # Download for specific platform
 */

const https = require("https");
const fs = require("fs");
const path = require("path");
const { execSync } = require("child_process");

// Node.js LTS version to download
const NODE_VERSION = "v22.12.0";
const BASE_URL = `https://nodejs.org/dist/${NODE_VERSION}`;

// Platform configurations
// Note: For macOS/Linux, bin/npm and bin/npx are symlinks that break when flattened.
// We only copy bin/node and lib/node_modules, then create wrapper scripts.
const PLATFORMS = {
  "win-x64": {
    archive: `node-${NODE_VERSION}-win-x64.zip`,
    extractDir: `node-${NODE_VERSION}-win-x64`,
    files: ["node.exe", "npm.cmd", "npx.cmd", "node_modules"],
  },
  "darwin-x64": {
    archive: `node-${NODE_VERSION}-darwin-x64.tar.gz`,
    extractDir: `node-${NODE_VERSION}-darwin-x64`,
    files: ["bin/node", "lib/node_modules"],
    flatten: true,
    createWrappers: true,
  },
  "darwin-arm64": {
    archive: `node-${NODE_VERSION}-darwin-arm64.tar.gz`,
    extractDir: `node-${NODE_VERSION}-darwin-arm64`,
    files: ["bin/node", "lib/node_modules"],
    flatten: true,
    createWrappers: true,
  },
  "linux-x64": {
    archive: `node-${NODE_VERSION}-linux-x64.tar.gz`,
    extractDir: `node-${NODE_VERSION}-linux-x64`,
    files: ["bin/node", "lib/node_modules"],
    flatten: true,
    createWrappers: true,
  },
  "linux-arm64": {
    archive: `node-${NODE_VERSION}-linux-arm64.tar.gz`,
    extractDir: `node-${NODE_VERSION}-linux-arm64`,
    files: ["bin/node", "lib/node_modules"],
    flatten: true,
    createWrappers: true,
  },
};

const SCRIPT_DIR = __dirname;
const NODE_DIR = SCRIPT_DIR;  // Download directly to src/nodejs/{platform}
const TEMP_DIR = path.join(SCRIPT_DIR, ".temp");

function downloadFile(url, dest) {
  return new Promise((resolve, reject) => {
    console.log(`Downloading: ${url}`);
    const file = fs.createWriteStream(dest);

    const request = (url) => {
      https
        .get(url, (response) => {
          if (response.statusCode === 302 || response.statusCode === 301) {
            // Follow redirect
            request(response.headers.location);
            return;
          }

          if (response.statusCode !== 200) {
            reject(new Error(`HTTP ${response.statusCode}: ${url}`));
            return;
          }

          const totalSize = parseInt(response.headers["content-length"], 10);
          let downloaded = 0;

          response.on("data", (chunk) => {
            downloaded += chunk.length;
            const percent = ((downloaded / totalSize) * 100).toFixed(1);
            process.stdout.write(`\rDownloading: ${percent}%`);
          });

          response.pipe(file);

          file.on("finish", () => {
            file.close();
            console.log("\nDownload complete.");
            resolve();
          });
        })
        .on("error", (err) => {
          fs.unlink(dest, () => {});
          reject(err);
        });
    };

    request(url);
  });
}

function extractArchive(archivePath, destDir, isZip) {
  console.log(`Extracting: ${archivePath}`);

  if (!fs.existsSync(destDir)) {
    fs.mkdirSync(destDir, { recursive: true });
  }

  if (isZip) {
    // Use PowerShell on Windows to extract zip
    if (process.platform === "win32") {
      execSync(
        `powershell -Command "Expand-Archive -Path '${archivePath}' -DestinationPath '${destDir}' -Force"`,
        { stdio: "inherit" }
      );
    } else {
      execSync(`unzip -o "${archivePath}" -d "${destDir}"`, {
        stdio: "inherit",
      });
    }
  } else {
    // Extract tar.gz
    execSync(`tar -xzf "${archivePath}" -C "${destDir}"`, { stdio: "inherit" });
  }

  console.log("Extraction complete.");
}

function copyNodeFiles(platform, config) {
  const platformDir = path.join(NODE_DIR, platform);
  const extractedDir = path.join(TEMP_DIR, config.extractDir);

  if (!fs.existsSync(platformDir)) {
    fs.mkdirSync(platformDir, { recursive: true });
  }

  console.log(`Copying Node.js files to: ${platformDir}`);

  for (const file of config.files) {
    const srcPath = path.join(extractedDir, file);
    let destPath;

    if (config.flatten) {
      // Flatten directory structure (bin/node -> node, lib/node_modules -> node_modules)
      const basename = path.basename(file);
      if (file.startsWith("lib/")) {
        destPath = path.join(platformDir, basename);
      } else if (file.startsWith("bin/")) {
        destPath = path.join(platformDir, basename);
      } else {
        destPath = path.join(platformDir, basename);
      }
    } else {
      destPath = path.join(platformDir, path.basename(file));
    }

    if (fs.existsSync(srcPath)) {
      const stat = fs.lstatSync(srcPath);
      if (stat.isDirectory()) {
        // Copy directory recursively
        fs.cpSync(srcPath, destPath, { recursive: true });
      } else if (stat.isSymbolicLink()) {
        // Skip symlinks - we'll create wrapper scripts instead
        console.log(`  Skipped symlink: ${path.basename(file)}`);
        continue;
      } else {
        fs.copyFileSync(srcPath, destPath);
      }
      console.log(`  Copied: ${path.basename(file)}`);
    } else {
      console.warn(`  Warning: ${file} not found`);
    }
  }

  // Create wrapper scripts for npm and npx on Unix platforms
  if (config.createWrappers) {
    createUnixWrapperScripts(platformDir);
  }

  // Make binaries executable on Unix
  if (platform !== "win-x64") {
    const nodeBin = path.join(platformDir, "node");
    const npmBin = path.join(platformDir, "npm");
    const npxBin = path.join(platformDir, "npx");

    [nodeBin, npmBin, npxBin].forEach((bin) => {
      if (fs.existsSync(bin)) {
        fs.chmodSync(bin, 0o755);
      }
    });
  }
}

// Create npm and npx wrapper scripts for Unix platforms
function createUnixWrapperScripts(platformDir) {
  // npm wrapper script
  const npmScript = `#!/bin/sh
basedir=$(dirname "$(echo "$0" | sed -e 's,\\\\,/,g')")
case \`uname\` in
    *CYGWIN*|*MINGW*|*MSYS*) basedir=\`cygpath -w "$basedir"\`;;
esac
"$basedir/node" "$basedir/node_modules/npm/bin/npm-cli.js" "$@"
ret=$?
exit $ret
`;

  // npx wrapper script
  const npxScript = `#!/bin/sh
basedir=$(dirname "$(echo "$0" | sed -e 's,\\\\,/,g')")
case \`uname\` in
    *CYGWIN*|*MINGW*|*MSYS*) basedir=\`cygpath -w "$basedir"\`;;
esac
"$basedir/node" "$basedir/node_modules/npm/bin/npx-cli.js" "$@"
ret=$?
exit $ret
`;

  const npmPath = path.join(platformDir, "npm");
  const npxPath = path.join(platformDir, "npx");

  fs.writeFileSync(npmPath, npmScript, { mode: 0o755 });
  fs.writeFileSync(npxPath, npxScript, { mode: 0o755 });

  console.log("  Created: npm (wrapper script)");
  console.log("  Created: npx (wrapper script)");
}

async function downloadPlatform(platform) {
  const config = PLATFORMS[platform];
  if (!config) {
    console.error(`Unknown platform: ${platform}`);
    console.log(`Available platforms: ${Object.keys(PLATFORMS).join(", ")}`);
    process.exit(1);
  }

  console.log(`\n=== Downloading Node.js ${NODE_VERSION} for ${platform} ===\n`);

  // Create temp directory
  if (!fs.existsSync(TEMP_DIR)) {
    fs.mkdirSync(TEMP_DIR, { recursive: true });
  }

  const archiveUrl = `${BASE_URL}/${config.archive}`;
  const archivePath = path.join(TEMP_DIR, config.archive);
  const isZip = config.archive.endsWith(".zip");

  try {
    // Download archive
    await downloadFile(archiveUrl, archivePath);

    // Extract archive
    extractArchive(archivePath, TEMP_DIR, isZip);

    // Copy required files
    copyNodeFiles(platform, config);

    console.log(`\n[OK] Node.js ${NODE_VERSION} for ${platform} is ready.\n`);
  } catch (error) {
    console.error(`\nError: ${error.message}`);
    process.exit(1);
  }
}

function cleanup() {
  if (fs.existsSync(TEMP_DIR)) {
    console.log("Cleaning up temporary files...");
    fs.rmSync(TEMP_DIR, { recursive: true, force: true });
  }
}

function getCurrentPlatform() {
  const platform = process.platform;
  const arch = process.arch;

  if (platform === "win32" && arch === "x64") return "win-x64";
  if (platform === "darwin" && arch === "x64") return "darwin-x64";
  if (platform === "darwin" && arch === "arm64") return "darwin-arm64";
  if (platform === "linux" && arch === "x64") return "linux-x64";
  if (platform === "linux" && arch === "arm64") return "linux-arm64";

  console.error(`Unsupported platform: ${platform}-${arch}`);
  process.exit(1);
}

async function main() {
  const arg = process.argv[2];

  console.log("=".repeat(60));
  console.log("AGIME Embedded Node.js Downloader");
  console.log(`Node.js Version: ${NODE_VERSION}`);
  console.log("=".repeat(60));

  try {
    if (arg === "all") {
      // Download for all platforms
      for (const platform of Object.keys(PLATFORMS)) {
        await downloadPlatform(platform);
      }
    } else if (arg && PLATFORMS[arg]) {
      // Download for specific platform
      await downloadPlatform(arg);
    } else if (!arg) {
      // Download for current platform
      const platform = getCurrentPlatform();
      await downloadPlatform(platform);
    } else {
      console.error(`Unknown argument: ${arg}`);
      console.log("\nUsage:");
      console.log("  node download.js              # Current platform");
      console.log("  node download.js all          # All platforms");
      console.log("  node download.js <platform>   # Specific platform");
      console.log(`\nAvailable platforms: ${Object.keys(PLATFORMS).join(", ")}`);
      process.exit(1);
    }
  } finally {
    cleanup();
  }

  console.log("\n" + "=".repeat(60));
  console.log("Download complete!");
  console.log("=".repeat(60));
}

main().catch(console.error);
