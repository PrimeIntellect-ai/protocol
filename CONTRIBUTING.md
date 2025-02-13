# Contributing Guidelines

We love your input! We want to make contributing to this project as easy and transparent as possible, whether it's:

- Reporting a bug
- Discussing the current state of the code
- Submitting a fix
- Proposing new features
- Becoming a maintainer

## We Develop with Github
We use GitHub to host code, to track issues and feature requests, as well as accept pull requests.

## Branch Strategy
We use a two-branch strategy for development:

1. `develop` - This is our main development branch where all feature branches are merged for nightly builds and testing
2. `main` - This is our stable production branch that contains reviewed and tested code

### Development Process

1. Create a new feature branch from `develop`
2. Make your changes and commit them
3. Submit a pull request to merge into `develop`
4. After review and testing in `develop`, changes will be merged into `main` for production releases

## Pull Request Process

1. Fork the repo and create your feature branch from `develop`
2. If you've added code that should be tested, add tests
3. If you've changed APIs, update the documentation
4. Ensure the test suite passes
5. Make sure your code lints
6. Submit a pull request to merge into `develop`

## Report bugs using Github's issue tracker
We use GitHub issues to track public bugs. Report a bug by opening a new issue; it's that easy!

## Write bug reports with detail, background, and sample code

**Great Bug Reports** tend to have:

- A quick summary and/or background
- Steps to reproduce
  - Be specific!
  - Give sample code if you can.
- What you expected would happen
- What actually happens
- Notes (possibly including why you think this might be happening, or stuff you tried that didn't work)

## Development Process

1. Create a new branch from `develop` for your work
2. Make your changes
3. Write or update tests as needed
4. Update documentation as needed
5. Submit a pull request to `develop`
6. Address any review feedback

### Commit Messages

- Use the present tense ("Add feature" not "Added feature")
- Use the imperative mood ("Move cursor to..." not "Moves cursor to...")
- Limit the first line to 72 characters or less
- Reference issues and pull requests liberally after the first line

## References
This document was adapted from the open-source contribution guidelines for [Facebook's Draft](https://github.com/facebook/draft-js/blob/a9316a723f9e918afde44dea68b5f9f39b7d9b00/CONTRIBUTING.md).
