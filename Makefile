PREFIX ?= /usr/local
BINDIR = $(PREFIX)/bin
BINARY = ivaldi
TARGET = target/release/$(BINARY)

.PHONY: build test install install-extras uninstall clean help

## Build release binary
build:
	cargo build --release

## Run all tests
test:
	cargo test

## Install ivaldi to $(PREFIX)/bin (default: /usr/local/bin)
install: build
	@echo "Installing $(BINARY) to $(BINDIR)..."
	@install -d $(BINDIR)
	@install -m 755 $(TARGET) $(BINDIR)/$(BINARY)
	@echo "Installed $(BINARY) to $(BINDIR)/$(BINARY)"
	@echo "Run 'ivaldi --version' to verify."

## Install man pages and shell completions (bash/zsh/fish)
install-extras: build
	@echo "Installing man pages to $(PREFIX)/share/man/man1..."
	@install -d $(PREFIX)/share/man/man1
	@$(TARGET) man --out $(PREFIX)/share/man/man1
	@echo "Installing shell completions..."
	@install -d $(PREFIX)/share/bash-completion/completions
	@$(TARGET) completions bash > $(PREFIX)/share/bash-completion/completions/ivaldi
	@install -d $(PREFIX)/share/zsh/site-functions
	@$(TARGET) completions zsh > $(PREFIX)/share/zsh/site-functions/_ivaldi
	@install -d $(PREFIX)/share/fish/vendor_completions.d
	@$(TARGET) completions fish > $(PREFIX)/share/fish/vendor_completions.d/ivaldi.fish
	@echo "Installed man pages and completions under $(PREFIX)/share"

## Uninstall ivaldi from $(PREFIX)/bin
uninstall:
	@echo "Removing $(BINDIR)/$(BINARY)..."
	@rm -f $(BINDIR)/$(BINARY)
	@echo "Uninstalled $(BINARY)"

## Clean build artifacts
clean:
	cargo clean

## Show help
help:
	@echo "Ivaldi VCS — Makefile targets:"
	@echo ""
	@echo "  make build      Build release binary"
	@echo "  make test       Run all tests"
	@echo "  make install    Install to $(BINDIR) (may need sudo)"
	@echo "  make install-extras  Install man pages and bash/zsh/fish completions (may need sudo)"
	@echo "  make uninstall  Remove from $(BINDIR) (may need sudo)"
	@echo "  make clean      Clean build artifacts"
	@echo ""
	@echo "Override install location:"
	@echo "  make install PREFIX=~/.local"
