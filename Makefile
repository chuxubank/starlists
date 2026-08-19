BIN := stars
PREFIX ?= $(HOME)/.local
DESTDIR := $(PREFIX)/bin

.PHONY: build test install-local fmt

build:
	cargo build --release

test:
	cargo test

fmt:
	cargo fmt

install-local: build
	mkdir -p $(DESTDIR)
	install -m 0755 target/release/$(BIN) $(DESTDIR)/$(BIN)
	@echo "installed $(DESTDIR)/$(BIN)"
