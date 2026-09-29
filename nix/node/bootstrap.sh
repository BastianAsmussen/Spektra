#!/usr/bin/env bash
set -euo pipefail

image=/radio-node-1.img.zst
token=/enrollment.env
wifi=/wifi.conf
backup=/root/agent-state
wifi_backup=/root/wifi.conf
card=/dev/mmcblk0
state=var/lib/spektra-node-agent
networks=etc/wpa_supplicant/imperative.conf

die() {
  echo "bootstrap: $*" >&2
  exit 1
}

if ((EUID != 0)); then
  exec sudo -- bash "$0" "$@"
fi

[[ -b $card ]] || die "$card not found, insert the SD card"
[[ -f $image ]] || die "$image not found"
[[ $(findmnt -no SOURCE /) != "$card"* ]] || die "running from $card, boot from the stick instead"

mnt=$(mktemp -d)
trap 'umount "$mnt" 2>/dev/null || true; rmdir "$mnt"' EXIT

lsblk -rno MOUNTPOINT "$card" | while read -r target; do
  if [[ -n $target ]]; then
    umount "$target"
  fi
done

saved=false
if mount -o ro "${card}p2" "$mnt" 2>/dev/null; then
  if [[ -f $mnt/$state/identity.json ]]; then
    rm -rf "$backup"
    cp -a "$mnt/$state" "$backup"
    saved=true
    echo "bootstrap: saved the node identity to $backup"
  fi
  if [[ ! -f $wifi && -s $mnt/$networks ]]; then
    install -m 600 "$mnt/$networks" "$wifi_backup"
    echo "bootstrap: saved the card's wireless networks to $wifi_backup"
  fi
  umount "$mnt"
fi

if [[ -f $wifi ]]; then
  networks_from=$wifi
elif [[ -s $wifi_backup ]]; then
  networks_from=$wifi_backup
else
  networks_from=
fi

if ! $saved && [[ -f $backup/identity.json ]]; then
  read -rp "bootstrap: the card holds no identity, restore $(grep -oE '"node_id": [0-9]+' "$backup/identity.json") from an earlier run? [y/N] " answer
  if [[ $answer == [yY] ]]; then
    saved=true
  fi
fi

if $saved; then
  restore=identity
elif [[ -f $token ]]; then
  restore=token
else
  die "no identity on the card and no $token, so the node could never register"
fi

zstd -dc "$image" | dd of="$card" bs=4M conv=fsync status=progress
blockdev --rereadpt "$card"
udevadm settle

mount "${card}p2" "$mnt"
install -d "$mnt/var/lib"
case $restore in
  identity) cp -a "$backup" "$mnt/$state" ;;
  token)
    install -d -m 700 "$mnt/$state"
    install -m 600 "$token" "$mnt/$state/"
    ;;
esac
if [[ -n $networks_from ]]; then
  install -D -m 600 "$networks_from" "$mnt/$networks"
  echo "bootstrap: wireless networks from $networks_from"
else
  echo "bootstrap: no wireless networks, the node needs a cable"
fi
umount "$mnt"
sync

echo "bootstrap: done, restored the $restore. Power off, pull the stick and power on."
