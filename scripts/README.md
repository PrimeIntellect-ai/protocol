chmod +x scripts/release.sh

./scripts/release.sh patch  # for 0.3.10 → 0.3.11
./scripts/release.sh minor  # for 0.3.11 → 0.4.0
./scripts/release.sh major  # for 0.4.0 → 1.0.0