import paramiko


HOST = "117.72.54.28"
USER = "root"
PASSWORD = "Jsjm4082880"


SCRIPT = r"""
set -e
systemctl stop agime-team-server || true
rm -rf /root/mongo_export_restore
mkdir -p /root/mongo_export_restore
tar -xzf /root/mongo_export.tar.gz -C /root/mongo_export_restore
python3 - <<'PY'
import json
from pathlib import Path
from pymongo import MongoClient
from bson import json_util

root = Path('/root/mongo_export_restore/mongo_export')
manifest = json.loads((root / 'manifest.json').read_text(encoding='utf-8'))
client = MongoClient('mongodb://127.0.0.1:27017', serverSelectionTimeoutMS=5000)
client.admin.command('ping')
client.drop_database('agime_team')
db = client['agime_team']

for coll_name in sorted(manifest.keys()):
    coll = db[coll_name]
    jsonl_path = root / f'{coll_name}.jsonl'
    docs = []
    with jsonl_path.open('r', encoding='utf-8') as f:
        for line in f:
            line = line.strip()
            if line:
                docs.append(json_util.loads(line))
    if docs:
        coll.insert_many(docs, ordered=True)
    else:
        db.create_collection(coll_name)

    indexes_path = root / f'{coll_name}.indexes.json'
    indexes = json.loads(indexes_path.read_text(encoding='utf-8'))
    for idx in indexes:
        index_name = idx.get('name')
        if index_name == '_id_':
            continue
        key = idx.get('key', {})
        key_items = list(key.items())
        kwargs = {}
        for opt in [
            'unique', 'sparse', 'expireAfterSeconds', 'partialFilterExpression',
            'collation', 'weights', 'default_language', 'language_override',
            'textIndexVersion', '2dsphereIndexVersion', 'bits', 'min', 'max',
            'bucketSize', 'wildcardProjection', 'hidden'
        ]:
            if opt in idx:
                kwargs[opt] = idx[opt]
        if index_name:
            kwargs['name'] = index_name
        coll.create_index(key_items, **kwargs)

summary = {
    'teams': db['teams'].count_documents({}),
    'team_agents': db['team_agents'].count_documents({}),
    'portals': db['portals'].count_documents({}),
    'documents': db['documents'].count_documents({}),
    'external_users': db['external_users'].count_documents({}),
    'skills': db['skills'].count_documents({}),
    'extensions': db['extensions'].count_documents({}),
}
print(summary)
client.close()
PY
systemctl start agime-team-server
sleep 4
curl -fsS http://127.0.0.1:9999/health
"""


def main() -> None:
    client = paramiko.SSHClient()
    client.set_missing_host_key_policy(paramiko.AutoAddPolicy())
    client.connect(HOST, username=USER, password=PASSWORD, timeout=20)
    stdin, stdout, stderr = client.exec_command("bash -s", timeout=5400)
    stdin.write(SCRIPT)
    stdin.channel.shutdown_write()
    out = stdout.read().decode("utf-8", "ignore")
    err = stderr.read().decode("utf-8", "ignore")
    print(out)
    if err:
        print("STDERR:")
        print(err)
    print(f"EXIT={stdout.channel.recv_exit_status()}")
    client.close()


if __name__ == "__main__":
    main()
