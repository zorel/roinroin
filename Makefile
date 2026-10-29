PREFIX ?= $(HOME)/.local
DESTDIR ?=
BINDIR = $(DESTDIR)$(PREFIX)/bin
APPDIR = $(DESTDIR)$(PREFIX)/share/applications
ICONDIR = $(DESTDIR)$(PREFIX)/share/icons/hicolor/scalable/apps

.PHONY: all build run test check install uninstall clean

all: build

build:
	cargo build --release

run:
	cargo run --release

test:
	cargo test

check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings

install: build
	install -d "$(BINDIR)" "$(APPDIR)" "$(ICONDIR)"
	install -m 0755 target/release/roinroin "$(BINDIR)/roinroin"
	install -m 0644 assets/roinroin.desktop "$(APPDIR)/roinroin.desktop"
	install -m 0644 assets/roinroin.svg "$(ICONDIR)/roinroin.svg"
	@if command -v update-desktop-database >/dev/null 2>&1; then \
		update-desktop-database "$(DESTDIR)$(PREFIX)/share/applications" || true; \
	fi
	@if command -v gtk-update-icon-cache >/dev/null 2>&1; then \
		gtk-update-icon-cache -f -t "$(DESTDIR)$(PREFIX)/share/icons/hicolor" || true; \
	fi

uninstall:
	rm -f "$(BINDIR)/roinroin" "$(APPDIR)/roinroin.desktop" "$(ICONDIR)/roinroin.svg"

clean:
	cargo clean
