# Entry point for every Drywire command. Each target runs a script in
# scripts/, where the work happens; `make help` lists the commands.
# Variables from an optional .env file (see .env.example) reach the scripts.

-include .env
export

.DEFAULT_GOAL := help
.PHONY: help dev test fmt vectors bench live desktop deploy status logs backup restore

help: ## List the commands
	@scripts/help.sh

dev: ## Run the server locally: relay and control plane on 127.0.0.1:10050
	@scripts/dev.sh

test: ## Run every test suite
	@scripts/test.sh

fmt: ## Format all code
	@scripts/fmt.sh

vectors: ## Regenerate the protocol test vectors
	@scripts/vectors.sh

bench: ## Replay simulated network traces against the jitter buffer
	@scripts/bench.sh

live: ## Live call on this Mac: you, an echo bot and a local relay (headphones)
	@scripts/live.sh

desktop: ## Run the desktop app in development (Vite on 127.0.0.1:10051)
	@scripts/desktop.sh

# Operating an instance on HOST, an SSH alias set in .env (deploy/README.md).

deploy: ## Build the server for Linux, send it to HOST, start it there
	@scripts/deploy.sh

status: ## Show the server on HOST: container, version, uptime, rooms
	@scripts/status.sh

logs: ## Follow the server logs on HOST
	@scripts/logs.sh

backup: ## Copy an encrypted snapshot of HOST's database to backups/
	@scripts/backup.sh

restore: ## Restore the newest backup, or FILE=..., on HOST, after confirmation
	@scripts/restore.sh
