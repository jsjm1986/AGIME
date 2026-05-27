#!/usr/bin/env python3
"""Strip orphaned paths/schemas from ui/desktop/openapi.json.

When Rust route handlers and their utoipa registrations are removed, the
committed openapi.json snapshot lags behind. CI's openapi-schema gate then
fails. We can't `cargo run --bin generate_schema` locally on Windows
(aws-lc-rs build script needs Unix tooling), so this script surgically
prunes endpoints/schemas that are no longer registered in
crates/agime-server/src/openapi.rs.

Usage:
    python3 scripts/strip_orphan_openapi.py
    cd ui/desktop && npm run generate-api
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OPENAPI = ROOT / "ui" / "desktop" / "openapi.json"

PATHS_TO_REMOVE = [
    # cfpm-memory snapshot surface (handlers removed earlier)
    "/sessions/{session_id}/memory/candidates",
    "/sessions/{session_id}/memory/path-rename",
    "/sessions/{session_id}/memory/rollback",
    "/sessions/{session_id}/memory/snapshots",
    "/sessions/{session_id}/memory/tool-gates",
    "/sessions/{session_id}/memory/facts",
    "/sessions/{session_id}/memory/facts/{fact_id}",
    # schedule routes (entire surface removed from agime-server)
    "/recipes/schedule",
    "/schedule/create",
    "/schedule/delete/{id}",
    "/schedule/list",
    "/schedule/{id}",
    "/schedule/{id}/inspect",
    "/schedule/{id}/kill",
    "/schedule/{id}/pause",
    "/schedule/{id}/run_now",
    "/schedule/{id}/sessions",
    "/schedule/{id}/unpause",
]

SCHEMAS_TO_REMOVE = [
    # cfpm memory schemas
    "CfpmToolGateEventRecord",
    "ListMemoryCandidatesQuery",
    "ListMemoryToolGatesQuery",
    "MemoryCandidate",
    "MemorySnapshotRecord",
    "RenameMemoryPathRequest",
    "RenameMemoryPathResponse",
    "RollbackMemorySnapshotRequest",
    "RollbackMemorySnapshotResponse",
    "CreateMemoryFactRequest",
    "MemoryFact",
    "MemoryFactPatch",
    "MemoryFactStatus",
    # schedule schemas (only referenced by removed schedule paths)
    "CreateScheduleRequest",
    "InspectJobResponse",
    "KillJobResponse",
    "ListSchedulesResponse",
    "RunNowResponse",
    "ScheduleRecipeRequest",
    "ScheduledJob",
    "SessionDisplayInfo",
    "SessionsQuery",
    "UpdateScheduleRequest",
]


def main() -> int:
    data = json.loads(OPENAPI.read_text(encoding="utf-8"))

    paths = data.get("paths", {})
    for p in PATHS_TO_REMOVE:
        if p in paths:
            print(f"removed path: {p}")
            del paths[p]

    schemas = data.get("components", {}).get("schemas", {})
    for s in SCHEMAS_TO_REMOVE:
        if s in schemas:
            print(f"removed schema: {s}")
            del schemas[s]

    OPENAPI.write_text(
        json.dumps(data, indent=2, ensure_ascii=False) + "\n",
        encoding="utf-8",
    )
    print(f"wrote {OPENAPI}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
