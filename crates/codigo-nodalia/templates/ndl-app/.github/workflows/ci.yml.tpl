name: CI

on:
  push:
    branches: [main]
  pull_request:
  workflow_dispatch:

permissions:
  contents: read

concurrency:
  group: ci-${{ github.event_name == 'pull_request' && github.ref || github.run_id }}
  cancel-in-progress: ${{ github.event_name == 'pull_request' }}

jobs:
  gate:
    uses: getnodalia/foundation/.github/workflows/rust-gate.yml@{{foundation}} # foundation main
    with:
      command: >-
        cargo fmt --all -- --check &&
        cargo clippy --all-targets --locked -- -D warnings &&
        cargo test --locked
      packages: pkg-config libxkbcommon-dev libwayland-dev libgl1-mesa-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev libfontconfig1-dev libdbus-1-dev
      targets: ""
      tools: ""
