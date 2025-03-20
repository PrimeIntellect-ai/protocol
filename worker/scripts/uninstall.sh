#!/usr/bin/env bash
set -e

# Colors for pretty output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
BINARY_NAME="prime-worker"
INSTALL_DIR="/usr/local/bin"
USER_INSTALL_DIR="$HOME/.local/bin"

# Print banner
echo -e "${BLUE}"
echo "╔═══════════════════════════════════════════╗"
echo "║                                           ║"
echo "║  Prime Intellect Protocol Worker Remove   ║"
echo "║                                           ║"
echo "╚═══════════════════════════════════════════╝"

# Check for binary in standard locations
BINARY_PATH=""
if [[ -f "$INSTALL_DIR/$BINARY_NAME" ]]; then
    BINARY_PATH="$INSTALL_DIR/$BINARY_NAME"
elif [[ -f "$USER_INSTALL_DIR/$BINARY_NAME" ]]; then
    BINARY_PATH="$USER_INSTALL_DIR/$BINARY_NAME"
fi

if [[ -z "$BINARY_PATH" ]]; then
    echo -e "${RED}✗ Prime Worker binary not found in standard locations${NC}"
    exit 1
fi

# Remove binary
echo -e "${BLUE}→ Removing Prime Intellect Protocol Worker...${NC}"
rm -f "$BINARY_PATH"
echo -e "${GREEN}✓ Binary removed${NC}"

# Clean up PATH in shell config files
if [[ -f "$HOME/.bashrc" ]]; then
    sed -i "s|export PATH=\"\$PATH:$USER_INSTALL_DIR\"||g" "$HOME/.bashrc"
fi
if [[ -f "$HOME/.zshrc" ]]; then
    sed -i "s|export PATH=\"\$PATH:$USER_INSTALL_DIR\"||g" "$HOME/.zshrc"
fi

echo -e "${GREEN}✓ Prime Intellect Protocol Worker successfully uninstalled${NC}"
