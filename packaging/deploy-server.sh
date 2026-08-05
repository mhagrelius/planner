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
TAG="${PLANNER_TAG:-$(date +%Y-%m-%d)}"
IMAGE="$REGISTRY/planner-server:$TAG"

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
if podman run --rm -e PLANNER_TOKEN=short "$IMAGE" 2>&1 | grep -q "at least"; then
    echo "    refuses a short token"
else
    echo "    the image did not refuse a short token" >&2
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
