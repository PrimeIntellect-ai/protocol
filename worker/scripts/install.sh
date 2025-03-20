#!/usr/bin/env bash
set -e

# Colors for pretty output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
BINARY_NAME="prime-worker"
RELEASE_URL="https://github.com/PrimeIntellect-ai/protocol/releases"
BINARY_URL="$RELEASE_URL/latest/download/worker-linux-x86_64"
INSTALL_DIR="/usr/local/bin"

# Check if dev flag is set
if [[ "$1" == "--dev" ]]; then
  # Get latest dev tag
  LATEST_DEV_TAG=$(curl -s "$RELEASE_URL" | grep -o 'dev-[0-9]\{8\}-[a-z0-9]\+' | head -1)
  if [[ -z "$LATEST_DEV_TAG" ]]; then
    echo -e "${RED}✗ Could not find latest dev release${NC}"
    exit 1
  fi
  BINARY_URL="$RELEASE_URL/download/$LATEST_DEV_TAG/worker-linux-x86_64"
fi

# Print banner and download URL
echo -e "${BLUE}"
echo "╔═══════════════════════════════════════════╗"
echo "║                                           ║"
echo "║      Prime Intellect Protocol Worker      ║"
echo "║                                           ║"
echo "╚═══════════════════════════════════════════╝"

# Check operating system
echo -e "${BLUE}→ Checking system compatibility...${NC}"
if [[ "$(uname -s)" != "Linux" ]]; then
  echo -e "${RED}✗ This installer is for Linux only.${NC}"
  exit 1
fi

if [[ "$(uname -m)" != "x86_64" ]]; then
  echo -e "${RED}✗ This installer is for x86_64 architecture only.${NC}"
  exit 1
fi
echo -e "${GREEN}✓ System is compatible${NC}"

# Create temporary directory
TMP_DIR=$(mktemp -d)
trap 'rm -rf "$TMP_DIR"' EXIT

# Download binary
echo -e "${BLUE}→ Downloading Prime Intellect Protocol Worker...${NC}"
curl -sSL "$BINARY_URL" -o "$TMP_DIR/$BINARY_NAME"
chmod +x "$TMP_DIR/$BINARY_NAME"
echo -e "${GREEN}✓ Download complete${NC}"

# Install binary
echo -e "${BLUE}→ Installing to $INSTALL_DIR...${NC}"
if [[ -w "$INSTALL_DIR" ]]; then
  mv "$TMP_DIR/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
  echo -e "${GREEN}✓ Installation complete${NC}"
else
  echo -e "${RED}✗ Cannot write to $INSTALL_DIR${NC}"
  echo -e "${BLUE}→ Please run this script with sudo to install to $INSTALL_DIR${NC}"
  exit 1
fi

# Verify installation
echo -e "${BLUE}→ Verifying installation...${NC}"
if command -v "$BINARY_NAME" &> /dev/null; then
  echo -e "${GREEN}✓ Prime Intellect Protocol Worker successfully installed!${NC}"
  echo -e "${BLUE}→ Run '$BINARY_NAME --help' to get started${NC}"
else
  echo -e "${RED}✗ Installation verification failed.${NC}"
  echo -e "${BLUE}→ The binary is located at: $INSTALL_DIR/$BINARY_NAME${NC}"
fi
echo -e "\n${GREEN}The Prime Intellect Protocol Worker was successfully installed${NC}"
