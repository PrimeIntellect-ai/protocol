.PHONY: install
install:
	@command -v uv > /dev/null || (echo "Please install uv first: curl -LsSf https://astral.sh/uv/install.sh | sh" && exit 1)
	@./setup.sh  # Uses uv for fast package management

.PHONY: build
build: install
	@uv cache clean
	@source .venv/bin/activate && maturin develop
	@source .venv/bin/activate && uv pip install --force-reinstall -e .

.PHONY: clean
clean:
	@rm -rf target/ dist/ *.egg-info .pytest_cache __pycache__ .venv/

.PHONY: help
help:
	@echo "Available commands:"
	@echo "  make install       - Setup environment and install dependencies"
	@echo "  make build         - Build development version (includes install and cache clear)"
	@echo "  make clean         - Clean build artifacts"