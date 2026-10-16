# SSH-App (iOS + Rust Core) Makefile
SHELL := /bin/zsh

SCHEME := SSHApp
PROJECT := ios/SSHApp.xcodeproj
BUNDLE_ID := com.semenov.sshapp
CONFIGURATION ?= Release
DERIVED_DATA := $(CURDIR)/.build/DerivedData
APP_PATH := $(DERIVED_DATA)/Build/Products/$(CONFIGURATION)-iphoneos/$(SCHEME).app

# Auto-detect the first connected physical iOS device (arm64) if DEVICE_ID is not passed
DEVICE_ID ?= $(shell xcrun devicectl list devices 2>/dev/null | grep -E "iPhone|00008130" | head -n 1 | grep -oE "[0-9A-F]{8}-[0-9A-F]{16}")

.PHONY: help iphone iphone-full xcframework ios-build ios-install ios-launch test lint clean

help:
	@echo "Доступные команды:"
	@echo "  make iphone        — Собрать iOS-приложение, установить и запустить на подключенном iPhone"
	@echo "  make iphone-full   — Пересобрать Rust XCFramework + собрать, установить и запустить на iPhone"
	@echo "  make xcframework   — Собрать Rust Core XCFramework и обновить UniFFI Swift bindings"
	@echo "  make ios-build     — Только скомпилировать и подписать .app для подключенного iPhone"
	@echo "  make ios-install   — Установить собранный .app на подключенный iPhone"
	@echo "  make ios-launch    — Запустить установленное приложение на подключенном iPhone"
	@echo "  make test          — Запустить тесты Rust Core и Swift Bridge"
	@echo "  make lint          — Проверить форматирование и Clippy в Rust Core"
	@echo "  make clean         — Очистить DerivedData"

xcframework:
	./scripts/build-xcframework.sh

ios-build:
	@echo "==> Сборка и подпись $(SCHEME) ($(CONFIGURATION)) для iOS..."
	xcodebuild \
		-project $(PROJECT) \
		-scheme $(SCHEME) \
		-configuration $(CONFIGURATION) \
		-destination "generic/platform=iOS" \
		-derivedDataPath $(DERIVED_DATA) \
		-skipPackagePluginValidation \
		-allowProvisioningUpdates \
		build

ios-install:
	@if [ -z "$(DEVICE_ID)" ]; then \
		echo "Ошибка: подключенный iPhone не найден."; \
		exit 1; \
	fi
	@echo "==> Установка $(APP_PATH) на iPhone ($(DEVICE_ID))..."
	xcrun devicectl device install app --device "$(DEVICE_ID)" "$(APP_PATH)" || ( \
		echo "\n⚠️  Не удалось установить приложение на iPhone ($(DEVICE_ID))."; \
		echo "Убедитесь, что iPhone подключен по кабелю (или в одной сети Wi-Fi), разблокирован и находится в режиме разработчика, затем выполните: make ios-install ios-launch"; \
		exit 1 \
	)

ios-launch:
	@if [ -z "$(DEVICE_ID)" ]; then \
		echo "Ошибка: подключенный iPhone не найден."; \
		exit 1; \
	fi
	@echo "==> Запуск $(BUNDLE_ID) на iPhone ($(DEVICE_ID))..."
	xcrun devicectl device process launch --device "$(DEVICE_ID)" $(BUNDLE_ID)

iphone: ios-build ios-install ios-launch

iphone-full: xcframework iphone

test:
	cargo test -p ssh-core
	swift test --package-path packages/SshCoreBridge

lint:
	cargo fmt --all -- --check
	cargo clippy -p ssh-core -- -D warnings

clean:
	rm -rf $(DERIVED_DATA)
