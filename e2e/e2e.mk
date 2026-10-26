# Reference device image and the end-to-end run on it. Included by the root
# Makefile; the behaviour is in openspec/specs/reference-device-image and
# device-e2e, the reasons in the design of the qemu-swupdate-e2e change.
#
# Everything lands in target/e2e/, which build-cache GC never looks at
# (scripts/gc.sh only scans target/debug):
#   dl/          Buildroot downloads: sources, the Bootlin toolchain
#   ccache/      compiler cache
#   buildroot-*/ the pinned Buildroot tree
#   br-output/   Buildroot's O=
#   keys/        signing keys, generated on first build, never committed
#   out/         what the harness consumes: disk.img and the .swu files

E2E_DIR := target/e2e
E2E_OUT := $(E2E_DIR)/out

# Pinned by version and by the sha256 from the release's signed .sign file.
# The Bootlin toolchain and every package are pinned by this release's own
# hash files, so this one pin pins the whole image.
BR_VERSION := 2026.02.3
BR_SHA256 := 5a59e7501b0b4ec52c41f4bfa79412320e0b37eae5f719605a258e8d0c6fc7fb
BR_URL := https://buildroot.org/downloads/buildroot-$(BR_VERSION).tar.xz
BR_TARBALL := $(E2E_DIR)/dl/buildroot-$(BR_VERSION).tar.xz
BR_SRC := $(E2E_DIR)/buildroot-$(BR_VERSION)
BR_OUT := $(E2E_DIR)/br-output
BR_EXTERNAL := e2e/br2-external
BR_DEFCONFIG := hawier_qemu_defconfig

E2E_BUILDER := hawier-e2e-builder
# Rootless podman maps the container's root onto the invoking user, so files
# come out owned by them as they are. Docker would leave them owned by root:
# it gets the user explicitly. The one engine-specific branch, asked for
# explicitly as the engine rule allows.
E2E_USER := $(if $(filter docker,$(notdir $(ENGINE))),--user $(shell id -u):$(shell id -g))
# The build container shares the host's network namespace: on a machine whose
# proxy listens on loopback, that is the only way to reach it (and in CI it
# changes nothing). The proxy variables are forwarded as they are.
E2E_PROXY := $(foreach v,http_proxy https_proxy no_proxy HTTP_PROXY HTTPS_PROXY NO_PROXY,\
  $(if $($(v)),-e $(v)=$($(v))))
E2E_RUN = $(ENGINE) run --rm --network=host $(E2E_USER) $(E2E_PROXY) -e HOME=/tmp \
  -e HAWIER_E2E_DIR=/src/$(E2E_DIR) -v $(CURDIR):/src -w /src $(E2E_BUILDER)

# The image is rebuilt only when something it is made of changed. Buildroot
# itself would re-run rootfs generation and re-sign every .swu on each call,
# and a warm `make e2e-image` must neither change an output nor need the
# network.
E2E_INPUTS := $(shell find $(BR_EXTERNAL) e2e/Containerfile -type f 2>/dev/null)

.PHONY: e2e-image e2e-builder e2e

e2e-image: $(E2E_OUT)/.built
	@bash e2e/budget.sh $(E2E_DIR)

$(BR_TARBALL):
	@mkdir -p $(@D)
	curl -fL --retry 3 -o $@.part $(BR_URL)
	@got=$$(sha256sum $@.part | cut -d' ' -f1); \
	if [ "$$got" != "$(BR_SHA256)" ]; then \
	  rm -f $@.part; \
	  echo "e2e-image: checksum mismatch for buildroot-$(BR_VERSION).tar.xz:" \
	       "expected $(BR_SHA256), got $$got" >&2; \
	  exit 1; \
	fi
	mv $@.part $@

$(BR_SRC)/.extracted: $(BR_TARBALL)
	@got=$$(sha256sum $< | cut -d' ' -f1); \
	if [ "$$got" != "$(BR_SHA256)" ]; then \
	  echo "e2e-image: checksum mismatch for $<: expected $(BR_SHA256), got $$got" >&2; \
	  exit 1; \
	fi
	rm -rf $(BR_SRC) && mkdir -p $(E2E_DIR)
	tar -xJf $< -C $(E2E_DIR)
	touch $@

# Built on every image build and cached by the engine, so a changed
# Containerfile is picked up without any bookkeeping here.
# Silent recipes: the forwarded proxy variables may carry credentials, and
# make would print them.
e2e-builder:
	@echo "e2e-image: builder image $(E2E_BUILDER)"
	@$(ENGINE) build --network=host $(foreach v,http_proxy https_proxy no_proxy HTTP_PROXY HTTPS_PROXY NO_PROXY,\
	  $(if $($(v)),--build-arg $(v)=$($(v)))) -t $(E2E_BUILDER) -f e2e/Containerfile e2e

$(E2E_OUT)/.built: $(BR_SRC)/.extracted $(E2E_INPUTS)
	$(MAKE) e2e-builder
	@mkdir -p $(BR_OUT) $(E2E_DIR)/keys $(E2E_DIR)/ccache
	@echo "e2e-image: buildroot $(BR_DEFCONFIG) in $(BR_OUT)"
	@$(E2E_RUN) make -C $(BR_SRC) O=/src/$(BR_OUT) BR2_EXTERNAL=/src/$(BR_EXTERNAL) $(BR_DEFCONFIG)
	@$(E2E_RUN) make -C $(BR_SRC) O=/src/$(BR_OUT)
	@mkdir -p $(E2E_OUT)
	@for f in disk.img update-rev1.swu update-rev1-foreign.swu update-rev2-label.swu \
	          fallback-rev1.swu slot-v1.squashfs; do \
	  cmp -s $(BR_OUT)/images/$$f $(E2E_OUT)/$$f || cp $(BR_OUT)/images/$$f $(E2E_OUT)/$$f; \
	done
	touch $@

# The run itself (spec: device-e2e). The node under test is the dev binary
# with the debug extension modules: the image here is what is being tested
# against, not what gets shipped. E2E_ONLY=<scenario> runs one scenario.
e2e: e2e-image ext
	cargo build -p hawier-server
	ENGINE=$(ENGINE) E2E_DIR=$(E2E_DIR) E2E_ONLY=$(E2E_ONLY) bash e2e/run.sh
