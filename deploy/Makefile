SHELL := /bin/bash

# Podman first, docker second; see the root Makefile. A command-line
# ENGINE= on the parent make reaches this one, a default does not, so the
# probe is repeated rather than inherited.
DEFAULT_ENGINE := $(shell command -v podman >/dev/null 2>&1 && echo podman || echo docker)
ENGINE ?= $(DEFAULT_ENGINE)
COMPOSE ?= $(ENGINE) compose
COMPOSE_FILE ?= docker-compose.yml

# The build context excludes .git, so the commit is handed to the build (it
# ends up in HAWIER_BUILD and on the console's Settings page).
export GIT_SHA ?= $(shell git -C .. rev-parse --short=9 HEAD 2>/dev/null || echo nogit)
export BUILD_DATE ?= $(shell git -C .. log -1 --format=%cd --date=format:%y.%m.%d 2>/dev/null || date +%y.%m.%d)

# Debug by default: these images exist to be looked at, and release spends
# minutes on optimisation nobody watching a page will notice. Override for
# anything that measures the binary or leaves this machine:
#   CARGO_PROFILE=release make scale
export CARGO_PROFILE ?= dev

# The image build needs the outside world (apk, crates.io), and a proxy set
# on the host does not reach into the build container by itself. Forward the
# proxy variables that are set — except loopback ones: inside the container
# 127.0.0.1 is the container, so forwarding it turns "no proxy configured"
# into "connection refused", which is how a rebuild here died once.
# http_proxy & co. are Docker's predefined build args: they stay out of
# `docker history` and out of the build cache, so no credentials are baked in.
PROXY_ARGS := $(foreach v,http_proxy https_proxy no_proxy HTTP_PROXY HTTPS_PROXY NO_PROXY,\
  $(if $($(v)),$(if $(findstring 127.0.0.1,$($(v))),,--build-arg $(v)=$($(v)))))

# Services that are already running are managed regardless of profile; a
# service that is only in a profile is not. The sub node lives in `scale`, so
# a plain `up -d` rebuilt control and left sub on the previous binary — and
# sub is exactly the node the fleet talks to in a split-role benchmark.
# Whatever is running stays on the same build as everything else.
RUNNING_PROFILES := $(shell $(COMPOSE) -f $(COMPOSE_FILE) ps --services 2>/dev/null \
  | grep -qx hawier-sub && echo --profile scale)

.PHONY: build up down logs ps restart scale

build:
	$(COMPOSE) -f $(COMPOSE_FILE) build $(PROXY_ARGS) hawier-ui

up:
	$(COMPOSE) -f $(COMPOSE_FILE) $(RUNNING_PROFILES) build $(PROXY_ARGS)
	$(COMPOSE) -f $(COMPOSE_FILE) $(RUNNING_PROFILES) up -d

# Control + sub: the layout the split-role benchmark needs (DDI on 8081).
scale:
	$(COMPOSE) -f $(COMPOSE_FILE) --profile scale build $(PROXY_ARGS)
	$(COMPOSE) -f $(COMPOSE_FILE) --profile scale up -d

down:
	$(COMPOSE) -f $(COMPOSE_FILE) down

logs:
	$(COMPOSE) -f $(COMPOSE_FILE) logs -f --tail=200

ps:
	$(COMPOSE) -f $(COMPOSE_FILE) ps

restart:
	$(COMPOSE) -f $(COMPOSE_FILE) restart hawier-ui
