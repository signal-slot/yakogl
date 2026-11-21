#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# imx95-sdcard.sh -- the Verdin iMX95's microSD card for the panthor Linux
# route (docs/notes/imx95-linux-panthor.md, option (iii) and section 5):
# mainline Linux v7.3 with its own Mallow device trees, a Debian trixie arm64
# rootfs, the Mali CSF firmware at arm/mali/arch10.12, a U-Boot script that
# Toradex's bootstd finds on mmc1. Built on the host, without root: no sudo,
# no loop mount, no user namespace (Ubuntu's AppArmor refuses unprivileged
# ones), no binfmt_misc.
#
#   BOARD_SSH_PUBKEY=<file.pub> tools/mali/panthor/imx95-sdcard.sh [--out DIR] [STEP...]
#
# STEPs, in order (default: all four):
#   kernel     v7.3 arm64 defconfig + imx95-sdcard.config (checked line by
#              line in the final .config), Image, modules, the
#              imx95-verdin-{wifi,nonwifi}-mallow dtbs; out/kernel/
#   rootfs     debootstrap's first stage of trixie under fakeroot, the
#              modules, the firmware, the configuration files (below), an
#              ext4 image of it (mke2fs -d), then debootstrap's second stage
#              and the configuration commands inside qemu-system-aarch64 on
#              the generic virt machine, booting the kernel above with that
#              image as its root; out/root.ext4
#   card       the FAT boot partition (Image, both dtbs, boot.scr and its
#              source, the kernel's config; mtools, no mount) and the card
#              image: MBR, p1 FAT32 bootable 256 MiB, p2 the rootfs;
#              out/card.img (sparse), out/card.img.sha256
#   verify     the card on the host: the partition table, the FAT file list,
#              the boot script, the kernel config, the dtbs, panthor.ko, the
#              firmware's sha256 against the MALI_CSFFW pin, the ssh key,
#              e2fsck; out/verify.txt
#
# The second stage's boot proves the kernel and the rootfs's packages on
# qemu's virt machine; a boot of the finished image to a login is not part of
# this script yet.
#
# Inputs, all pinned in tools/cts/deps.toml (board verdin_imx95; make them
# with `tools/cts/deps-env.sh --build --board verdin_imx95 IMX95_LINUX_SRC
# IMX95_DEBIAN_TOOLS IMX95_QEMU_SYSTEM MALI_CSFFW` and `tools/cts/deps-env.sh
# --build --board nanopc_t6 MALI_DEQP_TOOLCHAIN`): the kernel tarball,
# debootstrap + the Debian archive keyring + mtools, qemu-system-arm, the
# firmware, Debian trixie's arm64 cross gcc. The rootfs's packages come from snapshot.debian.org at
# SNAPSHOT (below), checked by debootstrap against the pinned keyring.
#
# Environment:
#   BOARD_SSH_PUBKEY   the public key root's authorized_keys gets (required
#                      for rootfs; never a file of the tree)
#   CROSS_COMPILE      the arm64 compiler prefix (default: Debian trixie's
#                      aarch64-linux-gnu- gcc 14 of MALI_DEQP_TOOLCHAIN,
#                      tools/cts/deps.toml, board nanopc_t6)
#   IMX95_SD_OUT       the output directory (default build/imx95-sdcard of
#                      the checkout, gitignored; --out wins). The card image
#                      is 6 GiB sparse, about 1 GiB used
#   IMX95_SD_CONSOLE_AUTOLOGIN  1 (default): root is logged in on the
#                      board's console UART (ttyLP2) without a password, the
#                      way in when the network does not come up; 0: a plain
#                      getty (root has no password, so no console login)
#   JOBS               make -j (default: nproc)
#
# Nothing here writes a card or names a device: writing out/card.img to a
# card is the owner's (docs/notes/imx95-linux-panthor.md section 5).
# The kernel source tree is read only (O= build directory).
set -euo pipefail
me=${0##*/}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)
PY="python3 -I $HERE/imx95_sdcard.py"
FRAGMENT=$HERE/imx95-sdcard.config
DEPS_TOML=$ROOT/tools/cts/deps.toml

KVER=7.3.0
SNAPSHOT=20261101T000000Z
MIRROR=https://snapshot.debian.org/archive/debian/$SNAPSHOT
DISK_ID=0x1a95c0de # the card's MBR id: root=PARTUUID=1a95c0de-02
PARTUUID=1a95c0de
BOOT_MIB=256
TOTAL_MIB=6144 # fits an 8 GB card
DTBS="imx95-verdin-wifi-mallow imx95-verdin-nonwifi-mallow"
CONSOLE=ttyLP2 # the board's console: Verdin UART_3 = LPUART1 = serial2
PACKAGES=systemd-sysv,udev,dbus,systemd-resolved,systemd-timesyncd,openssh-server,sudo,kmod,iproute2,iputils-ping,psmisc,procps,i2c-tools,kexec-tools,python3,busybox,xz-utils,zstd,libubootenv-tool,libstdc++6,less,ca-certificates
export SOURCE_DATE_EPOCH=1793491200 # 2026-11-01T00:00:00Z, the snapshot's day
export KBUILD_BUILD_TIMESTAMP="@$SOURCE_DATE_EPOCH" KBUILD_BUILD_USER=yakogl KBUILD_BUILD_HOST=imx95-sdcard

out=${IMX95_SD_OUT:-$ROOT/build/imx95-sdcard}
steps=() overlay=
while [ $# -gt 0 ]; do
  case $1 in
    --out) out=${2:?}; shift 2 ;;
    -h | --help) sed -n '5,/^set -euo/p' "$0" | sed '$d'; exit 0 ;;
    kernel | rootfs | card | verify) steps+=("$1"); shift ;;
    --write-overlay) overlay=${2:?}; shift 2 ;; # internal: rootfs's own call under fakeroot
    *) echo "$me: unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ ${#steps[@]} -gt 0 ] || steps=(kernel rootfs card verify)
mkdir -p "$out"
out=$(cd "$out" && pwd)
log() { echo "$me: $*" >&2; }
die() { echo "$me: $*" >&2; exit 1; }

# The pinned inputs, from the dependency layout.
eval "$("$ROOT/tools/cts/deps-env.sh" --present verdin_imx95 | grep -E '^(IMX95_LINUX_SRC|IMX95_DEBIAN_TOOLS|IMX95_QEMU_SYSTEM|MALI_CSFFW)=' | sed 's/^/export /')"
need() { # <variable>: an entry of the layout
  [ -n "${!1:-}" ] || die "$1 is not in the dependency layout: tools/cts/deps-env.sh --build --board verdin_imx95 $1"
}

KB=$out/kernel-build
cross() { # the compiler prefix: CROSS_COMPILE, else Debian trixie's cross gcc
  # (MALI_DEQP_TOOLCHAIN, pinned for the NanoPC-T6's deqp build) through
  # wrappers in $out/cross that give each tool its own host libraries. The
  # Zephyr SDK's aarch64-zephyr-elf- does not do: its gcc has no stdint.h,
  # which the kernel's NEON code (arm_neon.h, -ffreestanding) includes.
  if [ -n "${CROSS_COMPILE:-}" ]; then
    [ -x "${CROSS_COMPILE}gcc" ] || die "no compiler ${CROSS_COMPILE}gcc"
    echo "$CROSS_COMPILE"
    return
  fi
  local t f n
  t=$("$ROOT/tools/cts/deps-env.sh" --present nanopc_t6 | sed -n 's/^MALI_DEQP_TOOLCHAIN=//p')
  [ -n "$t" ] || die "no cross compiler: tools/cts/deps-env.sh --build --board nanopc_t6 MALI_DEQP_TOOLCHAIN, or set CROSS_COMPILE"
  mkdir -p "$out/cross"
  for f in "$t"/usr/bin/aarch64-linux-gnu-*; do
    n=${f##*/}
    n=${n%-14}
    printf '#!/bin/sh\nLD_LIBRARY_PATH="%s/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" exec "%s" "$@"\n' "$t" "$f" > "$out/cross/$n"
    chmod +x "$out/cross/$n"
  done
  echo "$out/cross/aarch64-linux-gnu-"
}
step_kernel() {
  need IMX95_LINUX_SRC
  local cc
  cc=$(cross)
  local mk=(make -C "$IMX95_LINUX_SRC" O="$KB" ARCH=arm64 CROSS_COMPILE="$cc" -j"${JOBS:-$(nproc)}")
  mkdir -p "$KB"
  log "kernel: defconfig + $(basename "$FRAGMENT") with ${cc}gcc"
  "${mk[@]}" -s defconfig
  "$IMX95_LINUX_SRC/scripts/kconfig/merge_config.sh" -m -O "$KB" "$KB/.config" "$FRAGMENT" > "$KB/merge.log"
  "${mk[@]}" -s olddefconfig
  $PY config-check "$FRAGMENT" "$KB/.config" || die "kernel: the .config lost lines of $(basename "$FRAGMENT")"
  log "kernel: Image modules dtbs (log: $KB/build.log)"
  "${mk[@]}" LOCALVERSION= Image modules $(for d in $DTBS; do echo "freescale/$d.dtb"; done) > "$KB/build.log" 2>&1 ||
    { tail -30 "$KB/build.log" >&2; die "kernel: build failed"; }
  [ "$("${mk[@]}" -s LOCALVERSION= kernelrelease)" = "$KVER" ] || die "kernel: release is not $KVER"
  rm -rf "$out/kernel"
  mkdir -p "$out/kernel/dtbs"
  cp "$KB/arch/arm64/boot/Image" "$KB/System.map" "$out/kernel/"
  cp "$KB/.config" "$out/kernel/config-$KVER"
  for d in $DTBS; do cp "$KB/arch/arm64/boot/dts/freescale/$d.dtb" "$out/kernel/dtbs/"; done
  log "kernel: $KVER in $out/kernel"
}

qemu_virt() { # <root image> <log> <append> [qemu args...]: boot the card's kernel on virt
  local img=$1 qlog=$2 append=$3
  shift 3
  "$IMX95_QEMU_SYSTEM" -M virt -cpu max -smp 4 -m 2048 -nographic -audio none -no-reboot \
    -kernel "$out/kernel/Image" -append "$append" \
    -drive "file=$img,format=raw,if=virtio" "$@" < /dev/null > "$qlog" 2>&1
}

# The configuration files written on the host into the first stage's tree.
write_overlay() { # <rootfs dir> (inside the fakeroot session)
  local r=$1
  echo verdin-imx95 > "$r/etc/hostname"
  printf '127.0.0.1\tlocalhost\n127.0.1.1\tverdin-imx95\n::1\t\tlocalhost ip6-localhost ip6-loopback\n' > "$r/etc/hosts"
  cat > "$r/etc/fstab" << EOF
# the card: p2 the root, p1 the FAT boot partition (Image, dtbs, boot.scr)
PARTUUID=$PARTUUID-02 / ext4 defaults,noatime 0 1
PARTUUID=$PARTUUID-01 /boot vfat defaults,nofail,x-systemd.device-timeout=10s,umask=0077 0 2
EOF
  mkdir -p "$r/etc/systemd/network"
  printf '[Match]\nType=ether\nName=en* eth*\n\n[Network]\nDHCP=yes\n' > "$r/etc/systemd/network/20-wired.network"
  mkdir -p "$r/etc/ssh/sshd_config.d"
  printf 'PasswordAuthentication no\nKbdInteractiveAuthentication no\nPermitRootLogin prohibit-password\n' > "$r/etc/ssh/sshd_config.d/10-imx95-sdcard.conf"
  # Host keys are made on the board's first boot, not baked into the image.
  mkdir -p "$r/etc/systemd/system/ssh.service.d"
  printf '[Service]\nExecStartPre=\nExecStartPre=/usr/bin/ssh-keygen -A\nExecStartPre=/usr/sbin/sshd -t\n' > "$r/etc/systemd/system/ssh.service.d/10-host-keys.conf"
  if [ "${IMX95_SD_CONSOLE_AUTOLOGIN:-1}" = 1 ]; then
    local getty=serial-getty@$CONSOLE.service
    mkdir -p "$r/etc/systemd/system/$getty.d"
    printf '[Service]\nExecStart=\nExecStart=-/sbin/agetty --autologin root --keep-baud 115200,57600,38400,9600 - $TERM\n' \
      > "$r/etc/systemd/system/$getty.d/10-autologin.conf"
  fi
  mkdir -p "$r/root/.ssh"
  chmod 700 "$r/root/.ssh"
  cp "$BOARD_SSH_PUBKEY" "$r/root/.ssh/authorized_keys"
  chmod 600 "$r/root/.ssh/authorized_keys"
  mkdir -p "$r/usr/lib/firmware/arm/mali/arch10.12"
  cp "$MALI_CSFFW" "$r/usr/lib/firmware/arm/mali/arch10.12/mali_csffw.bin"
  chmod 644 "$r/usr/lib/firmware/arm/mali/arch10.12/mali_csffw.bin"
  cat > "$r/yakogl-stage2" << 'EOF'
#!/bin/sh
# imx95-sdcard.sh's second stage, PID 1 on qemu's virt machine: debootstrap's
# second stage, then the configuration that needs the target's own commands.
export PATH=/usr/sbin:/usr/bin:/sbin:/bin LANG=C.UTF-8
CONSOLE=ttyLP2 # the board's console UART (Verdin UART_3, LPUART1)
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev 2> /dev/null
echo 1 > /proc/sys/kernel/sysrq
done_() { sync; echo "yakogl-stage2: $1"; echo o > /proc/sysrq-trigger; sleep 60; }
/debootstrap/debootstrap --second-stage || done_ "FAILED: debootstrap --second-stage"
cat > /etc/apt/sources.list << 'LIST'
deb http://deb.debian.org/debian trixie main
deb http://deb.debian.org/debian trixie-updates main
deb http://deb.debian.org/debian-security trixie-security main
LIST
systemctl enable systemd-networkd systemd-resolved systemd-timesyncd ssh "serial-getty@$CONSOLE.service" || done_ "FAILED: systemctl enable"
passwd -l root > /dev/null || done_ "FAILED: passwd -l root"
ln -sf /run/systemd/resolve/stub-resolv.conf /etc/resolv.conf
depmod -a KVER || done_ "FAILED: depmod"
rm -f /etc/ssh/ssh_host_*
: > /etc/machine-id
rm -f /var/lib/dbus/machine-id
apt-get clean
rm -rf /var/lib/apt/lists/* /var/log/*.log /yakogl-stage2.tmp
rm -f /yakogl-stage2
done_ OK
EOF
  sed -i "s/depmod -a KVER/depmod -a $KVER/" "$r/yakogl-stage2"
  chmod 755 "$r/yakogl-stage2"
}

step_rootfs() {
  need IMX95_DEBIAN_TOOLS
  need IMX95_QEMU_SYSTEM
  need MALI_CSFFW
  [ -n "${BOARD_SSH_PUBKEY:-}" ] && [ -f "$BOARD_SSH_PUBKEY" ] || die "rootfs: set BOARD_SSH_PUBKEY to root's public key file"
  grep -qE '^(ssh-|ecdsa-|sk-)' "$BOARD_SSH_PUBKEY" || die "rootfs: $BOARD_SSH_PUBKEY is not an OpenSSH public key"
  [ -f "$out/kernel/Image" ] || die "rootfs: no kernel (step kernel first)"
  local w=$out/rootfs-work r=$out/rootfs-work/root state=$out/rootfs-work/fakeroot.state
  rm -rf "$w"
  mkdir -p "$w"
  : > "$state"
  log "rootfs: debootstrap --foreign trixie arm64 from $MIRROR (log: $w/debootstrap.log)"
  DEBOOTSTRAP_DIR=$IMX95_DEBIAN_TOOLS/usr/share/debootstrap fakeroot -s "$state" \
    "$IMX95_DEBIAN_TOOLS/usr/sbin/debootstrap" --arch=arm64 --foreign --variant=minbase \
    --keyring="$IMX95_DEBIAN_TOOLS/usr/share/keyrings/debian-archive-keyring.gpg" \
    --include="$PACKAGES" trixie "$r" "$MIRROR" > "$w/debootstrap.log" 2>&1 ||
    { tail -20 "$w/debootstrap.log" >&2; die "rootfs: debootstrap's first stage failed"; }
  log "rootfs: modules, firmware, configuration"
  fakeroot -i "$state" -s "$state" make -s -C "$KB" ARCH=arm64 CROSS_COMPILE="$(cross)" INSTALL_MOD_PATH="$r" INSTALL_MOD_STRIP=1 \
    LOCALVERSION= DEPMOD=/bin/true modules_install
  rm -f "$r/lib/modules/$KVER/build" "$r/lib/modules/$KVER/source"
  fakeroot -i "$state" -s "$state" bash "$0" --out "$out" --write-overlay "$r"
  local size_mib=$((TOTAL_MIB - 1 - BOOT_MIB))
  rm -f "$w/root.ext4"
  fakeroot -i "$state" mke2fs -q -t ext4 -L yakogl-root -E root_owner=0:0 -d "$r" "$w/root.ext4" "${size_mib}M"
  log "rootfs: debootstrap's second stage in qemu (virt, TCG; log: $w/stage2.log)"
  qemu_virt "$w/root.ext4" "$w/stage2.log" "root=/dev/vda rw rootfstype=ext4 console=ttyAMA0 init=/yakogl-stage2 panic=-1" ||
    true
  grep -q 'yakogl-stage2: OK' "$w/stage2.log" ||
    { grep -E 'yakogl-stage2|Kernel panic' "$w/stage2.log" | tail -5 >&2; die "rootfs: the second stage did not finish (see $w/stage2.log)"; }
  e2fsck -fy "$w/root.ext4" > "$w/e2fsck.log" 2>&1 || [ $? -le 1 ] || die "rootfs: e2fsck failed (see $w/e2fsck.log)"
  mv "$w/root.ext4" "$out/root.ext4"
  rm -rf "$r"
  log "rootfs: $out/root.ext4"
}

step_card() {
  need IMX95_DEBIAN_TOOLS
  [ -f "$out/root.ext4" ] && [ -f "$out/kernel/Image" ] || die "card: steps kernel and rootfs first"
  local w=$out/card-work m=$IMX95_DEBIAN_TOOLS/usr/bin
  rm -rf "$w"
  mkdir -p "$w"
  $PY bootcmd "$PARTUUID" > "$w/boot.cmd"
  $PY mkscr "$w/boot.cmd" "$w/boot.scr"
  mkfs.vfat -C -F 32 -n BOOT -i "${PARTUUID}" "$w/boot.vfat" $((BOOT_MIB * 1024)) > /dev/null
  export MTOOLS_SKIP_CHECK=1
  "$m/mcopy" -i "$w/boot.vfat" "$out/kernel/Image" "$w/boot.scr" "$w/boot.cmd" "$out/kernel/config-$KVER" ::/
  "$m/mmd" -i "$w/boot.vfat" ::/dtbs
  for d in $DTBS; do "$m/mcopy" -i "$w/boot.vfat" "$out/kernel/dtbs/$d.dtb" ::/dtbs/; done
  rm -f "$out/card.img"
  truncate -s "${TOTAL_MIB}M" "$out/card.img"
  $PY layout "$DISK_ID" "$BOOT_MIB" "$TOTAL_MIB" | sfdisk -q "$out/card.img"
  dd if="$w/boot.vfat" of="$out/card.img" bs=1M seek=1 conv=notrunc,sparse status=none
  dd if="$out/root.ext4" of="$out/card.img" bs=1M seek=$((1 + BOOT_MIB)) conv=notrunc,sparse status=none
  (cd "$out" && sha256sum card.img > card.img.sha256)
  log "card: $out/card.img ($TOTAL_MIB MiB, $(du -m "$out/card.img" | cut -f1) MiB used)"
}

step_verify() {
  need IMX95_DEBIAN_TOOLS
  need MALI_CSFFW
  local v=$out/verify.txt m=$IMX95_DEBIAN_TOOLS/usr/bin bad=0 w=$out/verify-work
  rm -rf "$w"
  mkdir -p "$w"
  : > "$v"
  ok() { echo "ok    $*" | tee -a "$v" >&2; }
  no() { echo "FAIL  $*" | tee -a "$v" >&2; bad=1; }
  { echo "== card.img"; ls -ls "$out/card.img"; cat "$out/card.img.sha256"; echo "== partition table"; sfdisk -d "$out/card.img"; } >> "$v"
  if sfdisk -d "$out/card.img" | diff -q <(sfdisk -d "$out/card.img" | grep -E '^(label|label-id|unit):') <($PY layout "$DISK_ID" "$BOOT_MIB" "$TOTAL_MIB" | grep -E '^(label|label-id|unit):') > /dev/null &&
    sfdisk -d "$out/card.img" | grep -qE "card.img1 : start= *2048, size= *$((BOOT_MIB * 2048)), type=c, bootable" &&
    sfdisk -d "$out/card.img" | grep -qE "card.img2 : start= *$(((1 + BOOT_MIB) * 2048)), size= *$(((TOTAL_MIB - 1 - BOOT_MIB) * 2048)), type=83"; then
    ok "partition table: dos $DISK_ID, p1 FAT32 bootable ${BOOT_MIB} MiB at 1 MiB, p2 Linux"
  else no "partition table"; fi
  export MTOOLS_SKIP_CHECK=1
  local fat="$out/card.img@@1M"
  { echo "== p1 (FAT)"; "$m/mdir" -/ -a -i "$fat" ::/; } >> "$v" 2>&1
  local f
  for f in Image boot.scr boot.cmd "config-$KVER" $(for d in $DTBS; do echo "dtbs/$d.dtb"; done); do
    "$m/mcopy" -n -i "$fat" "::/$f" "$w/$(basename "$f")" 2> /dev/null && cmp -s "$w/$(basename "$f")" \
      "$(case $f in Image) echo "$out/kernel/Image" ;; config-*) echo "$out/kernel/$f" ;; dtbs/*) echo "$out/kernel/$f" ;; *) echo "$out/card-work/$f" ;; esac)" &&
      ok "p1: $f" || no "p1: $f"
  done
  if $PY scr-dump "$w/boot.scr" > "$w/boot.scr.txt" && cmp -s "$w/boot.scr.txt" "$w/boot.cmd" &&
    grep -q "root=PARTUUID=$PARTUUID-02" "$w/boot.cmd"; then
    ok "boot.scr: a valid arm64 script image of boot.cmd, root=PARTUUID=$PARTUUID-02"
  else no "boot.scr"; fi
  "$m/mcopy" -n -i "$fat" "::/config-$KVER" "$w/config" 2> /dev/null
  if $PY config-check "$FRAGMENT" "$w/config" >> "$v"; then ok "kernel config: every line of $(basename "$FRAGMENT")"; else no "kernel config"; fi
  for d in $DTBS; do
    if grep -qa "toradex,verdin-imx95-${d#imx95-verdin-}" "$w/$d.dtb"; then ok "dtb $d: compatible toradex,verdin-imx95-${d#imx95-verdin-}"; else no "dtb $d"; fi
  done
  grep -qa 'arm,mali-valhall-csf' "$w/imx95-verdin-nonwifi-mallow.dtb" && ok "dtb: the GPU node (arm,mali-valhall-csf)" || no "dtb: no GPU node"
  # p2: the rootfs, read with debugfs from a copy of the partition (no mount)
  dd if="$out/card.img" of="$w/p2.ext4" bs=1M skip=$((1 + BOOT_MIB)) conv=sparse status=none
  e2fsck -fn "$w/p2.ext4" >> "$v" 2>&1 && ok "p2: e2fsck clean" || no "p2: e2fsck"
  local dbg="debugfs -R"
  $dbg "ls -l /usr/lib/modules/$KVER/kernel/drivers/gpu/drm/panthor" "$w/p2.ext4" 2> /dev/null | grep -q 'panthor\.ko' &&
    ok "p2: /usr/lib/modules/$KVER/kernel/drivers/gpu/drm/panthor/panthor.ko" || no "p2: no panthor.ko"
  $dbg "dump /usr/lib/modules/$KVER/modules.alias $w/modules.alias" "$w/p2.ext4" > /dev/null 2>&1
  grep -q 'of:N\*T\*Carm,mali-valhall-csf.* panthor$' "$w/modules.alias" && ok "p2: modules.alias loads panthor for arm,mali-valhall-csf" || no "p2: modules.alias"
  $dbg "dump /usr/lib/firmware/arm/mali/arch10.12/mali_csffw.bin $w/fw.bin" "$w/p2.ext4" > /dev/null 2>&1
  local pin got
  pin=$($PY pin "$DEPS_TOML" MALI_CSFFW)
  got=$(sha256sum < "$w/fw.bin" | cut -d' ' -f1)
  [ "$got" = "$pin" ] && ok "p2: arm/mali/arch10.12/mali_csffw.bin sha256 $got = the MALI_CSFFW pin" || no "p2: firmware sha256 $got, pin $pin"
  $dbg "dump /root/.ssh/authorized_keys $w/authorized_keys" "$w/p2.ext4" > /dev/null 2>&1
  if [ -n "${BOARD_SSH_PUBKEY:-}" ]; then
    cmp -s "$w/authorized_keys" "$BOARD_SSH_PUBKEY" && ok "p2: root's authorized_keys is BOARD_SSH_PUBKEY" || no "p2: authorized_keys"
  fi
  $dbg "ls /etc/ssh" "$w/p2.ext4" 2> /dev/null | grep -q 'ssh_host_' && no "p2: ssh host keys baked in" || ok "p2: no ssh host keys (made on first boot)"
  $dbg "cat /etc/shadow" "$w/p2.ext4" 2> /dev/null | grep -qE '^root:[!*]' && ok "p2: root has no password" || no "p2: root's password field"
  $dbg "cat /usr/lib/os-release" "$w/p2.ext4" 2> /dev/null | grep -q 'VERSION_CODENAME=trixie' && ok "p2: Debian trixie" || no "p2: os-release"
  rm -rf "$w"
  [ $bad = 0 ] || die "verify: failures, see $v"
  log "verify: all checks passed ($v)"
}

if [ -n "$overlay" ]; then
  write_overlay "$overlay"
  exit 0
fi
for s in "${steps[@]}"; do
  case $s in
    kernel) step_kernel ;;
    rootfs) step_rootfs ;;
    card) step_card ;;
    verify) step_verify ;;
  esac
done
