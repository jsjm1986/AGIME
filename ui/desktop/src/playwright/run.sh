#!/bin/sh
# AGIME Playwright MCP Runner (Shell)
DIR="$(cd "$(dirname "$0")" && pwd)"
exec node "$DIR/run.js" "$@"
