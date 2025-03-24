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

# Print banner
echo -e "${BLUE}"
echo "╔═══════════════════════════════════════════╗"
echo "║                                           ║"
echo "║      Prime Intellect Protocol Worker      ║"
echo "║             Uninstallation                ║"
echo "╚═══════════════════════════════════════════╝"

# Check for binary
echo -e "${BLUE}→ Checking for installed binary...${NC}"
if [[ ! -f "$INSTALL_DIR/$BINARY_NAME" ]]; then
    echo -e "${RED}✗ Prime Worker binary not found in $INSTALL_DIR${NC}"
    exit 1
fi
echo -e "${GREEN}✓ Found installed binary${NC}"

# Remove binary
echo -e "${BLUE}→ Removing Prime Intellect Protocol Worker...${NC}"
if [[ -w "$INSTALL_DIR" ]]; then
    rm -f "$INSTALL_DIR/$BINARY_NAME"
    echo -e "${GREEN}✓ Binary removed${NC}"
else
    echo -e "${RED}✗ Cannot remove from $INSTALL_DIR${NC}"
    echo -e "${BLUE}→ Please run this script with sudo to remove from $INSTALL_DIR${NC}"
    exit 1
fi

echo -e "\n${GREEN}Prime Intellect Protocol Worker successfully uninstalled!${NC}"
