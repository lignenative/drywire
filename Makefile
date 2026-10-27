# Entry point for every Drywire command. Each target runs a script in
# scripts/, where the work happens; `make help` lists the commands.
# Variables from an optional .env file (see .env.example) reach the scripts.

-include .env
export

.DEFAULT_GOAL := help
.PHONY: help dev test fmt vectors bench

help: ## List the commands
	@scripts/help.sh

dev: ## Run the relay locally (127.0.0.1:10050, TCP and UDP)
	@scripts/dev.sh

test: ## Run every test suite
	@scripts/test.sh

fmt: ## Format all code
	@scripts/fmt.sh

vectors: ## Regenerate the protocol test vectors
	@scripts/vectors.sh

bench: ## Replay simulated network traces against the jitter buffer
	@scripts/bench.sh
