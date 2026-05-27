#!/usr/bin/env python3
"""Strip removed cfpm-memory endpoints/schemas from ui/desktop/openapi.json.

Earlier commits cleared the Rust handlers for the cfpm memory snapshot
surface but left the committed openapi.json in place. Drop the orphaned
paths and schema entries here so the openapi-schema CI gate passes.
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OPENAPI = ROOT / "ui" / "desktop" / "openapi.json"

# All paths whose handlers were removed
PATHS_TO_REMOVE = [
    "/sessions/{session_id}/memory/candidates",
    "/sessions/{session_id}/memory/path-rename",
    "/sessions/{session_id}/memory/rollback",
    "/sessions/{session_id}/memory/snapshots",
    "/sessions/{session_id}/memory/tool-gates",
    "/sessions/{session_id}/memory/facts",
    "/sessions/{session_id}/memory/facts/{fact_id}",
]

# Schemas only used by those removed paths
SCHEMAS_TO_REMOVE = [
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
