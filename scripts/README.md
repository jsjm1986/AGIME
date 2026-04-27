# AGIME Scripts

This directory contains scripts for benchmarks, smoke tests, database helpers,
and Team Server end-to-end validation.

## Team Server E2E

### `team_server_e2e.py`

Server-side Runtime 6 and business-flow suite for `agime-team-server`.

It validates:

- health and authentication basics
- runtime-profile preview
- direct chat and heavy chat compaction progression
- task/executor flow and heavy task compaction progression
- channel flow
- public portal flow
- document upload/read flow
- provider request log behavior for `prompt_caching_mode=off`

Typical usage on the server:

```bash
python3 scripts/team_server_e2e.py --mode full
python3 scripts/team_server_e2e.py --mode live --json-out /tmp/agime-team-server-e2e-report.json
python3 scripts/team_server_e2e.py --mode live --portal-slug portal-871
```

Important defaults:

- base URL: `http://127.0.0.1:9999`
- Mongo URI: `mongodb://127.0.0.1:27017`
- team name: `agime`
- agent name: `GLM`

Password-login validation is supported, but will be reported as
`blocked by environment` unless `AGIME_E2E_PASSWORD_EMAIL` and
`AGIME_E2E_PASSWORD` are provided.

### `run_remote_team_server_e2e.py`

Local wrapper that SSHes into the live server and runs `team_server_e2e.py`
inside `/opt/agime`.

```bash
python scripts/run_remote_team_server_e2e.py --mode full
python scripts/run_remote_team_server_e2e.py --mode live --skip-frontend-build
python scripts/run_remote_team_server_e2e.py --mode live --portal-slug portal-871
```

Default SSH values can be overridden with:

- `AGIME_E2E_SSH_HOST`
- `AGIME_E2E_SSH_USER`
- `AGIME_E2E_SSH_PASSWORD`
- `AGIME_E2E_REMOTE_WORKDIR`

## Benchmark Scripts

### `run-benchmarks.sh`

This script runs AGIME benchmarks across multiple provider:model pairs and analyzes the results.

### Prerequisites

- AGIME CLI must be built or installed
- `jq` command-line tool for JSON processing (optional, but recommended for result analysis)

### Usage

```bash
./scripts/run-benchmarks.sh [options]
```

#### Options

- `-p, --provider-models`: Comma-separated list of provider:model pairs (e.g., 'openai:gpt-4o,anthropic:claude-sonnet-4')
- `-s, --suites`: Comma-separated list of benchmark suites to run (e.g., 'core,small_models')
- `-o, --output-dir`: Directory to store benchmark results (default: './benchmark-results')
- `-d, --debug`: Use debug build instead of release build
- `-h, --help`: Show help message

#### Examples

```bash
# Run with release build (default)
./scripts/run-benchmarks.sh --provider-models 'openai:gpt-4o,anthropic:claude-sonnet-4' --suites 'core,small_models'

# Run with debug build
./scripts/run-benchmarks.sh --provider-models 'openai:gpt-4o' --suites 'core' --debug
```

### How It Works

The script:
1. Parses the provider:model pairs and benchmark suites
2. Determines whether to use the debug or release binary
3. For each provider:model pair:
   - Sets the `AGIME_PROVIDER` and `AGIME_MODEL` environment variables
   - Runs the benchmark with the specified suites
   - Analyzes the results for failures
4. Generates a summary of all benchmark runs

### Output

The script creates the following files in the output directory:

- `summary.md`: A summary of all benchmark results
- `{provider}-{model}.json`: Raw JSON output from each benchmark run
- `{provider}-{model}-analysis.txt`: Analysis of each benchmark run

### Exit Codes

- `0`: All benchmarks completed successfully
- `1`: One or more benchmarks failed

### `parse-benchmark-results.sh`

This script analyzes a single benchmark JSON result file and identifies any failures.

### Usage

```bash
./scripts/parse-benchmark-results.sh path/to/benchmark-results.json
```

### Output

The script outputs an analysis of the benchmark results to stdout, including:

- Basic information about the benchmark run
- Results for each evaluation in each suite
- Summary of passed and failed metrics

### Exit Codes

- `0`: All metrics passed successfully
- `1`: One or more metrics failed
