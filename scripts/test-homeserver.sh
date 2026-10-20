#!/usr/bin/env bash
# Starts a throwaway local Synapse (pip, no Docker needed) on :8008 with
# accounts alice/bob/darkmage (password pw-<name>-123), then prints how to run
# the integration tests. Usage: scripts/test-homeserver.sh <workdir>
set -euo pipefail
W=${1:?workdir}
mkdir -p "$W/syn" && cd "$W"
[ -d venv ] || { python3 -m venv venv && ./venv/bin/pip install -q matrix-synapse; }
cd syn
if [ ! -f homeserver.yaml ]; then
  ../venv/bin/python -m synapse.app.homeserver --server-name localhost \
    --config-path homeserver.yaml --generate-config --report-stats=no
  cat >> homeserver.yaml <<'Y'
enable_registration: true
enable_registration_without_verification: true
suppress_key_server_warning: true
room_list_publication_rules:
  - {user_id: "*", alias: "*", room_id: "*", action: allow}
rc_message: {per_second: 1000, burst_count: 1000}
rc_registration: {per_second: 1000, burst_count: 1000}
rc_login:
  address: {per_second: 1000, burst_count: 1000}
  account: {per_second: 1000, burst_count: 1000}
  failed_attempts: {per_second: 1000, burst_count: 1000}
Y
fi
nohup ../venv/bin/python -m synapse.app.homeserver -c homeserver.yaml >synapse.log 2>&1 &
until curl -sf localhost:8008/_matrix/client/versions >/dev/null; do sleep 1; done
for u in alice bob darkmage; do
  ../venv/bin/register_new_matrix_user -u $u -p "pw-$u-123" --no-admin -c homeserver.yaml http://localhost:8008 || true
done
echo "Run: MSM_TEST_HOMESERVER=http://localhost:8008 cargo test"
