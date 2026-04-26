import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

const root = process.cwd();
const scanRoots = [
  join(root, "index.html"),
  join(root, "src"),
];

const fileExtensions = new Set([
  ".html",
  ".css",
  ".ts",
  ".tsx",
  ".js",
  ".jsx",
]);

const forbiddenPatterns = [
  {
    name: "css-import",
    regex: /@import\s+url\(\s*['"]https?:\/\//i,
  },
  {
    name: "html-script-src",
    regex: /<script[^>]+src=["']https?:\/\//i,
  },
  {
    name: "html-link-href",
    regex: /<link[^>]+href=["']https?:\/\//i,
  },
  {
    name: "css-url",
    regex: /url\(\s*['"]?https?:\/\//i,
  },
];

function walk(target, files = []) {
  const stats = statSync(target);
  if (stats.isDirectory()) {
    for (const entry of readdirSync(target)) {
      walk(join(target, entry), files);
    }
    return files;
  }
  files.push(target);
  return files;
}

function shouldScan(filePath) {
  for (const ext of fileExtensions) {
    if (filePath.endsWith(ext)) {
      return true;
    }
  }
  return false;
}

const violations = [];

for (const target of scanRoots) {
  for (const filePath of walk(target)) {
    if (!shouldScan(filePath)) continue;
    const content = readFileSync(filePath, "utf8");
    const lines = content.split(/\r?\n/);
    lines.forEach((line, index) => {
      for (const pattern of forbiddenPatterns) {
        if (pattern.regex.test(line)) {
          violations.push({
            file: relative(root, filePath),
            line: index + 1,
            text: line.trim(),
            type: pattern.name,
          });
        }
      }
    });
  }
}

if (violations.length > 0) {
  console.error("Found forbidden runtime external asset references:");
  for (const violation of violations) {
    console.error(
      `- ${violation.file}:${violation.line} [${violation.type}] ${violation.text}`,
    );
  }
  console.error(
    "Use local static assets, npm-managed dependencies, or self-hosted resources instead of browser-loaded external URLs.",
  );
  process.exit(1);
}

console.log("No forbidden runtime external asset references found.");
