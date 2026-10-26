#!/bin/sh
# Runs on the target tree before the root filesystem image is made.
set -eu

KEYS="${HAWIER_E2E_DIR:?run through make e2e-image}/keys"

# GRUB's own config belongs on the boot partition, not inside a slot.
rm -rf "$TARGET_DIR/boot/grub"
mkdir -p "$TARGET_DIR/bootpart" "$TARGET_DIR/data"
echo 1 > "$TARGET_DIR/etc/hawier-version"

# The signing keys are made here, once, and never leave target/e2e/keys.
# `foreign` signs the update a device must refuse.
mkdir -p "$KEYS"
for key in signing foreign; do
	if [ ! -s "$KEYS/$key.pem" ]; then
		openssl genrsa -out "$KEYS/$key.pem.tmp" 3072 2>/dev/null
		mv "$KEYS/$key.pem.tmp" "$KEYS/$key.pem"
	fi
	openssl rsa -in "$KEYS/$key.pem" -pubout -out "$KEYS/$key.pub.pem" 2>/dev/null
done
install -D -m 0644 "$KEYS/signing.pub.pem" "$TARGET_DIR/etc/swupdate/public.pem"

# The external toolchain installs the C++ runtime because it can; nothing
# on this device is C++, and it is a third of the slot. libconfig++ is the
# only thing that links it, and nothing links libconfig++. Checked, not
# assumed: an ELF that still needs one of them fails the build.
rm -f "$TARGET_DIR"/usr/lib/libstdc++.so* "$TARGET_DIR"/usr/lib/libgomp.so* \
	"$TARGET_DIR"/usr/lib/libconfig++.so*
if find "$TARGET_DIR" -type f -exec readelf -d {} + 2>/dev/null \
	| grep -E 'NEEDED.*(libstdc\+\+|libgomp|libconfig\+\+)'; then
	echo "post-build: something still needs the C++ runtime" >&2
	exit 1
fi

# The package's sysv script would start the web server; ours replaces it
# through the overlay, and its helper is not needed.
rm -f "$TARGET_DIR/usr/lib/swupdate/conf.d/90-start-progress"
