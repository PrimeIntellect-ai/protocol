#!/bin/bash
set -e

# Check if uv is installed
if ! command -v uv &> /dev/null; then
    echo "Please install uv first: curl -LsSf https://astral.sh/uv/install.sh | sh"
    exit 1
fi

# Setup environment
uv venv
source .venv/bin/activate
uv pip install -r requirements-dev.txt
maturin develop

echo "Setup complete." 