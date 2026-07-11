SHELL := /bin/bash
.DEFAULT_GOAL := build

PRODUCT_NAME := JustTalk
BUNDLE_ID := com.justtalk.slim
VERSION := $(shell node -p "require('./package.json').version")
MACHINE := $(shell uname -m)
ARCH := $(if $(filter arm64,$(MACHINE)),aarch64,$(MACHINE))

TAURI_DIR := src-tauri
RELEASE_DIR := $(TAURI_DIR)/target/release
BUNDLE_DIR := $(RELEASE_DIR)/bundle
APP := $(BUNDLE_DIR)/macos/$(PRODUCT_NAME).app
DMG := $(BUNDLE_DIR)/dmg/JustTalk_$(VERSION)_$(ARCH).dmg
STAGING := $(RELEASE_DIR)/dmg-staging
MAC_ICON_SOURCE := $(TAURI_DIR)/icons/icon.png
MAC_ICON := $(TAURI_DIR)/icons/icon.icns
TRAY_ICON_SOURCE := $(TAURI_DIR)/icons/tray-template.svg
TRAY_ICON := $(TAURI_DIR)/icons/tray-template.png

.PHONY: install dev frontend check-overlay check test mac-icon tray-icon app dmg build verify clean paths

install:
	npm install

dev: install
	npm run tauri dev

frontend: install
	npm run build

check-overlay: frontend
	test -f dist/overlay.html
	test -n "$$(find dist/assets -maxdepth 1 -name 'overlay-*.js' -print -quit)"
	rg -q 'core:window:allow-show' src-tauri/capabilities/default.json
	rg -q 'core:window:allow-set-position' src-tauri/capabilities/default.json
	rg -q '"macOSPrivateApi": true' src-tauri/tauri.conf.json
	rg -q 'width: 188px; height: 52px' src/overlay.css
	rg -q 'audio-level' src/overlay.ts src-tauri/src/session.rs

check: check-overlay
	cargo check --manifest-path $(TAURI_DIR)/Cargo.toml
	cargo clippy --manifest-path $(TAURI_DIR)/Cargo.toml --all-targets -- -D warnings

test: check-overlay
	cargo test --manifest-path $(TAURI_DIR)/Cargo.toml --lib

mac-icon:
	@set -e; work="$$(mktemp -d)"; trap 'rm -rf "$$work"' EXIT; \
	cp "$(MAC_ICON_SOURCE)" "$$work/master.png"; \
	sips --resampleHeightWidth 1640 1640 "$$work/master.png" >/dev/null; \
	sips --padToHeightWidth 2048 2048 "$$work/master.png" >/dev/null; \
	mkdir -p "$$work/JustTalk.iconset"; \
	for spec in '16 icon_16x16.png' '32 icon_16x16@2x.png' '32 icon_32x32.png' '64 icon_32x32@2x.png' '128 icon_128x128.png' '256 icon_128x128@2x.png' '256 icon_256x256.png' '512 icon_256x256@2x.png' '512 icon_512x512.png' '1024 icon_512x512@2x.png'; do \
		set -- $$spec; sips --resampleHeightWidth $$1 $$1 "$$work/master.png" --out "$$work/JustTalk.iconset/$$2" >/dev/null; \
	done; \
	iconutil -c icns "$$work/JustTalk.iconset" -o "$(MAC_ICON)"

tray-icon:
	sips --resampleHeightWidth 44 44 -s format png "$(TRAY_ICON_SOURCE)" --out "$(TRAY_ICON)" >/dev/null

app: install
	$(MAKE) mac-icon
	$(MAKE) tray-icon
	npm run tauri build -- --bundles app
	codesign --force --deep --sign - --identifier $(BUNDLE_ID) --requirements '=designated => identifier "$(BUNDLE_ID)"' "$(APP)"
	codesign --verify --deep --strict --verbose=2 "$(APP)"

dmg: app
	rm -rf "$(STAGING)"
	mkdir -p "$(STAGING)" "$(BUNDLE_DIR)/dmg"
	cp -R "$(APP)" "$(STAGING)/"
	ln -s /Applications "$(STAGING)/Applications"
	hdiutil create -volname "$(PRODUCT_NAME)" -srcfolder "$(STAGING)" -ov -format UDZO "$(DMG)"
	rm -rf "$(STAGING)"

build: dmg
	$(MAKE) verify
	$(MAKE) paths

verify:
	test -d "$(APP)"
	test -f "$(DMG)"
	codesign --verify --deep --strict --verbose=2 "$(APP)"
	hdiutil verify "$(DMG)"
	shasum -a 256 "$(DMG)"

paths:
	@echo "APP: $(APP)"
	@echo "DMG: $(DMG)"

clean:
	rm -rf dist node_modules "$(STAGING)" "$(TAURI_DIR)/target"
