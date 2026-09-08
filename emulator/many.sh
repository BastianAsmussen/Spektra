#!/usr/bin/env bash

set -euo pipefail

SERVER="http://localhost:50051"
API="http://localhost:8080"
SESSION="${SPEKTRA_SESSION:-}"
NUM_NODES=100
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN="$DIR/../target/release/emulator"

usage() {
	cat >&2 <<-EOF
		Usage: $0 [-n NUM_NODES] [-s SERVER] [-a API] [-k SESSION] [-- EMULATOR ARGS...]

		Registration is authenticated, so each node is planned through the
		administration API first and enrols against the key that returns. -k is
		an administrator's session_token, or set SPEKTRA_SESSION.

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

SITES=(
	"57.048 9.921"    # Aalborg
	"56.162 10.204"   # Aarhus
	"55.396 10.389"   # Odense
	"55.676 12.569"   # København
	"55.471 8.452"    # Esbjerg
	"57.442 10.537"   # Frederikshavn
	"56.462 9.402"    # Viborg
	"55.708 9.536"    # Vejle
	"55.860 9.850"    # Horsens
	"56.360 8.616"    # Holstebro
	"55.229 11.761"   # Næstved
	"54.769 11.874"   # Nykøbing Falster
	"55.100 14.700"   # Rønne
	"56.951 8.694"    # Thisted
	"55.491 9.472"    # Kolding
	"55.860 12.035"   # Hillerød
)

site_for() {
	local index=$1
	local site=${SITES[$((index % ${#SITES[@]}))]}
	local ring=$((index / ${#SITES[@]}))

	read -r lat lon <<<"$site"
	awk -v lat="$lat" -v lon="$lon" -v ring="$ring" -v n="$index" 'BEGIN {
		if (ring == 0) { printf "%.5f %.5f\n", lat, lon; exit }
		angle = n * 2.399963
		radius = 0.045 * ring
		printf "%.5f %.5f\n", lat + radius * cos(angle), lon + (radius * sin(angle)) / 0.56
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

mapfile -t NAMES < <(name_pool | shuf -n "$NUM_NODES")

if [[ ${#NAMES[@]} -lt $NUM_NODES ]]; then
	echo "name pool holds only ${#NAMES[@]} names; -n $NUM_NODES is too many" >&2
	exit 1
fi

pids=()

cleanup() {
	trap - INT TERM EXIT
	[[ ${#pids[@]} -gt 0 ]] && kill "${pids[@]}" 2>/dev/null
	wait 2>/dev/null
}
trap cleanup INT TERM EXIT

for ((i = 1; i <= NUM_NODES; i++)); do
	IDENTITY="$(random_identity)"
	NAME="${NAMES[$((i - 1))]}"
	read -r LAT LON <<<"$(site_for "$((i - 1))")"

	if ! TOKEN="$(plan_node "$NAME" "$LAT" "$LON")" || [[ -z "$TOKEN" ]]; then
		echo "could not plan $NAME through $API" >&2
		exit 1
	fi

	"$BIN" --server "$SERVER" --identity "$IDENTITY" --name "$NAME" \
		--enrollment-token "$TOKEN" \
		--latitude "$LAT" --longitude "$LON" "${PASSTHROUGH[@]}" &
	pids+=("$!")
done

echo "Started $NUM_NODES nodes against $SERVER."

fail=0
for pid in "${pids[@]}"; do
	if ! wait "$pid"; then
		fail=$((fail + 1))
	fi
done

trap - INT TERM EXIT
echo "Done: $((NUM_NODES - fail))/$NUM_NODES nodes finished cleanly."
