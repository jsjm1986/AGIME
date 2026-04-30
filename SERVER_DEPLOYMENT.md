# AGIME Server Deployment Notes

This document records the current clean worktree deployment method for the
AGIME team server. Keep it short and update it when the server layout changes.

## Current Server Layout

- Source worktree: `/opt/agime-src`
- Build cache / release binary: `/opt/agime-build/target`
- Runtime service: `agime-team-server`
- Systemd unit: `/etc/systemd/system/agime-team-server.service`
- Runtime environment: `/etc/agime-team-server.env`
- Old deployment directory kept for rollback/reference: `/opt/agime`

The service should run from `/opt/agime-src`, not from `/opt/agime`.

Expected systemd shape:

```ini
WorkingDirectory=/opt/agime-src
EnvironmentFile=/etc/agime-team-server.env
ExecStart=/opt/agime-src/target/release/agime-team-server
```

`/opt/agime-src/target` is a symlink to `/opt/agime-build/target`.

## Deployment Principle

GitHub `main` is the source of truth for runnable code.

Do not manually patch random files under the server source directory unless it
is an emergency. Local code should be committed and pushed first, then deployed
to the server.

The current production server cannot reliably access GitHub. Treat bundle
deployment from the local workstation as the normal deployment path. Direct
server-side `git fetch origin main` is only an optional shortcut when GitHub
network access has been verified for that deployment.

Runtime data and secrets must stay outside Git:

- Keep secrets in `/etc/agime-team-server.env`.
- Keep MongoDB data in MongoDB.
- Keep uploads/workspace/artifacts in runtime storage, not in Git.
- Do not commit local test files, comparison repos, logs, or provider keys.

## Current Standard Deployment: Bundle

Use this for normal production deployment. GitHub `main` must still be pushed
first, but the server receives a local Git bundle instead of fetching from
GitHub.

On the local machine:

```bash
git push origin main
git rev-parse HEAD

tmp_dir="$TEMP/agime-clean-deploy"
rm -rf "$tmp_dir"
mkdir -p "$tmp_dir"
git clone --depth 1 "file:///path/to/local/goose" "$tmp_dir/shallow"
cd "$tmp_dir/shallow"
git bundle create "$tmp_dir/agime-head.bundle" HEAD
```

Upload `agime-head.bundle` to the server as `/tmp/agime-head.bundle`, then on
the server:

```bash
rm -rf /opt/agime-src.next
git clone /tmp/agime-head.bundle /opt/agime-src.next
cd /opt/agime-src.next
git switch -c main
git remote add origin https://github.com/jsjm1986/AGIME.git
git update-ref refs/remotes/origin/main HEAD
git config branch.main.remote origin
git config branch.main.merge refs/heads/main
git rev-parse HEAD > .git/shallow

mv /opt/agime-src /opt/agime-src.previous-$(date +%Y%m%d%H%M%S)
mv /opt/agime-src.next /opt/agime-src

ln -sfn /opt/agime-build/target /opt/agime-src/target
printf 'target\ncrates/agime-team-server/web-admin/node_modules\ncrates/agime-team-server/web-admin/dist\n' >> /opt/agime-src/.git/info/exclude
sort -u /opt/agime-src/.git/info/exclude -o /opt/agime-src/.git/info/exclude
```

Then build and restart:

```bash
cd /opt/agime-src

cd crates/agime-team-server/web-admin
CI=1 npm run build

cd /opt/agime-src
cargo build --release -p agime-team-server

systemctl restart agime-team-server
systemctl is-active agime-team-server
curl -fsS http://127.0.0.1:9999/
```

## Optional Direct Git Deployment

Use this only after confirming the server can reach GitHub during that
deployment window.

```bash
cd /opt/agime-src
git fetch origin main
git reset --hard origin/main
git status -sb
```

Then build and restart using the same commands from the current standard
deployment section.

## Verification Checklist

After deployment, confirm:

```bash
systemctl is-active agime-team-server
PID=$(systemctl show -p MainPID --value agime-team-server)
readlink -f /proc/$PID/cwd
readlink -f /proc/$PID/exe

cd /opt/agime-src
git status -sb
git rev-parse --short HEAD

curl -fsS http://127.0.0.1:9999/
journalctl -u agime-team-server --since '5 minutes ago' --no-pager \
  | grep -E ' ERROR |panic|fatal|thread .* panicked|stack overflow|failed to bind' \
  | tail -n 50
```

Expected:

- Service is `active`.
- Process cwd is `/opt/agime-src`.
- Process exe resolves under `/opt/agime-build/target`.
- Git status is clean.
- Health endpoint returns `Agime Team Server`.
- No severe startup errors.

## Rollback

If the new deployment fails:

1. Restore the previous systemd unit from `/etc/systemd/system/agime-team-server.service.bak-*`, or point it back to the previous known-good directory.
2. Run `systemctl daemon-reload`.
3. Run `systemctl restart agime-team-server`.
4. Verify health and logs.

Keep `/opt/agime` until the new `/opt/agime-src` layout has been stable for a
few days.
