#!/bin/bash

# Simple release script
# Usage: ./scripts/release.sh [patch|minor|major]

set -e

BUMP_TYPE=${1:-patch}

# Colors for output
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${YELLOW}Starting release process...${NC}"

# Ensure we're on main
CURRENT_BRANCH=$(git branch --show-current)
if [ "$CURRENT_BRANCH" != "main" ]; then
    echo "Error: You must be on main branch to release"
    exit 1
fi

# Pull latest
echo "Pulling latest changes..."
git pull origin main

# Get current version
CURRENT_VERSION=$(grep '^version =' Cargo.toml | head -n 1 | cut -d '"' -f 2)
echo "Current version: $CURRENT_VERSION"

# Calculate new version
IFS='.' read -r MAJOR MINOR PATCH <<< "$CURRENT_VERSION"

case "$BUMP_TYPE" in
    major)
        ((MAJOR++))
        MINOR=0
        PATCH=0
        ;;
    minor)
        ((MINOR++))
        PATCH=0
        ;;
    patch)
        ((PATCH++))
        ;;
    *)
        echo "Invalid bump type. Use: patch, minor, or major"
        exit 1
        ;;
esac

NEW_VERSION="${MAJOR}.${MINOR}.${PATCH}"
echo -e "${GREEN}New version: $NEW_VERSION${NC}"

# Update Cargo.toml file
echo "Updating Cargo.toml..."
sed -i.bak "s/version = \".*\"/version = \"${NEW_VERSION}\"/" ./Cargo.toml

# Show changes
echo -e "\n${YELLOW}Changes to be committed:${NC}"
git diff --name-only

# Confirm
echo -e "\n${YELLOW}Ready to release v${NEW_VERSION}. Continue? (y/n)${NC}"
read -r CONFIRM

if [ "$CONFIRM" != "y" ]; then
    echo "Release cancelled"
    git checkout -- .
    exit 0
fi

# Commit and tag
echo "Creating release commit..."
git add Cargo.toml Cargo.lock
git commit -m "chore: release v${NEW_VERSION}"

echo "Pushing to main..."
git push origin main

echo "Creating tag..."
git tag -a "v${NEW_VERSION}" -m "Release v${NEW_VERSION}"
git push origin "v${NEW_VERSION}"

echo -e "\n${GREEN}✅ Release v${NEW_VERSION} created successfully!${NC}"
echo -e "${GREEN}The production release workflow will now run automatically.${NC}"
echo -e "\nView progress at: https://github.com/PrimeIntellect-ai/protocol/actions"