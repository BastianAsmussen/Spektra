#!/usr/bin/env bash

set -euo pipefail

SERVER="http://localhost:50051"
API="http://localhost:8080"
SESSION="${SPEKTRA_SESSION:-}"
NUM_NODES=100
EXISTING=0
JOBS=32
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$DIR/../target/release/emulator"

usage() {
	cat >&2 <<-EOF
		Usage: $0 [-n NUM_NODES] [-e] [-j JOBS] [-s SERVER] [-a API] [-k SESSION] [-- EMULATOR ARGS...]

		Registration is authenticated, so each node is planned through the
		administration API first and enrols against the key that returns. -k is
		an administrator's session_token, or set SPEKTRA_SESSION.

		-e drives the nodes already in the directory before planning any new
		ones, so a rerun tops the fleet up to -n instead of doubling it. Each
		reused node's credential is rotated, which revokes whatever the real
		receiver at that site holds.

		-j sets how many nodes are prepared through the API at once, 32 by
		default.

		Both -s and -a point at the deployment; -a defaults to localhost and
		is easy to forget when only -s is overridden:

		  $0 -n 5 -s https://spektra.asmussen.tech -a https://spektra.asmussen.tech

		Anything after -- goes to every emulator, so the fleet's shape is set
		there rather than duplicated here:

		  $0 -n 20 -- --backfill-hours 168 --channels 4   a week of history
		  $0 -n 100 -- --once                             register and exit
		  $0 -n 5 -- --backfill-hours 0                   live only, no history
	EOF
	exit 1
}

PASSTHROUGH=()

while [[ $# -gt 0 ]]; do
	case "$1" in
	-n | --num)
		NUM_NODES="$2"
		shift 2
		;;
	-e | --existing)
		EXISTING=1
		shift
		;;
	-j | --jobs)
		JOBS="$2"
		shift 2
		;;
	-s | --server)
		SERVER="$2"
		shift 2
		;;
	-a | --api)
		API="$2"
		shift 2
		;;
	-k | --session)
		SESSION="$2"
		shift 2
		;;
	--)
		shift
		PASSTHROUGH=("$@")
		break
		;;
	*)
		usage
		;;
	esac
done

if [[ ! -x "$BIN" ]]; then
	echo "emulator binary not found at $BIN; run 'cargo build --release -p emulator' first" >&2
	exit 1
fi

if [[ -z "$SESSION" ]]; then
	echo "an administrator session token is required: pass -k or set SPEKTRA_SESSION" >&2
	exit 1
fi

plan_node() {
	local name=$1 lat=$2 lon=$3

	curl -fsS -X POST "$API/api/nodes" \
		-H "authorization: Bearer $SESSION" \
		-H "content-type: application/json" \
		-d "{\"name\":\"$name\",\"latitude\":$lat,\"longitude\":$lon}" |
		grep -o '"credential":"[^"]*"' | cut -d'"' -f4
}

random_identity() {
	od -An -N16 -tx1 /dev/urandom | tr -d ' \n'
}

existing_nodes() {
	curl -fsS "$API/api/nodes" -H "authorization: Bearer $SESSION" |
		jq -r --argjson n "$NUM_NODES" '
			[.[] | select(.suspended | not)][:$n][]
			| [.id, (.external_identity // ""), .name,
			   (.latitude // 57.05), (.longitude // 9.92)]
			| @tsv'
}

rotate_credential() {
	curl -fsS -X POST "$API/api/nodes/$1/credential" \
		-H "authorization: Bearer $SESSION" |
		jq -r '.credential'
}

prepare_node() {
	local kind key name lat lon identity token

	IFS=$'\t' read -r kind key name lat lon identity <<<"$1"

	if [[ $kind == reuse ]]; then
		token=$(rotate_credential "$key" 2>/dev/null) || token=""
	else
		token=$(plan_node "$name" "$lat" "$lon" 2>/dev/null) || token=""
	fi

	if [[ -z $token ]]; then
		printf 'could not %s %s through %s\n' "$kind" "$name" "$API" >&2

		return 1
	fi

	printf '%s\t%s\t%s\t%s\t%s\n' "$identity" "$name" "$lat" "$lon" "$token"
}

SITES=(
	"57.048 9.921"  # Aalborg
	"56.162 10.204" # Aarhus
	"55.396 10.389" # Odense
	"55.676 12.569" # København
	"55.471 8.452"  # Esbjerg
	"57.442 10.537" # Frederikshavn
	"56.462 9.402"  # Viborg
	"55.708 9.536"  # Vejle
	"55.860 9.850"  # Horsens
	"56.360 8.616"  # Holstebro
	"55.229 11.761" # Næstved
	"54.769 11.874" # Nykøbing Falster
	"55.100 14.700" # Rønne
	"56.951 8.694"  # Thisted
	"55.491 9.472"  # Kolding
	"55.860 12.035" # Hillerød
)

readonly LAT_MIN=54.56 LAT_MAX=57.75
readonly LON_MIN=8.07 LON_MAX=15.20

readonly SPREAD_DEG=0.30

site_for() {
	local index=$1 total=$2
	local site=${SITES[$((index % ${#SITES[@]}))]}
	local ring=$((index / ${#SITES[@]}))
	local rings=$(((total - 1) / ${#SITES[@]}))

	read -r lat lon <<<"$site"
	awk -v lat="$lat" -v lon="$lon" -v ring="$ring" -v rings="$rings" -v n="$index" \
		-v spread="$SPREAD_DEG" -v latmin="$LAT_MIN" -v latmax="$LAT_MAX" \
		-v lonmin="$LON_MIN" -v lonmax="$LON_MAX" '
		function clamp(v, lo, hi) { return v < lo ? lo : (v > hi ? hi : v) }
		BEGIN {
			if (ring == 0 || rings == 0) { printf "%.5f %.5f\n", lat, lon; exit }
			angle = n * 2.399963
			radius = spread * sqrt(ring / rings)
			printf "%.5f %.5f\n",
				clamp(lat + radius * cos(angle), latmin, latmax),
				clamp(lon + (radius * sin(angle)) / 0.56, lonmin, lonmax)
		}'
}

declare -A WORDS=(
	[b]="barsk blid bred brun brat bitter|bæver bakke bølge birk brise bogfink"
	[d]="dyb dristig doven dunkel dyster drøj|drossel dal due drage dyne dæmning"
	[f]="fin flink fri frisk fast fattig|fyr falk fjord fasan flod fugl"
	[g]="glad grøn grå grov gammel gæv|gedde grævling gøg granit gran gås"
	[h]="hurtig høj hård hvid hul herlig|hejre hare havn hassel hugorm høg"
	[k]="klog kold kort kraftig kvik køn|krage kilde klit kløver kvist kyst"
	[l]="lang let livlig lav lys lun|lærke lyng laks lind lygte løve"
	[m]="mild mørk munter modig mager mæt|mose måge mus mark mølle morgen"
	[n]="nem ny nordlig nænsom nøgen nyttig|natugle nælde nød natravn nattergal nøgle"
	[r]="rank rap rolig rund rusten rar|ravn rede rype rose ræv rist"
	[s]="snild stærk stille sort smal sikker|spurv sten sø slette sump stær"
	[t]="tam tavs tyk tør træt tapper|tjørn tundra tue torn trane tudse"
	[u]="ung uklar urolig usikker udsat uskyldig|ugle ulv urt udsigt uge urfugl"
	[v]="vild varm våd vis venlig vågen|vibe vig vinge vinter vase vej"
)

cycle() {
	local count=$1 emitted=0 round=1
	local -a pool

	mapfile -t pool

	while ((emitted < count)); do
		for name in "${pool[@]}"; do
			((emitted < count)) || break

			if ((round == 1)); then
				echo "$name"
			else
				echo "$name-$round"
			fi

			emitted=$((emitted + 1))
		done

		round=$((round + 1))
	done
}

name_pool() {
	local letter adjectives nouns adj noun

	for letter in "${!WORDS[@]}"; do
		IFS='|' read -r adjectives nouns <<<"${WORDS[$letter]}"

		for adj in $adjectives; do
			for noun in $nouns; do
				echo "$adj-$noun"
			done
		done
	done
}

ROWS=()

if ((EXISTING)); then
	mapfile -t ROWS < <(existing_nodes)
fi

FRESH=$((NUM_NODES - ${#ROWS[@]}))

if ((FRESH > 0)); then
	mapfile -t NAMES < <(name_pool | shuf | cycle "$FRESH")
fi

pids=()

cleanup() {
	trap - INT TERM EXIT
	[[ ${#pids[@]} -gt 0 ]] && kill "${pids[@]}" 2>/dev/null
	wait 2>/dev/null
}
trap cleanup INT TERM EXIT

WORK="$(mktemp)"
TABLE="$(mktemp)"
trap 'rm -f "$WORK" "$TABLE"' EXIT

for ((i = 1; i <= NUM_NODES; i++)); do
	if ((i <= ${#ROWS[@]})); then
		IFS=$'\t' read -r ID IDENTITY NAME LAT LON <<<"${ROWS[$((i - 1))]}"
		[[ -n $IDENTITY ]] || IDENTITY="$(random_identity)"

		printf 'reuse\t%s\t%s\t%s\t%s\t%s\n' "$ID" "$NAME" "$LAT" "$LON" "$IDENTITY"
	else
		NAME="${NAMES[$((i - 1 - ${#ROWS[@]}))]}"
		read -r LAT LON <<<"$(site_for "$((i - 1))" "$NUM_NODES")"

		printf 'plan\t-\t%s\t%s\t%s\t%s\n' "$NAME" "$LAT" "$LON" "$(random_identity)"
	fi
done >"$WORK"

export -f prepare_node plan_node rotate_credential
export API SESSION

xargs -d '\n' -n 1 -P "$JOBS" -a "$WORK" \
	bash -c 'for line; do prepare_node "$line"; done' _ >"$TABLE"

PLANNED=$(wc -l <"$TABLE")
if ((PLANNED < NUM_NODES)); then
	echo "only $PLANNED of $NUM_NODES nodes could be prepared through $API" >&2
	exit 1
fi

while IFS=$'\t' read -r IDENTITY NAME LAT LON TOKEN; do
	"$BIN" --server "$SERVER" --identity "$IDENTITY" --name "$NAME" \
		--enrollment-token "$TOKEN" \
		--latitude "$LAT" --longitude "$LON" "${PASSTHROUGH[@]}" &
	pids+=("$!")
done <"$TABLE"

echo "Started $NUM_NODES nodes against $SERVER."

fail=0
for pid in "${pids[@]}"; do
	if ! wait "$pid"; then
		fail=$((fail + 1))
	fi
done

trap - INT TERM EXIT
echo "Done: $((NUM_NODES - fail))/$NUM_NODES nodes finished cleanly."
