1791009407.705 [ 3168.436695] watchdog: watchdog0: watchdog did not stop!
1791009409.943 [ 3170.732559] systemd-shutdown[1]: Failed to disable hardware watchdog, ignoring: Device or resource busy
1791009409.943 [ 3170.742067] watchdog: watchdog0: watchdog did not stop!
1791009410.043 [ 3170.844966] failed to disconnect on suspend
1791009410.054 [ 3170.851238] reboot: Restarting system
1791009410.240 �
1791009410.254 U-Boot SPL 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
1791009410.258 SYSFW ABI: 4.0 (firmware rev 0x000b '11.2.5--v11.02.05 (Fancy Rat)')
1791009410.308 Changed A53 CPU frequency to 1250000000Hz (U grade) in DT
1791009410.310 SPL initial stack usage: 17088 bytes
1791009410.332 Trying to boot from MMC1
1791009410.430 Authentication passed
1791009410.440 Authentication passed
1791009410.440 Authentication passed
1791009410.508 Loading Environment from nowhere... OK
1791009410.518 init_env from device 9 not supported!
1791009410.522 Authentication passed
1791009410.534 Authentication passed
1791009410.547 Starting ATF on ARM64 core...
1791009410.547 
1791009410.555 NOTICE:  BL31: v2.13.0(release):v2.13.0-259-ge0c4d3903b-dirty
1791009410.559 NOTICE:  BL31: Built : 00:00:00, Jan  1 1970
1791009410.786 
1791009410.792 U-Boot SPL 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
1791009410.803 SYSFW ABI: 4.0 (firmware rev 0x000b '11.2.5--v11.02.05 (Fancy Rat)')
1791009410.804 SPL initial stack usage: 1920 bytes
1791009410.804 HW CFG: 0x00
1791009410.808 Trying to boot from MMC1
1791009410.898 Authentication passed
1791009410.906 Authentication passed
1791009412.426 
1791009412.426 
1791009412.441 U-Boot 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
1791009412.441 
1791009412.441 SoC:   AM62PX SR1.0 HS-FS
1791009412.441 DRAM:  2 GiB
1791009412.563 optee optee: OP-TEE: revision 4.7 (a9690ae39995af36)
1791009412.570 Core:  161 devices, 34 uclasses, devicetree: separate
1791009412.584 MMC:   mmc@fa10000: 0, mmc@fa00000: 1
1791009412.676 Loading Environment from MMC... OK
1791009412.696 In:    serial@2800000
1791009412.702 Out:   serial@2800000
1791009412.702 Err:   serial@2800000
1791009412.711 Model: Toradex 0099 Verdin AM62P Quad 2GB WB IT V1.0B
1791009412.711 Serial#: 00000000
1791009412.715 Carrier: Toradex Dahlia V1.1C, Serial# 00000000
1791009412.731 am65_cpsw_nuss ethernet@8000000: K3 CPSW: nuss_ver: 0x6BA01903 cpsw_ver: 0x6BA81903 ale_ver: 0x00290105 Ports:2
1791009412.787 Net:   eth0: ethernet@8000000port@1 [PRIME]Could not get PHY for mdio@f00: addr 7
1791009412.795 am65_cpsw_nuss_port ethernet@8000000port@2: phy_connect() failed
1791009412.795 
1791009412.804 Hit any key to stop autoboot:  0 
1791009412.804 yakogl-netboot: once=probe-none-1791009398
1791009412.820 Saving Environment to MMC... Writing to MMC(0)... OK
1791009412.820 yakogl-netboot: cleared
1791009412.835 ti-udma dma-controller@485c0000: k3_dmaring Ring probed rings:150, sci-dev-id:30
1791009412.838 ti-udma dma-controller@485c0000: dma-ring-reset-quirk: disabled
1791009412.852 am65_cpsw_nuss_port ethernet@8000000port@1: K3 CPSW: rflow_id_base: 19
1791009415.411 ethernet@8000000port@1 Waiting for PHY auto negotiation to complete...... done
1791009415.414 link up on port 1, speed 1000, full duplex
1791009415.414 BOOTP broadcast 1
1791009415.671 BOOTP broadcast 2
1791009416.177 BOOTP broadcast 3
1791009416.201 DHCP client bound to address 192.0.2.3 (785 ms)
1791009416.218 am65_cpsw_nuss_port ethernet@8000000port@1: K3 CPSW: rflow_id_base: 19
1791009416.222 link up on port 1, speed 1000, full duplex
1791009416.222 Using ethernet@8000000port@1 device
1791009416.230 TFTP from server 192.0.2.2; our IP address is 192.0.2.3
1791009416.234 Filename 'yakogl/probe-none-1791009398.bin'.
1791009416.250 Load address: 0x82000000
1791009416.250 Loading: *
1791009416.260 TFTP error: 'File not found' (1)
1791009416.266 Not retrying...
1791009416.266 am65_cpsw_nuss_port ethernet@8000000port@1: RX dma free_pkt failed -22
1791009416.269 yakogl-netboot: fetch failed, booting Linux
1791009416.333 6188160 bytes read in 55 ms (107.3 MiB/s)
1791009416.354 288 bytes read in 20 ms (13.7 KiB/s)
1791009416.364 Failed to load '/safeui/zephyr_a53.bin'
1791009416.364 SafeUI images not found, booting Linux
1791009416.472 7[r[999;999H[6n8MMC: no card present
1791009416.476 No EFI system partition
1791009416.476 No EFI system partition
1791009416.481 Failed to persist EFI variables
1791009416.486 No EFI system partition
1791009416.486 Failed to persist EFI variables
1791009416.491 No EFI system partition
1791009416.491 Failed to persist EFI variables
1791009416.495 ** Booting bootflow '<NULL>' with efi_mgr
1791009416.525 Loading Boot0000 'mmc 0' failed
1791009416.525 EFI boot manager: Cannot load any image
1791009416.527 Boot failed (err=-14)
1791009416.527 MMC: no card present
1791009416.537 MMC: no card present
1791009416.538 MMC: no card present
1791009416.542 MMC: no card present
1791009416.550 ** Booting bootflow 'mmc@fa10000.bootdev.part_1' with script
1791009416.574 6956 bytes read in 21 ms (323.2 KiB/s)
1791009416.599 75135 bytes read in 21 ms (3.4 MiB/s)
1791009416.641 65 bytes read in 20 ms (2.9 KiB/s)
1791009416.641 Working FDT set to 90200000
1791009416.641 Applying Overlay: verdin-am62p_panel-cap-touch-7inch-dsi_overlay.dtbo
1791009416.661 4693 bytes read in 22 ms (208 KiB/s)
1791009416.834 11272338 bytes read in 83 ms (129.5 MiB/s)
1791009416.920 13381763 bytes read in 94 ms (135.8 MiB/s)
1791009416.925    Uncompressing Kernel Image to 0
1791009417.334 ## Flattened Device Tree blob at 90200000
1791009417.338    Booting using the fdt blob at 0x90200000
1791009417.338 Working FDT set to 90200000
1791009417.348    Loading Ramdisk to fc1d3000, end fce96083 ... OK
1791009417.377    Loading Device Tree to 00000000fc19d000, end 00000000fc1d2fff ... OK
1791009417.377 Working FDT set to fc19d000
1791009417.405 
1791009417.409 Starting kernel ...
1791009417.409 
1791009418.662 [    1.059943] tps65219 0-0030: Error IRQ trap residual voltage for LDO4
1791009418.678 [    1.083605] debugfs: File ':bus@f0000:bus@48000000:interrupt-controller@48000000' in directory 'domains' already present!
1791009418.689 [    1.083945] debugfs: File ':bus@f0000:bus@4e000000:interrupt-controller@4e400000' in directory 'domains' already present!
1791009419.230 [    1.668540] OF: graph: no port node found in /bus@f0000/dss@30220000/oldi-txes/oldi@1/ports
1791009419.293 [    1.729595] OF: graph: no port node found in /bus@f0000/dss@30220000/oldi-txes/oldi@1/ports
1791009419.303 [    1.739541] cdns-dsi 30500000.dsi: [drm] *ERROR* modalias failure on /bus@f0000/dsi@30500000/port@1
1791009419.311 [    1.748609] cdns-dsi 30500000.dsi: [drm] *ERROR* modalias failure on /bus@f0000/dsi@30500000/port@0
1791009419.320 [    1.761031] OF: graph: no port node found in /bus@f0000/dss@30220000/oldi-txes/oldi@1/ports
1791009419.353 Starting systemd-udevd version 255.21^
1791009420.344 sysroot.readonly configuration value: 0 (fs writable: 1)
1791009420.348 Using legacy ostree bind mount for /
1791009425.580 [    8.022251] omap-mailbox 29000000.mailbox: no available mbox devices found
1791009425.602 [    8.043563] omap-mailbox 29020000.mailbox: no available mbox devices found
1791009425.676 [    8.117410] omap-mailbox 29030000.mailbox: no available mbox devices found
1791009425.769 [    8.211696] vdec 30210000.video-codec: OPP table not found in device tree
1791009426.369 [    8.809925] remoteproc remoteproc0: request_firmware failed: -2
1791009426.422 [    8.864905] platform 78000000.r5f: mbox_request_channel: can't parse "mboxes" property
1791009426.454 [    8.890510] platform 78000000.r5f: error -ENOENT: mbox_request_channel failed
1791009426.454 [    8.897678] k3_r5_rproc bus@f0000:bus@b00000:r5fss@78000000: k3_r5_cluster_rproc_init failed, ret = -2
1791009456.526 
1791009456.536 Torizon OS 7.6.1+build.38 verdin-am62p-00000000 ttyS2
1791009456.536 
1791009833.487 verdin-am62p-00000000 login: 
