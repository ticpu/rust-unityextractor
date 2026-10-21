NAME := rust-unityextractor
BINARY := rust-unityextractor
CARGO_VERSION := $(shell grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
VERSION := v$(CARGO_VERSION)
GIT_DIRTY := $(shell git diff-index --quiet HEAD -- . || echo dirty)
GIT_TAG := $(shell git tag --points-at HEAD | grep -E '^v')
GIT_VERSION := $(shell git log --oneline . | wc -l)-$(shell git rev-parse --short HEAD)
BASE_VERSION := $(if $(GIT_DIRTY),$(VERSION)+$(GIT_VERSION),$(if $(GIT_TAG),$(VERSION),$(VERSION)+$(GIT_VERSION)))
DEB_VERSION := $(patsubst v%,%,$(BASE_VERSION))
DEB_AMD64 := $(NAME)_$(DEB_VERSION)_amd64.deb
DEB_ARM64 := $(NAME)_$(DEB_VERSION)_arm64.deb
BINARIES := dist/$(BINARY).x86_64-linux dist/$(BINARY).aarch64-linux dist/$(BINARY).x86_64-windows.exe
PKG := package.tmp

.PHONY: all clean binaries deb deb-amd64 deb-arm64

all: binaries

binaries: $(BINARIES)

deb: deb-amd64 deb-arm64

deb-amd64: $(DEB_AMD64)

deb-arm64: $(DEB_ARM64)

define build-deb
	rm -rf "$(PKG)"
	install -D -m 755 -T "$<" "$(PKG)/usr/bin/$(BINARY)"
	install -D -m 644 -T README.md "$(PKG)/usr/share/doc/$(NAME)/README.md"
	install -D -m 644 -T LICENSE "$(PKG)/usr/share/doc/$(NAME)/copyright"
	install -D -m 644 -T DEBIAN/control "$(PKG)/DEBIAN/control"
	sed -i -e "s/^Version:.*/Version: $(DEB_VERSION)/" \
		-e "s/^Architecture:.*/Architecture: $(1)/" \
		"$(PKG)/DEBIAN/control"
	@if grep -rq "$$PWD" "$(PKG)"; then echo "ERROR: package contains build path ($$PWD)" >&2; exit 1; fi
	dpkg-deb --build --root-owner-group "$(PKG)" "$@"
	rm -rf "$(PKG)"
endef

$(DEB_AMD64): dist/$(BINARY).x86_64-linux DEBIAN/control README.md LICENSE
	$(call build-deb,amd64)

$(DEB_ARM64): dist/$(BINARY).aarch64-linux DEBIAN/control README.md LICENSE
	$(call build-deb,arm64)

# One container build cross-compiles all three, so they share a single rule
$(BINARIES) &: build.sh Containerfile Cargo.toml Cargo.lock $(wildcard src/*)
	./build.sh

clean:
	rm -rf "$(PKG)" dist *.deb
