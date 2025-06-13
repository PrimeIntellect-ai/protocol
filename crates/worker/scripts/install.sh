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
S3_BASE_URL="https://storage.googleapis.com/protocol-repo-assets/builds"
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
  echo -e "${BLUE}→ Using latest dev build from S3...${NC}"
  BINARY_URL="$S3_BASE_URL/latest/worker-linux-x86_64"
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
  
  # Check if user is in docker group
  if groups | grep -q '\bdocker\b'; then
    echo -e "${GREEN}✓ User is in the docker group - Docker access is available${NC}"
  else
    echo -e "${RED}⚠ User is not in the docker group${NC}"
    echo -e "${BLUE}→ To add your user to the docker group, run:${NC}"
    echo -e "${GREEN}   sudo usermod -aG docker $USER${NC}"
    echo -e "${BLUE}→ Then log out and log back in to apply the changes${NC}"
  fi
else
  echo -e "${RED}✗ Installation verification failed.${NC}"
fi

echo -e "\n${GREEN}The Prime Intellect Protocol Worker was successfully installed${NC}"