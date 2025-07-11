.PHONY: install
install:
	@command -v uv > /dev/null || (echo "Please install uv first: curl -LsSf https://astral.sh/uv/install.sh | sh" && exit 1)
	@./setup.sh  # Uses uv for fast package management

.PHONY: build
build:
	@source .venv/bin/activate && maturin develop
	@source .venv/bin/activate && uv pip install --force-reinstall -e .

.PHONY: dev
dev:
	@source .venv/bin/activate && maturin develop --watch

.PHONY: build-release
build-release:
	@source .venv/bin/activate && maturin build --release --strip

.PHONY: test
test:
	@source .venv/bin/activate && pytest tests/ -v

.PHONY: example
example:
	@source .venv/bin/activate && python examples/basic_usage.py

.PHONY: clean
clean:
	@rm -rf target/ dist/ *.egg-info .pytest_cache __pycache__ .venv/

.PHONY: clear-cache
clear-cache:
	@uv cache clean
	@echo "uv cache cleared"

.PHONY: help
help:
	@echo "Available commands:"
	@echo "  make install       - Setup environment and install dependencies"
	@echo "  make build         - Build development version"
	@echo "  make dev           - Build with hot reload (watches for changes)"
	@echo "  make build-release - Build release wheel"
	@echo "  make test          - Run tests"
	@echo "  make example       - Run example script"
	@echo "  make clean         - Clean build artifacts"
	@echo "  make clear-cache   - Clear uv cache" 