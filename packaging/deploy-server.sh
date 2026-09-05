#!/usr/bin/env bash
#
# Build planner-server, prove it starts, and push it to the NAS registry.
#
#     PLANNER_REGISTRY=nas.example.ts.net:5050 ./packaging/deploy-server.sh
#
# Tests first, then build, then a smoke test of the actual image, and only
# then a push. A registry is a place other machines pull from; getting a
# broken tag out of one is more work than not putting it there.
#
# The tag is today's date. `:latest` means a restart can quietly change what is
# holding the task list, which is the wrong kind of surprise.

set -euo pipefail
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

REGISTRY="${PLANNER_REGISTRY:-nas.example.ts.net:5050}"

# The date says when, the commit says what. A date alone cannot answer "which
# commit is running on the NAS", which is the question actually asked when
# something is behaving oddly — and two builds in one day made it unanswerable
# rather than merely awkward.
TAG="${PLANNER_TAG:-$(date +%Y-%m-%d)-$(git rev-parse --short HEAD)}"
IMAGE="$REGISTRY/planner-server:$TAG"

# A tag naming a commit has to mean it. Untracked files are fine — CLAUDE.md
# and a local .env live beside this — but a tracked change that is not in the
# commit would make the tag a lie.
if ! git diff-index --quiet HEAD --; then
    echo "the working tree has uncommitted changes, so $TAG would not describe" >&2
    echo "what is in the image. Commit them, or set PLANNER_TAG to say so." >&2
    exit 1
fi

echo "==> ./test.sh"
./test.sh

echo "==> podman build $IMAGE"
# --format docker, not podman's OCI default: HEALTHCHECK has no place in the
# OCI image spec, so an OCI build drops it with a warning that is easy to miss.
# The compose file declares one too, but an image that cannot say whether it is
# well is worth avoiding on its own.
podman build --format docker -f server/Containerfile -t "$IMAGE" .

# Start it with a deliberately broken database so it gets far enough to prove
# the binary runs, reads its configuration and refuses what it should — without
# needing the real credentials in a build script.
echo "==> smoke test"
# Captured before it is searched, not piped into grep. Refusing is the correct
# behaviour *and* a non-zero exit, and under `pipefail` that non-zero would
# fail the pipeline — so piping made a passing smoke test look like a failing
# one, which is the worst direction for a check to be wrong in.
refusal="$(podman run --rm -e PLANNER_TOKEN=short "$IMAGE" 2>&1 || true)"
if grep -q "at least" <<<"$refusal"; then
    echo "    refuses a short token"
else
    echo "    the image did not refuse a short token, it said:" >&2
    echo "$refusal" >&2
    exit 1
fi

echo "==> podman push $IMAGE"
# --tls-verify=false because the registry speaks plain HTTP. It is reachable
# only over the tailnet, which is what makes that acceptable.
podman push --tls-verify=false "$IMAGE"

cat <<EOF

Pushed $IMAGE

Next, on the NAS:
  1. Container Manager → Project → planner-server
  2. Set PLANNER_SERVER_IMAGE in its .env to:

       localhost:5050/planner-server:$TAG

     localhost, not the tailnet name — a registry stores repositories by name
     rather than by hostname, so it is the same image, and Docker accepts a
     registry reached over localhost on plain HTTP without being configured
     to. Naming the NAS means editing the daemon config over SSH for no gain.
  3. Build.

Then check it answers rather than trusting the status dot:
  curl -s http://nas.example.ts.net:8083/health
EOF
