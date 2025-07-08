# Contributing Guidelines

We welcome contributions via GitHub Pull Requests.

## Branching & Workflow

* **`main`:** The primary branch. All features, fixes, and releases go here.
* **Feature branches (`feature/*`, `fix/*`, etc.):** Short-lived branches for development work.

### 1. Development Workflow

1. **Create a feature branch from `main`:**
   ```bash
   git checkout main
   git pull origin main
   git checkout -b feature/your-feature-name
   ```

2. **Keep your branch updated (rebase strategy):**
   ```bash
   git fetch origin
   git rebase origin/main
   # Resolve any conflicts
   git push --force-with-lease origin feature/your-feature-name
   ```

3. **Make your changes, commit, and push:**
   ```bash
   git add .
   git commit -m "feat: add feature description"
   git push origin feature/your-feature-name
   ```

4. **Create a Pull Request to `main`:**
   - Target branch: `main`
   - Ensure your branch is rebased on latest `main`
   - Use **Squash and Merge** to keep history clean
   - Ensure all CI checks pass before merging

### 2. Release Process

**We follow a tag-based release process:**

1. **Prepare for release:**
   ```bash
   git checkout main
   git pull origin main
   
   # Update version in Cargo.toml
   # Update CHANGELOG.md with release notes
   git add Cargo.toml CHANGELOG.md
   git commit -m "chore: prepare release v0.1.8"
   git push origin main
   ```

2. **Create and push release tag:**
   ```bash
   # Tag MUST match the version in Cargo.toml
   git tag -a v0.1.8 -m "Release v0.1.8"
   git push origin v0.1.8
   ```

3. **Automated release process:**
   - Tag push triggers the production release pipeline
   - Creates GitHub Release with artifacts
   - Builds and pushes Docker images with `:latest` and `:v0.1.8` tags
   - Uploads binaries to Google Cloud Storage

### Release Types

- **Development releases:** Automatically created on every push to `main`
  - Version format: `v0.1.8-beta.1`, `v0.1.8-beta.2`, etc.
  - Docker tags: `:dev`, `:dev-latest`, `:v0.1.8-beta.1`
  - Used for testing and development environments

- **Production releases:** Created by pushing version tags
  - Version format: `v0.1.8` (semantic versioning)
  - Docker tags: `:latest`, `:stable`, `:v0.1.8`
  - Full GitHub Release with changelog and artifacts

## Rebase Strategy

We use a rebase workflow to maintain a clean, linear git history:

1. **Always rebase feature branches before merging:**
   ```bash
   git checkout feature/my-feature
   git fetch origin
   git rebase origin/main
   ```

2. **If you have conflicts during rebase:**
   ```bash
   # Fix conflicts in the affected files
   git add .
   git rebase --continue
   # Repeat until rebase is complete
   ```

3. **Force push with lease (safer than --force):**
   ```bash
   git push --force-with-lease origin feature/my-feature
   ```

**Important:** Never rebase branches that others are working on. Only rebase your own feature branches.

## Commit Messages

Use clear, descriptive commit messages following conventional commits:

* `feat:` New feature
* `fix:` Bug fix
* `docs:` Documentation changes
* `chore:` Maintenance tasks (version bumps, etc.)
* `test:` Test additions or changes
* `refactor:` Code refactoring

Examples:
* `feat: add validator health check endpoint`
* `fix: resolve memory leak in worker process`
* `chore: bump version to 0.1.8`

## Issues

* Report bugs via GitHub Issues
* Include:
  - Clear description of the problem
  - Steps to reproduce
  - Expected vs actual behavior
  - Environment details (OS, Rust version, etc.)
* Reference issues in commits: `fix: resolve login bug (#123)`

## Quick Reference

```bash
# Start new feature
git checkout main && git pull
git checkout -b feature/my-feature

# Keep branch updated with rebase
git fetch origin
git rebase origin/main
git push --force-with-lease origin feature/my-feature

# Create release
git checkout main && git pull
# Update Cargo.toml version and CHANGELOG.md
git add Cargo.toml CHANGELOG.md
git commit -m "chore: prepare release v0.1.8"
git push origin main
git tag -a v0.1.8 -m "Release v0.1.8"
git push origin v0.1.8
```

## Release Checklist

Before creating a release tag:
- [ ] Version bumped in `Cargo.toml`
- [ ] `CHANGELOG.md` updated with release notes
- [ ] All tests passing on `main`
- [ ] Documentation updated if needed
- [ ] Tag version matches Cargo.toml version exactly