#!/usr/bin/env bash
# Relative links and images in README.md and docs/**/*.md must point at
# something in the tree (spec: repository-docs, "Links in the documentation
# resolve"). Called by `make lint`.
#
# Targets are taken from Markdown `](target)` and from HTML `src=`/`srcset=`.
# Absolute URLs, mailto: and pure #anchors are skipped; a #fragment after a
# path is dropped (headings are not checked). A target is resolved against
# the directory of the file it is written in. This is a grep, not a Markdown
# parser: a link it misreads is fixed by writing the link plainly.
set -euo pipefail

cd "$(dirname "$0")/.."

broken=0
while IFS= read -r file; do
	dir=$(dirname "$file")
	while IFS=: read -r line target; do
		case "$target" in
			http://* | https://* | mailto:* | '#'* | '') continue ;;
		esac
		path=${target%%#*}
		if [ ! -e "$dir/$path" ]; then
			echo "$file:$line: $target"
			broken=1
		fi
	done < <(grep -noE '\]\([^) ]+\)|(src|srcset)="[^"]+"' "$file" \
		| sed -E 's/^([0-9]+):(\]\(|(src|srcset)=")/\1:/; s/[)"]$//')
done < <(git ls-files --cached --others --exclude-standard -- README.md 'docs/*.md' 'docs/**/*.md')

if [ "$broken" -ne 0 ]; then
	echo "broken links (above): each target must exist relative to its file" >&2
	exit 1
fi
