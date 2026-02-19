"""Upload local skills from ~/.claude/skills/ to the team server.

Reads the entire skill directory (SKILL.md + all supporting files) and
uploads via the enhanced create API in package mode.
"""
import os
import re
import sys
import base64
import requests

SERVER = os.environ.get("AGIME_SERVER", "http://localhost:8080")
TEAM_ID = os.environ.get("AGIME_TEAM_ID", "698616a1980c003c66f6421e")
API_KEY = os.environ.get("AGIME_API_KEY", "")
SKILLS_DIR = os.path.expanduser("~/.claude/skills")

REQUEST_TIMEOUT = 30  # seconds

# Binary file extensions that should be base64-encoded
BINARY_EXTENSIONS = {
    ".png", ".jpg", ".jpeg", ".gif", ".bmp", ".ico", ".webp",
    ".zip", ".tar", ".gz", ".bz2", ".7z",
    ".pdf", ".doc", ".docx", ".xls", ".xlsx",
    ".woff", ".woff2", ".ttf", ".eot",
    ".exe", ".dll", ".so", ".dylib",
}


def is_binary_file(path):
    """Check if a file is binary based on extension."""
    _, ext = os.path.splitext(path)
    return ext.lower() in BINARY_EXTENSIONS


def read_file_content(path):
    """Read file content, returning text or base64-encoded string."""
    if is_binary_file(path):
        with open(path, "rb") as f:
            return "base64:" + base64.b64encode(f.read()).decode("ascii")
    try:
        with open(path, "r", encoding="utf-8") as f:
            return f.read()
    except UnicodeDecodeError:
        print(f"    WARN: {path} failed UTF-8 decode, using base64")
        with open(path, "rb") as f:
            return "base64:" + base64.b64encode(f.read()).decode("ascii")


def collect_skill_files(skill_dir):
    """Recursively read all files in skill directory (excluding SKILL.md)."""
    files = []
    for root, _dirs, filenames in os.walk(skill_dir):
        for fname in filenames:
            if fname == "SKILL.md":
                continue
            full_path = os.path.join(root, fname)
            rel_path = os.path.relpath(full_path, skill_dir).replace("\\", "/")
            content = read_file_content(full_path)
            files.append({"path": rel_path, "content": content})
    return files


def parse_skill_md(path):
    """Parse SKILL.md with YAML frontmatter, return (meta, body, raw_text)."""
    with open(path, "r", encoding="utf-8") as f:
        text = f.read()

    m = re.match(r"^---\s*\n(.*?)\n---\s*\n(.*)", text, re.DOTALL)
    if not m:
        return None, None, text

    frontmatter = m.group(1)
    body = m.group(2).strip()

    meta = {}
    for line in frontmatter.split("\n"):
        if ":" in line:
            key, val = line.split(":", 1)
            meta[key.strip()] = val.strip()

    return meta, body, text


def get_existing_skills(session):
    """Get dict of existing skill names -> ids on the server (all pages)."""
    all_skills = {}
    page = 1
    while True:
        resp = session.get(f"{SERVER}/api/team/skills", params={
            "teamId": TEAM_ID,
            "page": page,
            "limit": 200,
        }, timeout=REQUEST_TIMEOUT)
        if resp.status_code != 200:
            print(f"  Warning: Could not fetch existing skills: {resp.status_code}")
            return all_skills
        data = resp.json()
        items = data.get("items", data.get("skills", []))
        if not items:
            break
        for s in items:
            all_skills[s.get("name", "")] = s.get("id", "")
        total_pages = data.get("totalPages", 1)
        if page >= total_pages:
            break
        page += 1
    return all_skills


def delete_skill(session, skill_id, name):
    """Delete a skill by ID. Returns True on success."""
    resp = session.delete(
        f"{SERVER}/api/team/skills/{skill_id}",
        timeout=REQUEST_TIMEOUT,
    )
    if resp.status_code in (200, 204):
        print(f"  DEL  {name}")
        return True
    else:
        print(f"  DEL FAIL {name}: {resp.status_code} {resp.text[:200]}")
        return False


def upload_skill(session, skill_dir, dirname):
    """Upload a single skill directory to the team server."""
    skill_md_path = os.path.join(skill_dir, "SKILL.md")
    if not os.path.isfile(skill_md_path):
        return None, "no SKILL.md"

    meta, body, raw_text = parse_skill_md(skill_md_path)
    if meta is None:
        return None, "could not parse SKILL.md frontmatter"

    name = meta.get("name", "")
    if not name:
        return None, "no name in frontmatter"

    description = meta.get("description", "")
    files = collect_skill_files(skill_dir)

    payload = {
        "teamId": TEAM_ID,
        "name": name,
        "description": description,
        "content": body or "",
        "skillMd": raw_text,
        "files": files,
        "tags": ["builtin", "claude-code"],
        "visibility": "team",
    }

    resp = session.post(
        f"{SERVER}/api/team/skills",
        json=payload,
        timeout=REQUEST_TIMEOUT,
    )
    if resp.status_code in (200, 201):
        file_count = len(files)
        total_size = sum(len(f["content"]) for f in files)
        return name, f"OK ({file_count} files, ~{total_size // 1024}KB)"
    else:
        return name, f"FAIL {resp.status_code}: {resp.text[:200]}"


def main():
    if not API_KEY:
        print("Error: AGIME_API_KEY environment variable is required")
        sys.exit(1)

    if not os.path.isdir(SKILLS_DIR):
        print(f"Skills directory not found: {SKILLS_DIR}")
        sys.exit(1)

    # Login with API key
    session = requests.Session()
    login_resp = session.post(
        f"{SERVER}/api/auth/login",
        json={"api_key": API_KEY},
        timeout=REQUEST_TIMEOUT,
    )
    if login_resp.status_code != 200:
        print(f"Login failed: {login_resp.status_code} {login_resp.text}")
        sys.exit(1)
    print("Logged in successfully")

    # Get existing skills
    existing = get_existing_skills(session)
    print(f"Existing skills on server: {len(existing)}")

    # Check --replace flag
    replace_mode = "--replace" in sys.argv
    if replace_mode:
        print("Replace mode: will delete existing skills before re-uploading")

    # Discover local skill directories
    entries = sorted(os.listdir(SKILLS_DIR))
    skill_dirs = [d for d in entries if os.path.isdir(os.path.join(SKILLS_DIR, d))]
    print(f"\nFound {len(skill_dirs)} local skill directories")

    uploaded = 0
    skipped = 0
    failed = 0

    for dirname in skill_dirs:
        skill_dir = os.path.join(SKILLS_DIR, dirname)

        # Quick-check: does it have SKILL.md?
        if not os.path.isfile(os.path.join(skill_dir, "SKILL.md")):
            print(f"  SKIP {dirname}: no SKILL.md")
            skipped += 1
            continue

        # Parse name to check for duplicates
        meta, _, _ = parse_skill_md(os.path.join(skill_dir, "SKILL.md"))
        name = meta.get("name", "") if meta else ""

        if name in existing:
            if replace_mode:
                if not delete_skill(session, existing[name], name):
                    print(f"  SKIP {name}: delete failed, cannot replace")
                    failed += 1
                    continue
            else:
                print(f"  SKIP {name}: already exists (use --replace to overwrite)")
                skipped += 1
                continue

        result_name, msg = upload_skill(session, skill_dir, dirname)
        if result_name is None:
            print(f"  SKIP {dirname}: {msg}")
            skipped += 1
        elif msg.startswith("OK"):
            print(f"  OK   {result_name}: {msg}")
            uploaded += 1
        else:
            print(f"  FAIL {result_name}: {msg}")
            failed += 1

    print(f"\nDone: {uploaded} uploaded, {skipped} skipped, {failed} failed")


if __name__ == "__main__":
    main()
