# SwitchFix — dev / test helpers
#
#   make            # release build
#   make run        # daemon with debug logs (background, log file)
#   make logs       # tail -f daemon log
#   make doctor     # diagnostics
#   make test       # workspace tests
#
#   make spell W=приветт
#   make stop | status | restart
#   make widget | plasma-restart   — update / reload the Plasma plasmoid

BIN        := ./target/release/switchfix
CARGO      ?= cargo
LOG        := /tmp/switchfix-daemon.log
CONFIG_DIR := $(HOME)/.config/switchfix
RUST_LOG   ?= debug
W          ?= приветт

.PHONY: all build debug test test-verbose clippy fmt \
        run run-fg restart stop status logs logs-history \
        doctor selftest beep settings spell suggest-bar \
        setup install clean watch widget plasma-restart

all: build

## Build

build: ## release build (the binary used by run/doctor/…)
	$(CARGO) build --release

debug: ## faster unoptimized build → ./target/debug/switchfix
	$(CARGO) build

## Test / lint

test: ## workspace tests (must stay green)
	$(CARGO) test --workspace

test-verbose:
	$(CARGO) test --workspace -- --nocapture

clippy:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

fmt:
	$(CARGO) fmt --all

## Run daemon

# heartbeat IPC written by the daemon once the main loop is up
HB         := /tmp/switchfix.daemon.json
START_TMO  := 30   # × 0.1 s to wait for heartbeat
STOP_TMO   := 20   # × 0.1 s to wait for old daemon exit

run: build ## start daemon in background, logs → $(LOG)
	@pkill -x switchfix 2>/dev/null || true
	@i=0; while pgrep -x switchfix >/dev/null && [ $$i -lt $(STOP_TMO) ]; do sleep 0.1; i=$$((i+1)); done
	@rm -f $(HB) /tmp/switchfix-daemon.pid
	@nohup env RUST_LOG=$(RUST_LOG) $(abspath $(BIN)) daemon >$(LOG) 2>&1 & \
		echo "$$!" > /tmp/switchfix-daemon.pid
	@i=0; while [ ! -s $(HB) ] && [ $$i -lt $(START_TMO) ]; do sleep 0.1; i=$$((i+1)); done
	@echo "daemon pid $$(cat /tmp/switchfix-daemon.pid)  log=$(LOG)"
	@id -nG | tr ' ' '\n' | grep -qx input \
		|| echo "(session lacks group 'input' — hotkeys need: sg input -c make run-fg)"
	@$(MAKE) --no-print-directory status

run-fg: build ## daemon in foreground (Ctrl+C to stop) — best for debugging
	RUST_LOG=$(RUST_LOG) $(abspath $(BIN)) daemon

restart: stop run ## bounce the daemon

stop: ## stop running daemon
	@pkill -x switchfix 2>/dev/null || true
	@i=0; while pgrep -x switchfix >/dev/null && [ $$i -lt $(STOP_TMO) ]; do sleep 0.1; i=$$((i+1)); done
	@pgrep -x switchfix >/dev/null && echo "still running (SIGKILL?)" || echo "stopped"
	@rm -f /tmp/switchfix-daemon.pid $(HB)

status: ## is the daemon alive?
	@if ! pgrep -x switchfix >/dev/null; then echo "daemon not running"; exit 1; fi
	@pgrep -a -x switchfix
	@echo "--- heartbeat ---"
	@i=0; peek=""; \
	while [ $$i -lt 10 ]; do \
		peek="$$($(BIN) suggest-peek 2>/dev/null)"; \
		echo "$$peek" | grep -q '"daemon":true' && break; \
		sleep 0.1; i=$$((i+1)); \
	done; \
	echo "$$peek" | head -c 400; echo

## Logs

logs: ## follow daemon log
	tail -f $(LOG)

logs-history: ## spellcheck history (replaces / undos)
	@cat /tmp/switchfix.spell-history.log 2>/dev/null || echo "(empty)"

logs-spell: ## LLM / spell counters
	@$(BIN) spell-stats 2>/dev/null || echo "(daemon not running?)"

watch: logs ## alias

## Diagnostics / quick actions

doctor: build ## platform, groups, uinput, hotkeys, dicts
	$(BIN) doctor

selftest: build ## inject ctrl+f20 via uinput → evdev must see it
	$(BIN) selftest

beep: build ## hotkey feedback sound
	$(BIN) beep

settings: build ## open settings GUI (separate process)
	$(BIN) settings

setup: build ## privileged setup (usermod / udev), interactive
	$(BIN) setup

install: build ## install .desktop + udev rule (packaging/install.sh)
	./packaging/install.sh

## Plasma widget

PLASMOID_ID   := org.switchfix.suggestions
PLASMOID_SRC  := packaging/plasmoid/$(PLASMOID_ID)
PLASMOID_DST  := $(HOME)/.local/share/plasma/plasmoids/$(PLASMOID_ID)

widget: ## copy plasmoid → ~/.local/share + restart plasmashell
	@mkdir -p $(PLASMOID_DST)/contents/ui
	@cp -v $(PLASMOID_SRC)/metadata.json $(PLASMOID_DST)/
	@cp -v $(PLASMOID_SRC)/contents/ui/main.qml $(PLASMOID_DST)/contents/ui/
	@$(MAKE) --no-print-directory plasma-restart

plasma-restart: ## systemctl --user restart plasma-plasmashell.service
	systemctl --user restart plasma-plasmashell.service

## One-shot CLI helpers

spell: build ## make spell W=приветт   — dictionary + edit-1 suggestions
	$(BIN) spell --text '$(W)'

convert: build ## make convert W=ghbdtn  — layout convert only
	$(BIN) convert --text '$(W)'

suggest-bar: build ## terminal suggestion-bar fallback
	$(BIN) suggest-bar

## Housekeeping

clean:
	$(CARGO) clean
	rm -f /tmp/switchfix-daemon.pid /tmp/switchfix-daemon.log

# show self-documenting help
help:
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'
