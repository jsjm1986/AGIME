import json
import tarfile
from pathlib import Path

from bson import json_util
from pymongo import MongoClient


EXPORT_ROOT = Path(r"E:\yw\agiatme\goose\.codex-temp\mongo_export")
ARCHIVE_PATH = Path(r"E:\yw\agiatme\goose\.codex-temp\mongo_export.tar.gz")
DB_NAME = "agime_team"


def main() -> None:
    EXPORT_ROOT.mkdir(parents=True, exist_ok=True)
    for child in EXPORT_ROOT.iterdir():
        if child.is_file():
            child.unlink()
        else:
            for nested in child.rglob("*"):
                if nested.is_file():
                    nested.unlink()
            for nested in sorted(child.rglob("*"), reverse=True):
                if nested.is_dir():
                    nested.rmdir()
            child.rmdir()
    if ARCHIVE_PATH.exists():
        ARCHIVE_PATH.unlink()

    client = MongoClient("mongodb://127.0.0.1:27017", serverSelectionTimeoutMS=5000)
    client.admin.command("ping")
    db = client[DB_NAME]

    manifest: dict[str, dict[str, int]] = {}

    for name in sorted(db.list_collection_names()):
        coll = db[name]
        docs = list(coll.find())
        indexes = list(coll.list_indexes())

        with (EXPORT_ROOT / f"{name}.jsonl").open("w", encoding="utf-8") as f:
            for doc in docs:
                f.write(json_util.dumps(doc, ensure_ascii=False) + "\n")

        serialized_indexes = [json.loads(json_util.dumps(idx)) for idx in indexes]
        with (EXPORT_ROOT / f"{name}.indexes.json").open("w", encoding="utf-8") as f:
            json.dump(serialized_indexes, f, ensure_ascii=False, indent=2)

        manifest[name] = {"count": len(docs), "indexes": len(indexes)}

    with (EXPORT_ROOT / "manifest.json").open("w", encoding="utf-8") as f:
        json.dump(manifest, f, ensure_ascii=False, indent=2)

    with tarfile.open(ARCHIVE_PATH, "w:gz") as tar:
        tar.add(EXPORT_ROOT, arcname="mongo_export")

    print(
        json.dumps(
            {
                "db": DB_NAME,
                "collection_count": len(manifest),
                "archive": str(ARCHIVE_PATH),
            },
            ensure_ascii=False,
        )
    )


if __name__ == "__main__":
    main()
