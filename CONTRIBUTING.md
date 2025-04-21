# Contributing Guidelines

We welcome contributions via GitHub Pull Requests.

## Branching & Workflow

*   **`main`:** Stable releases ONLY. Merged from `release/*`. **NEVER commit directly.**
*   **`develop`:** Integration branch for the next release. Source for features/releases.

**1. Features (`feature/*`)**
    *   Branch from `develop`.
    *   PR targets `develop`.
    *   Use **Squash and Merge** when merging the PR.

**2. Releases (`release/vX.Y.Z`)**
    *   Branch from `develop`. Add only version bumps & critical final fixes.
    *   **PR 1:** `release/*` -> `main`. Use **Merge Commit (NO Squash)**. 
    *   **PR 2:** `release/*` -> `develop`. Use **Merge Commit (NO Squash)**.

**Rule:** **NEVER** merge `main` back into `develop`.

## Pull Requests (Features to `develop`)

*   Base your feature branch on the latest `develop`.
*   Include tests and documentation updates.
*   Ensure tests and linting pass.
*   Submit PR targeting `develop`; address feedback.

## Issues & Commits

*   Report bugs via GitHub Issues with details.
*   Use clear, present-tense commit messages (e.g., "Fix login bug #123").