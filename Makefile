.DEFAULT_GOAL := help
GO ?= go
GOFMT ?= gofmt
WORDS ?=
APP_DIR ?=
APP_PACKAGE ?= .
APP_NAME ?=
DESTDIR ?=
ARCH ?= $(shell uname -m | sed -e 's/^x86_64$$/amd64/' -e 's/^aarch64$$/arm64/')
export GO WORDS APP_DIR APP_PACKAGE APP_NAME DESTDIR ARCH
export GOWORK := off
VERSION := $(shell cat VERSION)

.PHONY: help verify build test dev dev-fmt dev-fmt-check dev-vet dev-test dev-race dev-build dev-wordcount dev-mesh-a dev-mesh-b release release-check deploy-build deploy

help: ## List development, release and deployment commands
	@awk 'BEGIN {FS=":.*## "} /^[a-z][a-z-]*:.*## / {printf "  %-18s %s\n", $$1, $$2}' Makefile

verify: dev ## Run every check (format, vet, tests, race, build)

build: dev-build ## Build the examples under .dev/bin

test: dev-test ## Run the tests

dev: dev-fmt-check dev-vet dev-test dev-race dev-build ## Run the complete development checks

dev-fmt: ## Format the Go source
	@"$(GOFMT)" -w *.go examples/*/*.go examples/workflows/*/*.go workflow/*.go sketch/*.go clienthttp/*.go cmd/dostoevsky-gateway/*.go wire/*.go wireserver/*.go cmd/dostoevsky-server/*.go cmd/dost-conform/*.go internal/seam/*.go

dev-fmt-check: ## Check formatting without changing files
	@files=$$("$(GOFMT)" -l *.go examples/*/*.go examples/workflows/*/*.go workflow/*.go sketch/*.go clienthttp/*.go cmd/dostoevsky-gateway/*.go wire/*.go wireserver/*.go cmd/dostoevsky-server/*.go cmd/dost-conform/*.go internal/seam/*.go) && { test -z "$$files" || { echo "$$files"; exit 1; }; }

dev-vet: ## Run Go static analysis
	@"$(GO)" vet ./...

dev-test: ## Run tests with external module access and cgo disabled
	@CGO_ENABLED=0 GOPROXY=off "$(GO)" test ./...

dev-race: ## Run tests with the race detector (requires a C compiler)
	@CGO_ENABLED=1 "$(GO)" test -race ./...

dev-build: ## Build both examples under .dev/bin
	@mkdir -p .dev/bin
	@CGO_ENABLED=0 "$(GO)" build -o .dev/bin/ ./examples/...

dev-wordcount: ## Run the word-count example with WORDS='blue red blue'
	@set -f; "$(GO)" run ./examples/wordcount .dev/data/counts $$WORDS

dev-mesh-a: ## Start loopback demo node a
dev-mesh-b: ## Start loopback demo node b
dev-mesh-a dev-mesh-b:
	@DOST_TOKEN="$${DOST_TOKEN:-local-example-only}" "$(GO)" run ./examples/mesh -node "$(@:dev-mesh-%=%)" -dir ".dev/data/$(@:dev-mesh-%=%)"

release: dev ## Build a checked, versioned source tarball under dist/
	@sh scripts/release.sh "$(VERSION)"

release-check: ## Verify the current source tarball and test its extracted contents offline
	@sh scripts/release.sh "$(VERSION)" verify

deploy-build: ## Build a Linux host executable (APP_DIR, APP_PACKAGE, APP_NAME, ARCH)
	@sh scripts/deploy.sh build

deploy: ## Build and atomically install the host executable into an explicit DESTDIR
	@sh scripts/deploy.sh install

.PHONY: dev-bench-notify
dev-bench-notify: ## Measure repeated signals with eight already-notified readers
	@"$(GO)" test -run '^$$' -bench '^BenchmarkPendingSignal8$$' -benchmem .

RUN_DIR ?= .dev/data/openai
export RUN_DIR
.PHONY: dev-openai-text dev-openai-agent dev-openai-replay
dev-openai-text: ## Record live LLM output events in RUN_DIR
dev-openai-agent: ## Record a hosted agent task in RUN_DIR
dev-openai-replay: ## Replay recorded events offline
dev-openai-text dev-openai-agent dev-openai-replay:
	@"$(GO)" run ./examples/openai "$(@:dev-openai-%=%)" "$$RUN_DIR"

QUEUE_DIR ?= .dev/data/taskqueue
JOB_ID ?= example-1
TASK ?= sha256
INPUT ?= hello
export QUEUE_DIR JOB_ID TASK INPUT
.PHONY: dev-task-enqueue dev-task-drain dev-task-inspect
dev-task-enqueue: ## Submit a durable job using JOB_ID, TASK and INPUT
	@"$(GO)" run ./examples/taskqueue "$$QUEUE_DIR" enqueue "$$JOB_ID" "$$TASK" "$$INPUT"
dev-task-drain: ## Run one worker per partition until the queue drains
	@"$(GO)" run ./examples/taskqueue "$$QUEUE_DIR" drain
dev-task-inspect: ## Print completed jobs and dead letters
	@"$(GO)" run ./examples/taskqueue "$$QUEUE_DIR" inspect

BURST_GROUPS ?= 0
BURST_DURATION ?= 0s
BURST_RECORDS ?= 100000
BURST_TOPICS ?= 100000
BURST_BATCH ?= 128
BURST_WORKERS ?= 4
.PHONY: dev-burst
dev-burst: ## Benchmark durable ingest and verify all record IDs after restart
	@"$(GO)" run ./examples/burst -count "$(BURST_RECORDS)" -topics "$(BURST_TOPICS)" -batch "$(BURST_BATCH)" -workers "$(BURST_WORKERS)" -duration "$(BURST_DURATION)" -groups "$(BURST_GROUPS)"

.PHONY: dev-bench-embedded
dev-bench-embedded: ## Compare synced single-record publishes with embedded JetStream on Linux
	@sh benchmarks/embedded/run.sh

.PHONY: dev-bench-publish
dev-bench-publish: ## Measure concurrent durable publication against embedded JetStream on Linux
	@BENCH_SUITE=concurrent sh benchmarks/embedded/run.sh

.PHONY: dev-bench-publish-gate
dev-bench-publish-gate: ## Compare median throughput and peak RSS with JetStream at 1, 8 and 32 producers
	@BENCH_SUITE=gate sh benchmarks/embedded/run.sh

.PHONY: plots
plots: ## Regenerate docs/img/*.svg from the raw receipts (standard library only)
	@"$(GO)" run ./cmd/plots
	@"$(GO)" run ./cmd/plots -receipt docs/receipts/production-03/explicit-2.log -section '*' -meta docs/receipts/production-03/meta.txt -sketch /nonexistent -architecture=false -out docs/img/production-03

.PHONY: sketch-report
sketch-report: ## Measure sketch accuracy against exact values and write docs/sketch-accuracy-results.txt
	@"$(GO)" run ./cmd/sketchreport > docs/sketch-accuracy-results.txt
	@echo wrote docs/sketch-accuracy-results.txt

.PHONY: result
result: ## Validate a comparison log and print medians: make result LOG=comparison.log ENGINES=a,b,c [TRIALS=5]
	@test -n "$(LOG)" -a -n "$(ENGINES)" || { echo "usage: make result LOG=comparison.log ENGINES=dostoevsky,jetstream,..." >&2; exit 2; }
	@"$(GO)" run ./cmd/plots -summary -receipt "$(LOG)" -expect-engines "$(ENGINES)" -trials "$(or $(TRIALS),5)"

.PHONY: conformance
conformance: ## Run the conformance cases against this engine (Linux, Python 3.10+)
	@mkdir -p .dev/bin
	@CGO_ENABLED=0 "$(GO)" build -o .dev/bin/dost-conform ./cmd/dost-conform
	@PYTHONDONTWRITEBYTECODE=1 python3 scripts/conformance.py --engine go=.dev/bin/dost-conform

# The Rust engine (rust/README.md). Its build output goes to .dev/rust-target.
CARGO ?= cargo
RUST_DRIVER = .dev/rust-target/release/dost-conform

.PHONY: rust-check rust-driver conformance-rust differential
rust-check: ## Check the Rust workspace: format, lints as errors, tests
	@cd rust && "$(CARGO)" fmt --check && "$(CARGO)" clippy --all-targets -- -D warnings && "$(CARGO)" test

rust-driver:
	@cd rust && "$(CARGO)" build --release --bin dost-conform

conformance-rust: rust-driver ## Run the conformance cases against both engines, each opening the other's stores
	@mkdir -p .dev/bin
	@CGO_ENABLED=0 "$(GO)" build -o .dev/bin/dost-conform ./cmd/dost-conform
	@PYTHONDONTWRITEBYTECODE=1 python3 scripts/conformance.py --engine go=.dev/bin/dost-conform --engine rust=$(RUST_DRIVER)

differential: rust-driver ## Compare the bytes both engines write for the same requests (needs the test seam in both)
	@mkdir -p .dev/bin
	@CGO_ENABLED=0 "$(GO)" build -o .dev/bin/dost-conform ./cmd/dost-conform
	@PYTHONDONTWRITEBYTECODE=1 python3 scripts/differential_selftest.py
	@PYTHONDONTWRITEBYTECODE=1 python3 scripts/differential.py --engine go=.dev/bin/dost-conform --engine rust=$(RUST_DRIVER)

# Clients in other languages (see docs/clients.md). Each target needs only its own toolchain.
JAVAC ?= javac
CLIENT_LANGUAGES ?= python javascript typescript java rust

.PHONY: dev-gateway dev-client-python dev-client-javascript dev-client-typescript dev-client-java dev-client-rust dev-client-interop
dev-gateway: ## Run the client gateway on loopback (needs DOSTOEVSKY_TOKEN; make one with: go run ./cmd/dostoevsky-gateway -gen-token)
	@"$(GO)" run ./cmd/dostoevsky-gateway

dev-client-python: ## Run the Python publisher and consumer (Python 3.10+)
	@PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=clients/python python3 examples/clients/python/worker.py

dev-client-javascript: ## Run the JavaScript publisher and consumer (Node 18+)
	@node examples/clients/javascript/worker.mjs

dev-client-typescript: ## Run the TypeScript publisher and consumer (Node 22.18+ strips types; no compiler needed)
	@node examples/clients/typescript/worker.mts

dev-client-java: ## Compile and run the Java publisher and consumer (JDK 17+)
	@mkdir -p .dev/clients/java
	@"$(JAVAC)" --release 17 -encoding UTF-8 -d .dev/clients/java clients/java/src/main/java/io/github/rodriveracom/dostoevsky/Client.java examples/clients/java/Worker.java
	@java -cp .dev/clients/java Worker

dev-client-rust: ## Build and run the Rust publisher and consumer (no crates; builds offline)
	@CARGO_TARGET_DIR="$(CURDIR)/.dev/clients/rust" cargo run --offline --quiet --manifest-path examples/clients/rust/Cargo.toml

dev-client-interop: ## All clients against one gateway across a restart, disposable directory: CLIENT_LANGUAGES="python java" for a subset
	@mkdir -p .dev/bin
	@CGO_ENABLED=0 "$(GO)" build -o .dev/bin/dostoevsky-gateway ./cmd/dostoevsky-gateway
	@PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=clients/python python3 scripts/client_interop.py $(CLIENT_LANGUAGES)

.PHONY: test-scripts
test-scripts: ## Test the scripts' own plumbing (the interop startup deadline); stdlib Python only, no gateway needed
	@PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=clients/python python3 scripts/test_client_interop.py

HISTORY_COUNT ?= 16000
HISTORY_MODE ?= open
HISTORY_ID_FLAGS ?=
.PHONY: dev-history bench-clients
dev-history: ## Measure seed, open or verify in a disposable HISTORY_DIR on Linux
	@test -n "$(HISTORY_DIR)" || { echo "set HISTORY_DIR to a disposable benchmark directory" >&2; exit 2; }
	@"$(GO)" run ./benchmarks/history -dir "$(HISTORY_DIR)" -mode "$(HISTORY_MODE)" -n "$(HISTORY_COUNT)" $(HISTORY_ID_FLAGS)

bench-clients: ## Run the language comparison with CLIENT_LANGS, CLIENT_ENGINES and BENCH_* settings
	@sh benchmarks/clients/run.sh

.PHONY: dev-native-kill
dev-native-kill: ## Verify durable receipts and exact retries across SIGKILL on Linux
	@mkdir -p .dev/bin
	@CGO_ENABLED=0 "$(GO)" build -o .dev/bin/dostoevsky-server ./cmd/dostoevsky-server
	@PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=clients/python python3 benchmarks/recovery/kill_native.py .dev/bin/dostoevsky-server

.PHONY: dev-peer-kill
dev-peer-kill: ## Verify peer receipts after forced restarts using the configured benchmark binaries and venv
	@PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=clients/python .dev/bench-venv/bin/python benchmarks/recovery/kill_peers.py

.PHONY: dev-storage dev-storage-recovery dev-storage-crash dev-storage-scale
dev-storage: ## Build offline storage verification, migration and backup tool
	@mkdir -p .dev/bin
	@"$(GO)" build -o .dev/bin/dost-storage ./cmd/dost-storage

dev-storage-recovery: ## Test segmented recovery, cross-format restoration and loss detection
	@"$(GO)" test -race -count=1 -run 'Segment|Frame3|Checkpoint3' .

dev-storage-crash: ## Kill the native segmented server and verify 20,000 receipts and retries
	@mkdir -p .dev/bin
	@"$(GO)" build -o .dev/bin/dostoevsky-server ./cmd/dostoevsky-server
	@PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=clients/python python3 benchmarks/recovery/kill_native.py .dev/bin/dostoevsky-server --segmented --count 20000

RECOVERY_RECORDS ?= 1000000
RECOVERY_BYTES ?= 512
RECOVERY_PARENT ?= .dev
dev-storage-scale: ## Measure five fresh opens, integrity scan and exact paged readback on Linux
	@mkdir -p .dev/bin "$(RECOVERY_PARENT)"
	@"$(GO)" build -o .dev/bin/history ./benchmarks/history
	@python3 benchmarks/recovery/scale.py .dev/bin/history "$(RECOVERY_PARENT)" "$(RECOVERY_RECORDS)" "$(RECOVERY_BYTES)"

RECOVERY_TOPICS ?= 100000
.PHONY: dev-storage-catalog
dev-storage-catalog: ## Measure recovery with many one-record topics and consumer checkpoints
	@mkdir -p .dev/bin "$(RECOVERY_PARENT)"
	@"$(GO)" build -o .dev/bin/catalog ./benchmarks/catalog
	@set -eu; d=$$(mktemp -d "$(RECOVERY_PARENT)/catalog-XXXXXX"); trap 'rm -rf "$$d"' EXIT; .dev/bin/catalog -seed -dir "$$d" -topics "$(RECOVERY_TOPICS)"; .dev/bin/catalog -dir "$$d" -topics "$(RECOVERY_TOPICS)"

ID_HISTORY ?= 1000000
ID_ROUNDS ?= 3
ID_PYTHON ?= python3
.PHONY: dev-id-publish dev-id-peers dev-id-eventloop
dev-id-publish: ## Compare identified publishes with 1 and 32 workers on a disposable Linux volume
	@mkdir -p .dev/bin "$(RECOVERY_PARENT)"
	@"$(GO)" build -o .dev/bin/id-fixture ./benchmarks/idpublish
	@"$(GO)" test -c -o .dev/bin/id-workload .
	@python3 benchmarks/idpublish/local.py .dev/bin/id-fixture .dev/bin/id-workload "$(RECOVERY_PARENT)" "$(ID_HISTORY)" --rounds "$(ID_ROUNDS)"

dev-id-peers: ## Compare loopback native clients and verify receipts after SIGKILL (Linux)
	@mkdir -p .dev/bin
	@"$(GO)" build -o .dev/bin/dostoevsky-server ./cmd/dostoevsky-server
	@ID_NATIVE_BIN="$(CURDIR)/.dev/bin/dostoevsky-server" BENCH_NATIVE_SERVER="$(CURDIR)/benchmarks/idpublish/segmented-server.sh" PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=clients/python "$(ID_PYTHON)" benchmarks/idpublish/peers.py --rounds "$(ID_ROUNDS)"

dev-id-eventloop: ## Cross-check identified publish throughput with JavaScript native and JetStream clients
	@mkdir -p .dev/bin
	@"$(GO)" build -o .dev/bin/dostoevsky-server ./cmd/dostoevsky-server
	@ID_NATIVE_BIN="$(CURDIR)/.dev/bin/dostoevsky-server" BENCH_NATIVE_SERVER="$(CURDIR)/benchmarks/idpublish/segmented-server.sh" node benchmarks/idpublish/eventloop.mjs

.PHONY: dev-bench-admission dev-bench-hot-topics
dev-bench-admission: ## Compare the original and candidate index policies on the same traces
	@sh scripts/admission-bench.sh

dev-bench-hot-topics: ## Measure topic tracker accuracy, memory and observation cost
	@"$(GO)" test -run HotTopic -v -bench HotTopics -benchmem -count=3 ./sketch

.PHONY: dev-bench-fresh
dev-bench-fresh: ## Compare fresh segmented durable publishes against 93acac3
	@sh scripts/fresh-publish-bench.sh

.PHONY: dev-profile-fresh dev-test-sync-modes dev-test-sockets
dev-profile-fresh: ## Time single-publisher writes and syncs on a disposable segmented fixture
	@test -n "$(DOST_FRESH_PROFILE_DIR)" || { echo "Set DOST_FRESH_PROFILE_DIR to a disposable data directory"; exit 2; }
	@"$(GO)" test -v -count=1 -run '^TestFreshDurableStages$$' .

dev-test-sync-modes: ## Compare Linux synchronous write modes on the target volume
	@test -d "$(DOST_SYNC_MODE_DIR)" || { echo "Set DOST_SYNC_MODE_DIR to an existing directory"; exit 2; }
	@"$(GO)" test -v -count=1 -run '^TestFreshSyncModes$$' .

dev-test-sockets: ## Exercise Unix socket permissions and stale and active listeners
	@"$(GO)" test -v -count=1 -run '^(TestUnixSocketIsPrivate|TestAStaleSocketOfOursIsReplaced|TestALiveSocketIsNotDisplaced)$$' ./cmd/dostoevsky-server

.PHONY: dev-test-workflows dev-bench-workflows dev-test-ephemeral
dev-test-workflows: ## Check workflow conformance, crash recovery and competing leases
	@"$(GO)" test -race -count=1 ./workflow

dev-bench-workflows: ## Measure complete three-activity workflows on a disposable volume
	@test -d "$(DOST_WORKFLOW_BENCH_DIR)" || { echo "Set DOST_WORKFLOW_BENCH_DIR to a disposable directory"; exit 2; }
	@"$(GO)" test -count=3 -v -run '^TestWorkflowCost$$' ./workflow

dev-test-ephemeral: ## Run the same public-API scenarios against a vendored SQLite reference
	@sh scripts/workflow-conformance.sh

.PHONY: dev-bench-workflow-reopen
dev-bench-workflow-reopen: ## Measure fresh-process reopen memory versus retained closed runs
	@test -d "$(DOST_WORKFLOW_BENCH_DIR)" || { echo "Set DOST_WORKFLOW_BENCH_DIR to a disposable directory"; exit 2; }
	@"$(GO)" test -count=1 -v -run '^TestWorkflowReopenFootprint$$' ./workflow

.PHONY: dev-test-workflow-boundary dev-bench-workflow-boundary
dev-test-workflow-boundary: ## Exercise workflow commit witnesses, corruption and rollover kills
	@"$(GO)" test -race -count=1 -v -run 'WorkflowBoundary|WorkflowWitness|WorkflowGroupCrashImages|AbruptExit|MissingUncommittedSuccessor' . ./workflow

dev-bench-workflow-boundary: ## Compare workflow durability costs against bcc0238 on a disposable disk
	@sh scripts/workflow-boundary-bench.sh

.PHONY: dev-bench-workflow-scale
dev-bench-workflow-scale: ## Measure 3 and 30 activities and 100,000 durable signal waits on Linux
	@test -n "$(DOST_WORKFLOW_BENCH_DIR)" || (echo "Set DOST_WORKFLOW_BENCH_DIR to a disposable disk directory"; exit 1)
	DOST_WORKFLOW_BENCH_DIR="$(DOST_WORKFLOW_BENCH_DIR)" GOMAXPROCS=4 $(GO) test -count=1 -timeout 30m -v ./workflow -run '^TestWorkflow(Cost|LongCost)$$'
	@for workers in 1 32; do DOST_WAIT_READERS=$$workers DOST_WORKFLOW_BENCH_DIR="$(DOST_WORKFLOW_BENCH_DIR)" GOMAXPROCS=4 $(GO) test -count=1 -timeout 30m -v ./workflow -run '^TestWorkflowWaitingScale$$' || exit 1; done
