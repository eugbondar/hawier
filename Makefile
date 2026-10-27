SHELL := /bin/bash

# The image name is split so a deploy can ask for a versioned tag without
# changing what a local build is called: `make docker` still writes
# hawier:local-{server,ui}, `make deploy-<host>` passes IMAGE_TAG=$(VERSION).
IMAGE_REPO ?= hawier
IMAGE_TAG ?= local
IMAGE ?= $(IMAGE_REPO):$(IMAGE_TAG)
# Podman first, docker second. A machine with both is usually running podman
# on purpose, and all of this needs only the part of the CLI they share:
# build, images, save, load, pull, rmi, compose. Override with ENGINE=docker.
DEFAULT_ENGINE := $(shell command -v podman >/dev/null 2>&1 && echo podman || echo docker)
ENGINE ?= $(DEFAULT_ENGINE)
COMPOSE ?= $(ENGINE) compose -f deploy/docker-compose.yml
ARGS ?=

.PHONY: all check-tools fmt lint test build run run-ui device it it-keep ui-audit reset-ui-state gc gc-dry gc-auto docker up scale down logs ps clean deploy-list e2e-image e2e screenshots FORCE

all: check-tools lint test build

check-tools:
	@command -v cargo >/dev/null || { echo "cargo is required"; exit 1; }
	@command -v $(ENGINE) >/dev/null || { echo "podman or docker is required (ENGINE=$(ENGINE))"; exit 1; }
	@command -v shellcheck >/dev/null || { echo "shellcheck is required (make lint checks e2e/)"; exit 1; }

fmt:
	cargo fmt --all

lint:
	@# The repository is English-only. The pattern is the Cyrillic block written
	@# as escapes, so this line never matches itself. -I skips binary files:
	@# they hold no text, and the bytes of a PNG match the pattern by chance.
	@! git grep --untracked -I -nP '[\x{0400}-\x{04FF}]' || { echo "Cyrillic found (above): the repository is English-only"; exit 1; }
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	# --all-targets honours `test = false` (apps/*/Cargo.toml), so the two
	# integration suites have to be named or they stop being checked at all.
	# Clippy only type-checks — it writes .rmeta and never links, so naming
	# them here costs no disk.
	cargo clippy -p hawier-server --test it -- -D warnings
	cargo clippy -p hawier-ui --test it -- -D warnings
	@command -v shellcheck >/dev/null || { echo "shellcheck is required: make lint checks the e2e harness"; exit 1; }
	shellcheck -x e2e/*.sh e2e/scenarios/*.sh e2e/br2-external/board/hawier-qemu/*.sh \
		e2e/br2-external/board/hawier-qemu/rootfs-overlay/etc/init.d/* \
		scripts/screenshots.sh

test: gc-auto
	cargo test --workspace

build: gc-auto ext-release
	cargo build --workspace --release

# Extensions are wasm modules, so they need their own target. `ext` builds
# the debug modules the tests load; `ext-release` the ones a release ships.
EXT_CRATES := -p hawier-ext-fs -p hawier-ext-s3 -p hawier-ext-webhook \
              -p hawier-ext-retention -p hawier-ext-rollout-classic \
              -p hawier-ext-rollout-bucket \
              -p hawier-ext-console -p hawier-ext-authz

ext:
	cargo build --target wasm32-wasip1 $(EXT_CRATES)

ext-release:
	cargo build --target wasm32-wasip1 --release $(EXT_CRATES)

run:
	HAWIER_ROLE=control cargo run -p hawier-server

run-ui:
	cargo run -p hawier-ui

device:
	cargo run -p hawier-client --bin hawier-device -- $(ARGS)

# Load test against a running hawier (make up / make run first).
# Tunables: URL, DDI_URL, COUNT, SIZE_MB, TIMEOUT_S.
#   COUNT=1000 SIZE_MB=16 make load
# Point the fleet at a sub node so the progress query does not queue behind
# the fleet's own polling (see bench/NOTES.md):
#   DDI_URL=http://localhost:8081 COUNT=2000 make load
# The script builds the device itself — running it directly must give the
# same binary as running it through make.
load:
	bash scripts/load.sh

# Poll benchmark (the production-shaped load). Tunables: URL, DDI_URL, COUNT,
# INTERVAL, DURATION_S — e.g. COUNT=5000 INTERVAL=500ms make load-poll
load-poll:
	bash scripts/load_poll.sh

# README screenshots (spec: build-tooling, "Console screenshots"): a stand of
# its own on loopback, demo data, a pinned headless browser in a container.
# Writes docs/assets/screenshots/ only when every page rendered. Nothing
# depends on it — the browser image is pulled only here.
screenshots: ext
	cargo build -p hawier-server -p hawier-ui -p hawier-client
	ENGINE=$(ENGINE) bash scripts/screenshots.sh

# `up -d` returns as soon as the container starts, not when Postgres accepts
# connections. `make it` stops Postgres when it finishes, so two runs in a row
# always raced it: every test died on "the database system is starting up".
define wait_for_postgres
	@for i in $$(seq 1 60); do \
		$(COMPOSE) exec -T postgres pg_isready -U hawkbit -q && exit 0; \
		sleep 1; \
	done; \
	echo "postgres did not become ready in 60s" >&2; exit 1
endef

# Integration tests: real Postgres from docker compose, #[ignore]-gated tests.
it: gc-auto ext
	$(COMPOSE) up -d postgres
	$(call wait_for_postgres)
	HAWIER_IT_DATABASE_URL=postgres://hawkbit:hawkbit@localhost:5432/hawkbit \
		cargo test -p hawier-server --test it -- --ignored --test-threads=4
	HAWIER_IT_DATABASE_URL=postgres://hawkbit:hawkbit@localhost:5432/hawkbit \
		cargo test -p hawier-ui --test it -- --ignored --test-threads=4
	$(COMPOSE) stop postgres

# Console UI/UX audit only (all resources x views, dashboards, structure).
# Fast feedback loop for agents touching apps/ui or static/assets.
ui-audit: gc-auto ext
	$(COMPOSE) up -d postgres
	$(call wait_for_postgres)
	HAWIER_IT_DATABASE_URL=postgres://hawkbit:hawkbit@localhost:5432/hawkbit \
		cargo test -p hawier-ui --test it -- --ignored --test-threads=4 ui_audit::

# --- build cache ----------------------------------------------------------
# Cargo never deletes an artifact it has stopped using: every change of the
# dependency graph, of the feature union or of the profile gives a unit a new
# -<hash>, and the previous file stays for good. Four days of work left 47 GB
# in target/, 24 GB of it unreachable. `make clean` is not the way back — it
# also drops target/release/hawier-device (scripts/load.sh measures with it)
# and target/wasm32-wasip1 (make it loads those modules), which is exactly why
# nobody ever runs it. scripts/gc.sh keeps the newest generation of every unit
# and everything cargo linked into target/debug/; see the header there.
gc:
	@scripts/gc.sh

gc-dry:
	@scripts/gc.sh --dry-run

# Sits in front of every building target, so the tail can never pile up again.
# Two O(1) probes, ~3 ms, against the `docker compose up -d postgres` that
# follows it. The file threshold is about four times a collected tree, so
# alternating `make it` (--workspace) with `make ui-audit` (-p hawier-ui) —
# whose feature unions genuinely differ — never trips it. Free space is in the
# condition too: /dev/sdb1 is shared with Docker, so hawier can be tidy and
# still be squeezed out by another project.
GC_TRIP_FILES   ?= 6000
GC_TRIP_FREE_GB ?= 20

gc-auto:
	@[ -d target/debug/deps ] || exit 0; \
	files=$$(ls -U target/debug/deps | wc -l); \
	free=$$(df -BG --output=avail target | tail -1 | tr -dc 0-9); \
	if [ "$$files" -gt $(GC_TRIP_FILES) ] || [ "$$free" -lt $(GC_TRIP_FREE_GB) ]; then \
		echo "== target/debug/deps: $$files files, $${free}G free -- collecting =="; \
		scripts/gc.sh; \
	fi

# The hawier track is one migration file edited in place (development
# stage, no upgrade path). After changing it, drop the console-state tables
# so the file re-applies; hawkBit's sp_* data is untouched.
reset-ui-state:
	$(COMPOSE) exec -T postgres psql -U hawkbit -d hawkbit -c \
	  "DROP TABLE IF EXISTS hawier_saved_view, hawier_action_log, hawier_incident, \
	   hawier_search_query, hawier_session, hawier_event_outbox, \
	   hawier_webhook_delivery, hawier_webhook, hawier_retention_rule, hawier_rollout_ext, hawier_grant, hawier_role CASCADE; \
	   DELETE FROM hawier_schema_version;"

# Same as `it` but leaves postgres running for debugging.
it-keep: gc-auto ext
	$(COMPOSE) up -d postgres
	$(call wait_for_postgres)
	HAWIER_IT_DATABASE_URL=postgres://hawkbit:hawkbit@localhost:5432/hawkbit \
		cargo test -p hawier-server --test it -- --ignored --test-threads=4
	HAWIER_IT_DATABASE_URL=postgres://hawkbit:hawkbit@localhost:5432/hawkbit \
		cargo test -p hawier-ui --test it -- --ignored --test-threads=4

# The image build has no .git: pass the commit and date in so the binaries
# can stamp `vYY.MM.DD-<sha>` (shown on the console's Settings page).
GIT_SHA ?= $(shell git rev-parse --short=9 HEAD 2>/dev/null || echo nogit)
BUILD_DATE ?= $(shell git log -1 --format=%cd --date=format:%y.%m.%d 2>/dev/null || date +%y.%m.%d)
# Debug by default; see deploy/Makefile for why.
CARGO_PROFILE ?= dev
BUILD_ARGS = --build-arg GIT_SHA=$(GIT_SHA) --build-arg BUILD_DATE=$(BUILD_DATE) \
             --build-arg CARGO_PROFILE=$(CARGO_PROFILE)

# Named for what it produces, not for the engine that produces it: $(ENGINE)
# is podman wherever podman is installed.
docker:
	$(ENGINE) build $(BUILD_ARGS) --target server -t $(IMAGE)-server .
	$(ENGINE) build $(BUILD_ARGS) --target ui -t $(IMAGE)-ui .

up:
	$(MAKE) -C deploy up

# Control + sub node. The split-role benchmark needs it: the fleet polls the
# sub node (DDI on 8081) while progress is read from control, so the
# measurement does not queue behind the load it is measuring.
scale:
	$(MAKE) -C deploy scale

down:
	$(MAKE) -C deploy down

logs:
	$(MAKE) -C deploy logs

ps:
	$(MAKE) -C deploy ps

clean:
	cargo clean
	$(ENGINE) image rm -f $(IMAGE)-server $(IMAGE)-ui 2>/dev/null || true

# --- deployment -----------------------------------------------------------
# `make deploy-<host>` builds release images, streams them to the host named
# in deploy/ansible/inventory.yml (or in the untracked inventory.local.yml
# next to it) and brings the stack up there. The tag is
# equal to what the binaries stamp into HAWIER_BUILD by construction — both
# come from BUILD_DATE and GIT_SHA — so `docker image ls` on the host reads as
# a deploy log and matches the console's Settings page.
ANSIBLE_DIR := deploy/ansible
INVENTORY := $(ANSIBLE_DIR)/inventory.yml
# Private hosts live in an untracked file next to it; ansible merges every -i.
INVENTORY_LOCAL := $(ANSIBLE_DIR)/inventory.local.yml
INVENTORY_ARGS := -i $(INVENTORY) $(if $(wildcard $(INVENTORY_LOCAL)),-i $(INVENTORY_LOCAL))
INVENTORY_FILES := $(INVENTORY) $(wildcard $(INVENTORY_LOCAL))
VERSION := v$(BUILD_DATE)-$(GIT_SHA)
# The repo name the host will know these by. `localhost/` survives a save and
# load in both directions, between either engine; a bare name does not. See
# hawier_repo in deploy/ansible/group_vars/all.yml.
DEPLOY_REPO ?= localhost/hawier
# ansible only picks up ansible.cfg from $CWD, and make runs from the root.
ANSIBLE_ENV := ANSIBLE_CONFIG=$(ANSIBLE_DIR)/ansible.cfg
# Pass-through for --check, --diff, -vvv: the escape hatch that keeps
# "everything through make" honest.
ANSIBLE_ARGS ?=

# `ansible all --list-hosts` prints "  hosts (N):" and then one name per line.
deploy_hosts = $$($(ANSIBLE_ENV) ansible $(INVENTORY_ARGS) all --list-hosts 2>/dev/null | tail -n +2 | tr -d ' ')

# An explicit rule beats a pattern rule, so this lands here and not in
# deploy-%. Which also means `list` is a host name you cannot use.
deploy-list:
	@command -v ansible >/dev/null || { \
	  echo "ansible is required: pipx install ansible-core" >&2; exit 1; }
	@echo "hosts in $(INVENTORY_FILES):"; for h in $(deploy_hosts); do echo "  $$h"; done

# .PHONY does not pattern-match, so `.PHONY: deploy-%` would declare a target
# literally named "deploy-%". FORCE is the idiom that works.
# The same two checks in front of every host-taking target. A `define` and not
# a prerequisite, because a prerequisite cannot see the stem.
define check_host
	@command -v ansible-playbook >/dev/null || { \
	  echo "ansible-playbook is required: pipx install ansible-core" >&2; exit 1; }
	@hosts="$(deploy_hosts)"; printf '%s\n' $$hosts | grep -qx "$(1)" || { \
	  echo "unknown host '$(1)'" >&2; echo "hosts in $(INVENTORY_FILES):" >&2; \
	  for h in $$hosts; do echo "  $$h" >&2; done; exit 1; }
endef

# What the stack on a host is doing: container states, restart counts and the
# tail of every service's log. Deploys nothing, needs no version, and works
# when the stack is broken — which is the only time anyone wants it.
#   make logs-prod
#   make logs-prod DIAG_LINES=300
DIAG_LINES ?= 50

REPORT = $(CURDIR)/hawier-$*.log

# ansible's own chatter goes to the file too, and is only shown if the run
# failed: what belongs on the terminal is the report, as text, ready to paste.
logs-%: FORCE
	$(call check_host,$*)
	@$(ANSIBLE_ENV) ansible-playbook $(INVENTORY_ARGS) $(ANSIBLE_DIR)/diagnose.yml \
	  -l "$*" -e hawier_diag_log_lines=$(DIAG_LINES) \
	  -e hawier_report_path=$(REPORT) $(ANSIBLE_ARGS) \
	  > $(REPORT).ansible 2>&1 || { \
	    echo "could not reach $* — ansible said:" >&2; \
	    tail -20 $(REPORT).ansible >&2; exit 1; }
	@cat $(REPORT)
	@echo "-- also in $(REPORT)"

# Who to log in as. The first deploy prints this once and that is no help a
# week later; `cat` on the host is not an answer make is allowed to give.
creds-%: FORCE
	$(call check_host,$*)
	@$(ANSIBLE_ENV) ansible-playbook $(INVENTORY_ARGS) $(ANSIBLE_DIR)/credentials.yml \
	  -l "$*" $(ANSIBLE_ARGS) 2>&1 \
	  | sed -n '/"msg": \[/,/^    \]/p' \
	  | sed -e '/"msg"/d' -e '/^    \]/d' -e 's/^ *"//' -e 's/",\{0,1\}$$//'

deploy-%: FORCE
	$(call check_host,$*)
	@# The inventory is not part of the images (.dockerignore drops deploy/), so
	@# setting a host's address there does not make the version a lie.
	@if [ "$(ALLOW_DIRTY)" != 1 ] && ! git diff --quiet HEAD -- . ':(exclude)$(INVENTORY)'; then \
	  echo "the working tree is dirty: $(VERSION) would not describe what is shipped" >&2; \
	  echo "commit first, or ALLOW_DIRTY=1 make deploy-$*" >&2; exit 1; fi
	@echo "== asking $* whether it can take a deploy =="
	$(ANSIBLE_ENV) ansible-playbook $(INVENTORY_ARGS) $(ANSIBLE_DIR)/preflight.yml \
	  -l "$*" $(ANSIBLE_ARGS)
	@echo "== building $(IMAGE_REPO):$(VERSION)-{server,ui} (release) =="
	$(MAKE) docker CARGO_PROFILE=release IMAGE_REPO=$(DEPLOY_REPO) IMAGE_TAG=$(VERSION)
	@$(ANSIBLE_ENV) ansible-playbook $(INVENTORY_ARGS) $(ANSIBLE_DIR)/site.yml \
	  -l "$*" -e hawier_version=$(VERSION) -e hawier_repo=$(DEPLOY_REPO) \
	  -e hawier_local_engine=$(ENGINE) -e hawier_report_path=$(REPORT) $(ANSIBLE_ARGS) \
	  || { [ -s $(REPORT) ] && { echo; echo "== $* =="; cat $(REPORT); \
	       echo "-- also in $(REPORT)"; }; exit 1; }

# Roll back to a version still on the host: no rebuild, no transfer.
#   make rollback-prod VERSION_OVERRIDE=v25.09.20-abc123def
rollback-%: FORCE
	$(call check_host,$*)
	@[ -n "$(VERSION_OVERRIDE)" ] || { \
	  echo "usage: make rollback-$* VERSION_OVERRIDE=v25.09.20-abc123def" >&2; exit 1; }
	$(ANSIBLE_ENV) ansible-playbook $(INVENTORY_ARGS) $(ANSIBLE_DIR)/site.yml \
	  -l "$*" --skip-tags ship -e hawier_version=$(VERSION_OVERRIDE) \
	  -e hawier_repo=$(DEPLOY_REPO) $(ANSIBLE_ARGS)

FORCE: ;

# Reference device image and the end-to-end run on it. Kept out of `all`
# and `build`: Buildroot is built only when asked for.
include e2e/e2e.mk
