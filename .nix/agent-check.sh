# shellcheck shell=bash
set -euo pipefail

./scripts/doctor.sh
docker compose config --quiet
