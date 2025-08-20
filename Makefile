BINARY_NAME = ivaldi
INSTALL_DIR = /usr/local/bin
SHARE_DIR = /usr/local/share/ivaldi
GO_CMD = go
GO_BUILD = $(GO_CMD) build
GO_CLEAN = $(GO_CMD) clean
GO_TEST = $(GO_CMD) test
GO_GET = $(GO_CMD) get
GO_MOD = $(GO_CMD) mod
MAIN_PATH = ./cmd/ivaldi
BUILD_DIR = build
VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo "dev")
LDFLAGS = -ldflags "-X main.Version=$(VERSION)"

.PHONY: all build clean install uninstall test deps tidy help

all: build

build:
	@echo "Building $(BINARY_NAME)..."
	@mkdir -p $(BUILD_DIR)
	$(GO_BUILD) $(LDFLAGS) -o $(BUILD_DIR)/$(BINARY_NAME) $(MAIN_PATH)
	@echo "Build complete: $(BUILD_DIR)/$(BINARY_NAME)"

install: build
	@echo "Installing $(BINARY_NAME) to $(INSTALL_DIR)..."
	@sudo mkdir -p $(INSTALL_DIR)
	@sudo cp $(BUILD_DIR)/$(BINARY_NAME) $(INSTALL_DIR)/$(BINARY_NAME)
	@sudo chmod 755 $(INSTALL_DIR)/$(BINARY_NAME)
	@echo "Installing prompt scripts to $(SHARE_DIR)..."
	@sudo mkdir -p $(SHARE_DIR)/scripts
	@sudo cp scripts/ivaldi-prompt.sh $(SHARE_DIR)/scripts/
	@sudo cp scripts/INSTALL_PROMPT.md $(SHARE_DIR)/scripts/
	@sudo cp scripts/example-bashrc.sh $(SHARE_DIR)/scripts/
	@sudo cp scripts/example-zshrc.sh $(SHARE_DIR)/scripts/
	@sudo cp -r scripts/oh-my-zsh-plugin $(SHARE_DIR)/scripts/
	@sudo chmod 755 $(SHARE_DIR)/scripts/ivaldi-prompt.sh
	@sudo chmod 755 $(SHARE_DIR)/scripts/oh-my-zsh-plugin/install.sh
	@sudo chmod 644 $(SHARE_DIR)/scripts/*.md $(SHARE_DIR)/scripts/example-*.sh
	@sudo chmod 644 $(SHARE_DIR)/scripts/oh-my-zsh-plugin/*.md $(SHARE_DIR)/scripts/oh-my-zsh-plugin/*.zsh*
	@echo "$(BINARY_NAME) installed successfully to $(INSTALL_DIR)"
	@echo "Prompt scripts installed to $(SHARE_DIR)/scripts"
	@echo "You can now run '$(BINARY_NAME)' from anywhere"
	@echo ""
	@echo "To add timeline info to your shell prompt:"
	@echo "  General instructions: $(SHARE_DIR)/scripts/INSTALL_PROMPT.md"
	@echo "  Oh My Zsh users: $(SHARE_DIR)/scripts/oh-my-zsh-plugin/README.md"
	@echo "  Quick Oh My Zsh install: $(SHARE_DIR)/scripts/oh-my-zsh-plugin/install.sh"

uninstall:
	@echo "Uninstalling $(BINARY_NAME) from $(INSTALL_DIR)..."
	@if [ -f $(INSTALL_DIR)/$(BINARY_NAME) ]; then \
		sudo rm -f $(INSTALL_DIR)/$(BINARY_NAME); \
		echo "$(BINARY_NAME) uninstalled successfully"; \
	else \
		echo "$(BINARY_NAME) not found in $(INSTALL_DIR)"; \
	fi
	@echo "Removing prompt scripts from $(SHARE_DIR)..."
	@if [ -d $(SHARE_DIR) ]; then \
		sudo rm -rf $(SHARE_DIR); \
		echo "Prompt scripts removed successfully"; \
	else \
		echo "Prompt scripts not found in $(SHARE_DIR)"; \
	fi

clean:
	@echo "Cleaning up build artifacts..."
	@rm -rf $(BUILD_DIR)
	@rm -f $(BINARY_NAME)
	@$(GO_CLEAN)
	@echo "Clean complete"

deep-clean: clean
	@echo "Performing deep clean..."
	@$(GO_CLEAN) -cache
	@$(GO_CLEAN) -testcache
	@$(GO_CLEAN) -modcache
	@echo "Deep clean complete"

test:
	@echo "Running tests..."
	$(GO_TEST) -v ./tests/...

deps:
	@echo "Downloading dependencies..."
	$(GO_GET) ./...

tidy:
	@echo "Tidying module dependencies..."
	$(GO_MOD) tidy

dev-install: build
	@echo "Installing $(BINARY_NAME) to user local bin (~/.local/bin)..."
	@mkdir -p ~/.local/bin
	@cp $(BUILD_DIR)/$(BINARY_NAME) ~/.local/bin/$(BINARY_NAME)
	@chmod 755 ~/.local/bin/$(BINARY_NAME)
	@echo "Installing prompt scripts to ~/.local/share/ivaldi..."
	@mkdir -p ~/.local/share/ivaldi/scripts
	@cp scripts/ivaldi-prompt.sh ~/.local/share/ivaldi/scripts/
	@cp scripts/INSTALL_PROMPT.md ~/.local/share/ivaldi/scripts/
	@cp scripts/example-bashrc.sh ~/.local/share/ivaldi/scripts/
	@cp scripts/example-zshrc.sh ~/.local/share/ivaldi/scripts/
	@cp -r scripts/oh-my-zsh-plugin ~/.local/share/ivaldi/scripts/
	@chmod 755 ~/.local/share/ivaldi/scripts/ivaldi-prompt.sh
	@chmod 755 ~/.local/share/ivaldi/scripts/oh-my-zsh-plugin/install.sh
	@chmod 644 ~/.local/share/ivaldi/scripts/*.md ~/.local/share/ivaldi/scripts/example-*.sh
	@chmod 644 ~/.local/share/ivaldi/scripts/oh-my-zsh-plugin/*.md ~/.local/share/ivaldi/scripts/oh-my-zsh-plugin/*.zsh*
	@echo "$(BINARY_NAME) installed to ~/.local/bin"
	@echo "Prompt scripts installed to ~/.local/share/ivaldi/scripts"
	@echo "Make sure ~/.local/bin is in your PATH"
	@echo ""
	@echo "To add timeline info to your shell prompt:"
	@echo "  General instructions: ~/.local/share/ivaldi/scripts/INSTALL_PROMPT.md"
	@echo "  Oh My Zsh users: ~/.local/share/ivaldi/scripts/oh-my-zsh-plugin/README.md"
	@echo "  Quick Oh My Zsh install: ~/.local/share/ivaldi/scripts/oh-my-zsh-plugin/install.sh"

dev-uninstall:
	@echo "Uninstalling $(BINARY_NAME) from user local bin..."
	@if [ -f ~/.local/bin/$(BINARY_NAME) ]; then \
		rm -f ~/.local/bin/$(BINARY_NAME); \
		echo "$(BINARY_NAME) uninstalled from ~/.local/bin"; \
	else \
		echo "$(BINARY_NAME) not found in ~/.local/bin"; \
	fi
	@echo "Removing prompt scripts from ~/.local/share/ivaldi..."
	@if [ -d ~/.local/share/ivaldi ]; then \
		rm -rf ~/.local/share/ivaldi; \
		echo "Prompt scripts removed successfully"; \
	else \
		echo "Prompt scripts not found in ~/.local/share/ivaldi"; \
	fi

help:
	@echo "Available targets:"
	@echo "  make build         - Build the binary"
	@echo "  make install       - Build and install to $(INSTALL_DIR) (requires sudo)"
	@echo "  make uninstall     - Remove from $(INSTALL_DIR) (requires sudo)"
	@echo "  make clean         - Remove build artifacts"
	@echo "  make deep-clean    - Remove all Go cache and build artifacts"
	@echo "  make test          - Run tests"
	@echo "  make deps          - Download dependencies"
	@echo "  make tidy          - Tidy module dependencies"
	@echo "  make dev-install   - Install to ~/.local/bin (no sudo required)"
	@echo "  make dev-uninstall - Remove from ~/.local/bin"
	@echo "  make help          - Show this help message"

.DEFAULT_GOAL := help