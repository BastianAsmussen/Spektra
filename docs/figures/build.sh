#!/usr/bin/env bash
set -euo pipefail

dir=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fonts=$(nix-build '<nixpkgs>' -A libertinus --no-out-link)/share/fonts/truetype

retext() {
  sed -E \
    -e 's/font-family: *"?d2-[0-9]+-font-bold"?/font-family: "Libertinus Serif"; font-weight: bold/g' \
    -e 's/font-family: *"?d2-[0-9]+-font-(regular|italic)"?/font-family: "Libertinus Serif"/g' \
    "$1" | perl -0pe 's/\@font-face\s*\{[^}]*\}//g' >"$2"
}

# Only the gantt subset the .mmd files use: sections, and `label :[tags,] id, YYYY-MM-DD, Nd`.
gantt_json() {
  perl -MJSON::PP -ne '
    BEGIN { $d = { sections => [] } }
    if (/^\s*section\s+(.+?)\s*$/) { push @{$d->{sections}}, { name => $1, tasks => [] }; next }
    next unless /^\s*(.+?)\s+:(.+)$/ and @{$d->{sections}};
    ($label, @f) = ($1, map { s/^\s+|\s+$//gr } split /,/, $2);
    ($days) = pop(@f) =~ /(\d+)d/; $start = pop @f; pop @f;
    push @{$d->{sections}[-1]{tasks}}, { label => $label, tags => [@f], start => $start, days => $days + 0 };
    END { print JSON::PP->new->utf8(0)->encode($d) }
  '
}

build() {
  local name=$1
  if [[ -f $dir/$name.d2 ]]; then
    nix-shell -p d2 --run "d2 --font-regular $fonts/LibertinusSerif-Regular.ttf \
      --font-bold $fonts/LibertinusSerif-Bold.ttf \
      $dir/$name.d2 $tmp/$name.svg" >&2
    retext "$tmp/$name.svg" "$dir/$name.svg"
  else
    gantt_json <"$dir/$name.mmd" >"$tmp/$name.json"
    cp "$dir/gantt.typ" "$tmp/"
    nix-shell -p typst --run "typst compile --root $tmp --format svg \
      --input data=/$name.json $tmp/gantt.typ $dir/$name.svg" >&2
  fi
  echo "docs/figures/$name.svg"
}

for n in "${@:-arkitektur database tidsplan realiseret}"; do
  for x in $n; do build "$x"; done
done
