#!/usr/bin/env bash
set -e

# Colors for pretty output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Configuration
BINARY_NAME="prime-worker"
RELEASE_URL="https://github.com/PrimeIntellect-ai/protocol/releases"
BINARY_URL="$RELEASE_URL/latest/download/worker-linux-x86_64"

# Determine install directory - try system dir first, fall back to user dir if needed
if [ "$EUID" -eq 0 ]; then
  # Running as root, use system directory
  INSTALL_DIR="/usr/local/bin"
else
  # Not running as root, use user directory (create if doesn't exist)
  INSTALL_DIR="$HOME/.local/bin"
  mkdir -p "$INSTALL_DIR"
fi

# Check if dev flag is set
if [[ "$1" == "--dev" ]]; then
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
echo -e "${NC}"

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
mv "$TMP_DIR/$BINARY_NAME" "$INSTALL_DIR/$BINARY_NAME"
echo -e "${GREEN}✓ Installation complete${NC}"

# Verify installation
echo -e "${BLUE}→ Verifying installation...${NC}"
if command -v "$BINARY_NAME" &> /dev/null || [ -x "$INSTALL_DIR/$BINARY_NAME" ]; then
  echo -e "${GREEN}✓ Prime Intellect Protocol Worker successfully installed!${NC}"
  echo -e "${BLUE}→ Binary location: $INSTALL_DIR/$BINARY_NAME${NC}"
  
  # Check if install dir is in PATH
  if [[ ":$PATH:" != *":$INSTALL_DIR:"* ]]; then
    echo -e "${YELLOW}⚠ $INSTALL_DIR is not in your PATH${NC}"
    echo -e "${BLUE}→ Run this command to add it to your PATH:${NC}"
    echo -e "${GREEN}   echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> ~/.bashrc && source ~/.bashrc${NC}"
    echo -e "${BLUE}→ Or run the binary directly: $INSTALL_DIR/$BINARY_NAME --help${NC}"
  else
    echo -e "${BLUE}→ Run '$BINARY_NAME --help' to get started${NC}"
  fi
else
  echo -e "${RED}✗ Installation verification failed.${NC}"
fi

echo -e "\n${GREEN}The Prime Intellect Protocol Worker was successfully installed${NC}"