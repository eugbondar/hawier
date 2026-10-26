#!/bin/sh
# Builds everything the harness boots and installs, from one target tree
# (design D8): the slot images, the disk, and the signed .swu variants.
set -eu

BOARD_DIR=$(cd "$(dirname "$0")" && pwd)
KEYS="${HAWIER_E2E_DIR:?run through make e2e-image}/keys"
WORK="$BUILD_DIR/hawier-e2e"
rm -rf "$WORK" && mkdir -p "$WORK"

# --- slot images ------------------------------------------------------------
# v1 is Buildroot's own root filesystem. The others are it with one change,
# repacked with the same compression.
cp "$BINARIES_DIR/rootfs.squashfs" "$BINARIES_DIR/slot-v1.squashfs"
repack() { # <name> <version> <edit command…>
	name=$1 version=$2
	shift 2
	tree="$WORK/tree-$name"
	"$HOST_DIR/bin/unsquashfs" -q -n -d "$tree" "$BINARIES_DIR/rootfs.squashfs"
	echo "$version" > "$tree/etc/hawier-version"
	"$@" "$tree"
	"$HOST_DIR/bin/mksquashfs" "$tree" "$BINARIES_DIR/slot-$name.squashfs" \
		-noappend -all-root -comp xz -quiet
}
repack v2 2 true
# No init: with init=/sbin/init on the command line the kernel panics, and
# panic=5 reboots it into the fallback path.
remove_init() { rm -f "$1/sbin/init"; }
repack noinit 2 remove_init

# --- boot partition and disk ------------------------------------------------
staging="$WORK/bootpart"
mkdir -p "$staging/boot/grub"
cp "$BOARD_DIR/grub.cfg" "$staging/boot/grub/grub.cfg"
# A GRUB environment block is exactly 1024 bytes: header, variables, '#' fill.
{
	printf '# GRUB Environment Block\nrootslot=a\nustate=0\n'
	head -c 1024 /dev/zero | tr '\0' '#'
} | head -c 1024 > "$staging/boot/grub/grubenv"
cp "$(find "$BUILD_DIR" -path "*/build-i386-pc/grub-core/boot.img" | head -n 1)" "$BINARIES_DIR/boot.img"

rm -rf "$WORK/genimage.tmp"
"$HOST_DIR/bin/genimage" \
	--rootpath "$staging" \
	--tmppath "$WORK/genimage.tmp" \
	--inputpath "$BINARIES_DIR" \
	--outputpath "$BINARIES_DIR" \
	--config "$BOARD_DIR/genimage.cfg"

# --- signed updates ---------------------------------------------------------
swu() { # <output> <slot image> <version> <revision> <key>
	out=$1 slot=$2 version=$3 revision=$4 key=$5
	dir="$WORK/swu-$out"
	mkdir -p "$dir"
	cp "$BINARIES_DIR/$slot" "$dir/slot.squashfs"
	sha=$(sha256sum "$dir/slot.squashfs" | cut -d' ' -f1)
	sed -e "s|@VERSION@|$version|" -e "s|@REVISION@|$revision|" -e "s|@SHA256@|$sha|g" \
		"$BOARD_DIR/sw-description.in" > "$dir/sw-description"
	openssl dgst -sha256 -sign "$KEYS/$key.pem" -out "$dir/sw-description.sig" "$dir/sw-description"
	# sw-description first, its signature second: SWUpdate reads them before
	# anything else and refuses the rest if they do not verify.
	(cd "$dir" && printf 'sw-description\nsw-description.sig\nslot.squashfs\n' \
		| cpio -o -H crc --quiet) > "$BINARIES_DIR/$out"
}
swu update-rev1.swu         slot-v2.squashfs     2 rev1 signing
swu update-rev1-foreign.swu slot-v2.squashfs     2 rev1 foreign
swu update-rev2-label.swu   slot-v2.squashfs     2 rev2 signing
swu fallback-rev1.swu       slot-noinit.squashfs 2 rev1 signing
