SHELL := /bin/bash

PROJECT_NAME := token-usage-insights
PORT ?= 3003
PRICING_FILE := model_prices_and_context_window.json
LITELLM_PRICING_URL ?= https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json

CARGO := cargo
SERVICE_TEMPLATE := shell/token-usage-insights.service
SERVICE_NAME := token-usage-insights.service
SERVICE_FILE := /etc/systemd/system/$(SERVICE_NAME)
RELEASE_BIN := target/release/$(PROJECT_NAME)

.PHONY: help run dev run-release build build-release test test-scripts fmt clippy check lint all clean \
	update-pricing update-litellm-pricing \
	service-file install-service uninstall-service enable-service disable-service \
	start-service stop-service restart-service status

help:
	@echo "Token 戰情室 Makefile"
	@echo
	@echo "常用指令："
	@echo "  make run             以 Release 版本啟動本機伺服器（含建置，預設 PORT=$(PORT)）"
	@echo "  make dev             以 Debug 版本啟動（快速迭代用，預設 PORT=$(PORT)）"
	@echo "  make run-release     等同 run"
	@echo "  make build           建置 Debug 版本"
	@echo "  make build-release   建置 Release 版本"
	@echo "  make test            執行 Rust 測試"
	@echo "  make test-scripts    執行安裝腳本的 systemd 單元測試（不需要 systemd）"
	@echo "  make fmt             套用 Rust formatting"
	@echo "  make clippy          執行 clippy 全量檢查"
	@echo "  make check           執行 cargo check --all-targets --all-features"
	@echo "  make lint            先執行 fmt 後執行 clippy"
	@echo "  make all             依序執行 fmt、check、test、build-release"
	@echo "  make clean           清除建置快取"
	@echo "  make update-pricing  更新 LiteLLM 模型價格表 ($(PRICING_FILE))"
	@echo "  make service-file    產生 service 檔（輸出到標準輸出）"
	@echo "  make install-service 安裝並重新載入 systemd service（需要 sudo）"
	@echo "  make uninstall-service 移除 systemd service 並重新載入（需要 sudo）"
	@echo "  make enable-service  啟用開機自動啟動（需要 sudo）"
	@echo "  make disable-service 停用開機自動啟動（需要 sudo）"
	@echo "  make start-service   啟動 service（需要 sudo）"
	@echo "  make stop-service    停止 service（需要 sudo）"
	@echo "  make restart-service 重啟 service（需要 sudo）"
	@echo "  make status          查詢 service 狀態（需要 sudo）"

run: build-release
	PORT=$(PORT) $(RELEASE_BIN)

dev:
	PORT=$(PORT) $(CARGO) run --bin $(PROJECT_NAME)

run-release: run

build:
	$(CARGO) build

build-release:
	$(CARGO) build --release

test:
	$(CARGO) test

test-scripts:
	bash tests/install-systemd.test.sh

fmt:
	$(CARGO) fmt

clippy:
	$(CARGO) clippy --all-targets --all-features

check:
	$(CARGO) check --all-targets --all-features

lint: fmt clippy

all: fmt check test build-release

clean:
	$(CARGO) clean

update-pricing:
	@echo "正在從 LiteLLM 下載最新價格表..."
	@tmp="$$(mktemp)"; \
	trap 'rm -f "$$tmp"' EXIT; \
	if ! curl -fsSL --retry 3 --connect-timeout 10 -o "$$tmp" "$(LITELLM_PRICING_URL)"; then \
		echo "下載價格表失敗。" >&2; \
		exit 1; \
	fi; \
	if command -v node >/dev/null 2>&1; then \
		node -e "const fs=require('fs'); const d=JSON.parse(fs.readFileSync(process.argv[1],'utf8')); const c=Object.keys(d).length; if(c<100){throw new Error('模型數量異常不足: '+c);} console.log('✓ 驗證通過，共 ' + c + ' 個模型');" "$$tmp" || exit 1; \
	elif command -v python3 >/dev/null 2>&1; then \
		python3 -c "import json, sys; d=json.load(open(sys.argv[1])); c=len(d); sys.exit(0 if c>=100 else 1); print(f'✓ 驗證通過，共 {c} 個模型')" "$$tmp" || exit 1; \
	elif command -v jq >/dev/null 2>&1; then \
		c="$$(jq 'keys | length' "$$tmp")" || exit 1; \
		if [ "$$c" -lt 100 ]; then echo "模型數量異常不足: $$c" >&2; exit 1; fi; \
		echo "✓ 驗證通過，共 $$c 個模型"; \
	fi; \
	mv -f "$$tmp" $(PRICING_FILE) && \
	echo "✓ 已成功更新 $(PRICING_FILE)"

update-litellm-pricing: update-pricing

service-file:
	@sed "s|<PROJECT_DIR>|$(CURDIR)|g" $(SERVICE_TEMPLATE) | \
		sed "s|^Environment=PORT=.*|Environment=PORT=$(PORT)|"

install-service:
	@if [ "$$(uname -s)" != "Linux" ]; then \
		echo "systemd service 安裝僅支援 Linux。"; \
		exit 1; \
	fi
	@tmp="$$(mktemp)"; \
	trap 'rm -f "$$tmp"' EXIT; \
	$(CARGO) build --release && \
		sed "s|<PROJECT_DIR>|$(CURDIR)|g" $(SERVICE_TEMPLATE) | \
		sed "s|^Environment=PORT=.*|Environment=PORT=$(PORT)|" > "$$tmp" && \
		sudo install -m 644 "$$tmp" $(SERVICE_FILE) && \
		sudo systemctl daemon-reload

uninstall-service:
	@if [ "$$(uname -s)" != "Linux" ]; then \
		echo "systemd service 移除僅支援 Linux。"; \
		exit 1; \
	fi
	@sudo systemctl stop $(SERVICE_NAME) || true
	@sudo rm -f $(SERVICE_FILE)
	@sudo systemctl daemon-reload

enable-service:
	@if [ "$$(uname -s)" != "Linux" ]; then \
		echo "systemd service 啟用僅支援 Linux。"; \
		exit 1; \
	fi
	@sudo systemctl enable $(SERVICE_NAME)

disable-service:
	@if [ "$$(uname -s)" != "Linux" ]; then \
		echo "systemd service 停用僅支援 Linux。"; \
		exit 1; \
	fi
	@sudo systemctl disable $(SERVICE_NAME)

start-service:
	@if [ "$$(uname -s)" != "Linux" ]; then \
		echo "systemd service 啟動僅支援 Linux。"; \
		exit 1; \
	fi
	@sudo systemctl start $(SERVICE_NAME)

stop-service:
	@if [ "$$(uname -s)" != "Linux" ]; then \
		echo "systemd service 停止僅支援 Linux。"; \
		exit 1; \
	fi
	@sudo systemctl stop $(SERVICE_NAME)

restart-service:
	@if [ "$$(uname -s)" != "Linux" ]; then \
		echo "systemd service 重啟僅支援 Linux。"; \
		exit 1; \
	fi
	@sudo systemctl restart $(SERVICE_NAME)

status:
	@if [ "$$(uname -s)" != "Linux" ]; then \
		echo "systemd service 查詢僅支援 Linux。"; \
		exit 1; \
	fi
	@sudo systemctl status $(SERVICE_NAME)
