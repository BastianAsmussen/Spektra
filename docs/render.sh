#!/usr/bin/env bash

# Usage: docs/render.sh [proces|produkt]...  Renders docs/<Rapport>.pdf with its bilag appended.
set -euo pipefail

root=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
cd "$root"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

landscape() {
	local top=''
	[[ ${3:-} == first ]] && top='#heading(level: 1)[Bilag]'

	printf '```{=typst}\n#page(flipped: true)[\n%s\n#heading(level: 2)[%s]\n#image("%s", width: 100%%, height: 1fr, fit: "contain")\n]\n```\n\n' "$top" "$1" "$2"
}

proces_bilag() {
	landscape 'Bilag 1: Estimeret tidsplan' docs/figures/tidsplan.svg first
	landscape 'Bilag 2: Realiseret tidsplan' docs/figures/realiseret.svg

	echo '## Bilag 3: Projektdagbog'
	echo

	sed '1{/^# /d}' docs/LOGBOG.md
}

produkt_bilag() {
	landscape 'Bilag 1: Overordnet arkitektur' docs/figures/arkitektur.svg first
	landscape 'Bilag 2: Databasediagram' docs/figures/database.svg

	echo '## Bilag 3: Skærmbilleder af webklienten'
	echo

	shopt -s nullglob
	shots=(docs/figures/screenshots/*.png)
	if ((${#shots[@]} == 0)); then
		echo '[TJEK: ingen skærmbilleder i docs/figures/screenshots/]'
		echo
	fi

	for s in "${shots[@]}"; do
		printf '![%s](%s)\n\n' "$(basename "$s" .png)" "$s"
	done

	echo '## Bilag 4: Protokolskema'
	echo

	for p in protocol/proto/v1/*.proto; do
		printf '### `%s`\n\n```protobuf\n' "$p"
		cat "$p"
		printf '```\n\n'
	done
}

render() {
	local name=$1 gen=$2
	"$gen" >"$tmp/$name-bilag.md"

	nix-shell -p pandoc typst --run "pandoc -s -f markdown -t pdf --pdf-engine=typst \
    --toc -V mainfont='Libertinus Serif' -V monofont='DejaVu Sans Mono' \
    -V margin-x=2.5cm -V margin-y=2.5cm \
    -V header-includes='#show figure: set block(breakable: true)' \
    -V header-includes='#show table: set par(justify: false)' \
    -V header-includes='#show table: set text(hyphenate: true)' \
    -V header-includes='#show heading: set block(above: 2.4em)' \
    -V header-includes='#show \"->\": box' \
    -o docs/$name.pdf docs/$name.md $tmp/$name-bilag.md"

	echo "docs/$name.pdf"
}

for r in "${@:-proces produkt}"; do
	for x in $r; do
		case $x in
		proces) render Processrapport proces_bilag ;;
		produkt) render Produktrapport produkt_bilag ;;
		*)
			echo "unknown rapport: $x" >&2
			exit 2
			;;
		esac
	done
done
