#!/usr/bin/env bash
#
# Collect the build artifacts cargo has stopped using. Entry point: `make gc`.
#
# Cargo never deletes an artifact it has abandoned. Every change of the
# dependency graph, of the feature union or of the profile gives a unit a new
# -<hash>, and the previous file stays on disk for good: four days of work left
# 47 GB in target/, 24 GB of it unreachable. `cargo clean` is not the way back —
# it also deletes target/release/hawier-device (scripts/load.sh measures with
# it) and target/wasm32-wasip1 (make it loads those modules), which is exactly
# why nobody ever runs it.
#
# What is kept:
#   * everything cargo hard-linked into target/debug/ — those are the binaries
#     in use, and deleting one would not free a byte anyway;
#   * the newest generation of every unit. "Newest" is a window, not a single
#     file: 580 of 621 stems in deps/ carry two hashes written in the same
#     second, because `make ext` compiles the build scripts and proc macros of
#     a wasm build for the host and drops them right here. Keeping one of a
#     concurrent pair would force a rebuild on every collection.
#   * the ranking runs per file class (executable / rlib / rmeta / so), so a
#     leftover .d from an interrupted build cannot shadow a real 440 MB binary.
#
# What is never looked at: target/release, target/wasm32-wasip1, and
# target/e2e (the Buildroot tree of make e2e-image: its downloads and compiler
# cache are what keep a rebuild at minutes, and it is not cargo's to sweep).
#
# The worst a mistake here can do is cause a rebuild.

set -euo pipefail

# Two artifacts of the same unit belong to the same generation if they were
# written within this many seconds of each other. A full workspace build takes
# minutes; distinct generations here are hours or days apart.
GC_KEEP_GRACE=${GC_KEEP_GRACE:-3600}

# Incremental sessions to keep per crate.
GC_KEEP_INCR=${GC_KEEP_INCR:-2}

# The backstop for the one case no per-unit rule can see: a unit that no longer
# exists. Delete or rename an integration test and its last binary stays the
# newest of its stem forever. Days.
GC_MAX_AGE=${GC_MAX_AGE:-30}

dry=0
verbose=0
for arg in "$@"; do
	case "$arg" in
	--dry-run | -n) dry=1 ;;
	--verbose | -v) verbose=1 ;;
	*)
		echo "usage: $0 [--dry-run] [--verbose]" >&2
		exit 2
		;;
	esac
done

cd "$(dirname "$0")/.."
D=target/debug
[ -d "$D/deps" ] || {
	echo "gc: no $D/deps — nothing to collect"
	exit 0
}

# Cargo's own lock. Collecting under a live build would race it.
exec 9>"$D/.cargo-lock"
if ! flock -n 9; then
	echo "gc: a cargo build holds $D — not collecting" >&2
	exit 1
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

find "$D/deps" -maxdepth 1 -type f -printf '%T@\t%n\t%s\t%f\n' >"$tmp/deps"

# Decide per hash. Emits "keep|drop <hash>".
awk -F'\t' -v grace="$GC_KEEP_GRACE" -v maxage="$GC_MAX_AGE" '
	# Split "<stem>-<16 hex>[.ext]". mawk has no interval regexes, hence the
	# hand-rolled scan; stem and ext come back in globals.
	function parse(n,   i, c, dot, base, h) {
		dot = 0
		for (i = length(n); i > 0; i--) {
			c = substr(n, i, 1)
			if (c == ".") { dot = i; break }
			if (c == "-") break
		}
		base = (dot ? substr(n, 1, dot - 1) : n)
		ext  = (dot ? substr(n, dot + 1)    : "bin")
		if (length(base) < 18) return ""
		h = substr(base, length(base) - 15)
		if (length(h) != 16 || h !~ /^[0-9a-f]+$/) return ""
		if (substr(base, length(base) - 16, 1) != "-") return ""
		stem = substr(base, 1, length(base) - 17)
		return h
	}
	{
		mtime = $1 + 0; nlink = $2 + 0; size = $3 + 0; name = $4
		h = parse(name)
		if (h == "") next                      # no hash: not ours, never touch
		bytes[h] += size
		if (mtime > seen[h]) seen[h] = mtime
		if (nlink > 1) pinned[h] = 1           # uplifted into target/debug/
		if (ext == "d") next                   # dep-info follows its hash
		k = stem SUBSEP ext
		if (mtime > gen[k, h]) gen[k, h] = mtime
		if (mtime > newest[k]) newest[k] = mtime
		group[k, h] = k
		hash[k, h] = h
		if (mtime > treenewest) treenewest = mtime
	}
	END {
		cutoff = systime() - maxage * 86400
		for (x in group) {
			k = group[x]
			if (newest[k] - gen[x] <= grace) keep[hash[x]] = 1
		}
		for (h in bytes) {
			verdict = (h in keep) ? "keep" : "drop"
			# A unit nobody has built in maxage days is gone, whatever its rank.
			if (verdict == "keep" && !(h in pinned) && seen[h] < cutoff) verdict = "drop"
			if (h in pinned) verdict = "keep"
			print verdict "\t" h "\t" bytes[h]
		}
	}
' "$tmp/deps" >"$tmp/verdict"

awk -F'\t' '$1=="drop" {print $2}' "$tmp/verdict" | sort >"$tmp/drop"
awk -F'\t' '$1=="keep" {print $2}' "$tmp/verdict" | sort >"$tmp/keep"

freed=$(awk -F'\t' '$1=="drop" {s+=$3} END {printf "%.2f", s/1073741824}' "$tmp/verdict")
nkeep=$(wc -l <"$tmp/keep")
ndrop=$(wc -l <"$tmp/drop")

# Files of the dropped hashes.
awk -F'\t' 'NR==FNR {d[$1]=1; next} {
	n = $4
	for (i = length(n); i > 0; i--) if (substr(n, i, 1) == ".") { n = substr(n, 1, i - 1); break }
	h = substr(n, length(n) - 15)
	if (h in d) print $4
}' "$tmp/drop" "$tmp/deps" | sed "s|^|$D/deps/|" >"$tmp/files"

# .fingerprint and build/ directories of the dropped hashes, plus fingerprints
# whose artifacts are already gone. Cargo checks that a unit's outputs exist
# before calling it fresh, so such a fingerprint can never report "fresh" — it
# only slows the directory scan.
: >"$tmp/dirs"
for base in "$D/.fingerprint" "$D/build"; do
	[ -d "$base" ] || continue
	find "$base" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' \
		| awk -v base="$base" 'NR==FNR {d[$1]=1; next} {
			if (length($0) < 18) next
			h = substr($0, length($0) - 15)
			if (length(h) != 16 || h !~ /^[0-9a-f]+$/) next
			if (h in d) print base "/" $0
		}' "$tmp/drop" - >>"$tmp/dirs"
done
if [ -d "$D/.fingerprint" ]; then
	find "$D/.fingerprint" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' \
		| awk -v base="$D/.fingerprint" 'NR==FNR {k[$1]=1; next} {
			if (length($0) < 18) next
			h = substr($0, length($0) - 15)
			if (length(h) != 16 || h !~ /^[0-9a-f]+$/) next
			if (!(h in k)) print base "/" $0
		}' "$tmp/keep" - >>"$tmp/dirs"
fi
sort -u "$tmp/dirs" -o "$tmp/dirs"

# Incremental: keep the newest GC_KEEP_INCR sessions per crate.
: >"$tmp/incr"
if [ -d "$D/incremental" ]; then
	find "$D/incremental" -mindepth 1 -maxdepth 1 -type d -printf '%T@\t%f\n' \
		| sed -E 's/^([0-9]+)\.[0-9]+\t((.+)-[0-9a-z]+)$/\1\t\3\t\2/' \
		| sort -t"$(printf '\t')" -k2,2 -k1,1nr \
		| awk -F'\t' -v k="$GC_KEEP_INCR" -v base="$D/incremental" \
			'NF==3 && ++n[$2] > k {print base "/" $3}' >"$tmp/incr"
fi

incrfreed=$(xargs -r -a "$tmp/incr" du -sb 2>/dev/null | awk '{s+=$1} END {printf "%.2f", s/1073741824}')

echo "gc: deps    keep $nkeep units, drop $ndrop units ($freed GiB, $(wc -l <"$tmp/files") files)"
echo "gc: meta    drop $(wc -l <"$tmp/dirs") .fingerprint/build dirs"
echo "gc: incr    drop $(wc -l <"$tmp/incr") of $(find "$D/incremental" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l) sessions (${incrfreed:-0} GiB)"

if [ "$verbose" = 1 ]; then
	echo
	cat "$tmp/files" "$tmp/dirs" "$tmp/incr"
fi

if [ "$dry" = 1 ]; then
	echo
	echo "gc: --dry-run, nothing removed. Largest units that would go:"
	sort -t"$(printf '\t')" -k3,3nr "$tmp/verdict" | awk -F'\t' '$1=="drop" && n++ < 10 {printf "  %7.0f MB  %s\n", $3/1048576, $2}'
	exit 0
fi

xargs -r -a "$tmp/files" rm -f --
xargs -r -a "$tmp/dirs" rm -rf --
xargs -r -a "$tmp/incr" rm -rf --

echo "gc: done; $(find "$D/deps" -maxdepth 1 -type f | wc -l) files left in $D/deps"
