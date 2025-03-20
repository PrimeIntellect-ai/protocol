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
USER_INSTALL_DIR="$HOME/.local/bin"

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

# Determine install location
if [[ -w "$INSTALL_DIR" ]]; then
  # Can write to /usr/local/bin
  FINAL_INSTALL_DIR="$INSTALL_DIR"
else
  # Fall back to user directory
  FINAL_INSTALL_DIR="$USER_INSTALL_DIR"
  mkdir -p "$USER_INSTALL_DIR"
fi

# Install binary
echo -e "${BLUE}→ Installing to $FINAL_INSTALL_DIR...${NC}"
mv "$TMP_DIR/$BINARY_NAME" "$FINAL_INSTALL_DIR/$BINARY_NAME"
echo -e "${GREEN}✓ Installation complete${NC}"

# Check if install location is in PATH
if [[ ":$PATH:" != *":$FINAL_INSTALL_DIR:"* ]]; then
  echo -e "${BLUE}→ Adding $FINAL_INSTALL_DIR to your PATH...${NC}"
  
  # Determine shell config file
  SHELL_CONFIG=""
  if [[ -n "$BASH_VERSION" ]]; then
    SHELL_CONFIG="$HOME/.bashrc"
  elif [[ -n "$ZSH_VERSION" ]]; then
    SHELL_CONFIG="$HOME/.zshrc"
  fi
  
  if [[ -n "$SHELL_CONFIG" ]]; then
    echo "export PATH=\"\$PATH:$FINAL_INSTALL_DIR\"" >> "$SHELL_CONFIG"
    echo -e "${GREEN}✓ Added to PATH in $SHELL_CONFIG${NC}"
    echo -e "${BLUE}→ Run 'source $SHELL_CONFIG' to update your current session${NC}"
    # Source the shell config file immediately
    source "$SHELL_CONFIG"
  else
    echo -e "${BLUE}→ Please add $FINAL_INSTALL_DIR to your PATH manually${NC}"
  fi
fi

# Verify installation
echo -e "${BLUE}→ Verifying installation...${NC}"
if command -v "$BINARY_NAME" &> /dev/null; then
  echo -e "${GREEN}✓ Prime Intellect Protocol Worker successfully installed!${NC}"
  echo -e "${BLUE}→ Run '$BINARY_NAME --help' to get started${NC}"
else
  echo -e "${RED}✗ Installation verification failed. Please check your PATH.${NC}"
  echo -e "${BLUE}→ The binary is located at: $FINAL_INSTALL_DIR/$BINARY_NAME${NC}"
fi

echo -e "\n${GREEN}The Prime Intellect Protocol Worker was successfully installed${NC}"