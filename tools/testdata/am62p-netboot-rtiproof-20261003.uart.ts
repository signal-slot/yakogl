1791013375.598 [ 3958.045126] watchdog: watchdog0: watchdog did not stop!
1791013377.923 [ 3960.365811] systemd-shutdown[1]: Failed to disable hardware watchdog, ignoring: Device or resource busy
1791013377.928 [ 3960.375343] watchdog: watchdog0: watchdog did not stop!
1791013378.031 [ 3960.476314] failed to disconnect on suspend
1791013378.033 [ 3960.482597] reboot: Restarting system
1791013378.220 �
1791013378.230 U-Boot SPL 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
1791013378.237 SYSFW ABI: 4.0 (firmware rev 0x000b '11.2.5--v11.02.05 (Fancy Rat)')
1791013378.284 Changed A53 CPU frequency to 1250000000Hz (U grade) in DT
1791013378.294 SPL initial stack usage: 17088 bytes
1791013378.304 Trying to boot from MMC1
1791013378.408 Authentication passed
1791013378.418 Authentication passed
1791013378.423 Authentication passed
1791013378.497 Loading Environment from nowhere... OK
1791013378.498 init_env from device 9 not supported!
1791013378.498 Authentication passed
1791013378.513 Authentication passed
1791013378.526 Starting ATF on ARM64 core...
1791013378.526 
1791013378.534 NOTICE:  BL31: v2.13.0(release):v2.13.0-259-ge0c4d3903b-dirty
1791013378.534 NOTICE:  BL31: Built : 00:00:00, Jan  1 1970
1791013378.805 
1791013378.805 U-Boot SPL 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
1791013378.805 SYSFW ABI: 4.0 (firmware rev 0x000b '11.2.5--v11.02.05 (Fancy Rat)')
1791013378.805 SPL initial stack usage: 1920 bytes
1791013378.805 HW CFG: 0x00
1791013378.805 Trying to boot from MMC1
1791013378.875 Authentication passed
1791013378.887 Authentication passed
1791013380.404 
1791013380.404 
1791013380.421 U-Boot 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
1791013380.421 
1791013380.421 SoC:   AM62PX SR1.0 HS-FS
1791013380.421 DRAM:  2 GiB
1791013380.532 optee optee: OP-TEE: revision 4.7 (a9690ae39995af36)
1791013380.545 Core:  161 devices, 34 uclasses, devicetree: separate
1791013380.567 MMC:   mmc@fa10000: 0, mmc@fa00000: 1
1791013380.651 Loading Environment from MMC... OK
1791013380.673 In:    serial@2800000
1791013380.678 Out:   serial@2800000
1791013380.684 Err:   serial@2800000
1791013380.693 Model: Toradex 0099 Verdin AM62P Quad 2GB WB IT V1.0B
1791013380.693 Serial# 00000000
1791013380.693 Carrier: Toradex Dahlia V1.1C, Serial# 00000000
1791013380.710 am65_cpsw_nuss ethernet@8000000: K3 CPSW: nuss_ver: 0x6BA01903 cpsw_ver: 0x6BA81903 ale_ver: 0x00290105 Ports:2
1791013380.761 Net:   eth0: ethernet@8000000port@1 [PRIME]Could not get PHY for mdio@f00: addr 7
1791013380.766 am65_cpsw_nuss_port ethernet@8000000port@2: phy_connect() failed
1791013380.772 
1791013380.783 Hit any key to stop autoboot:  0 
1791013380.788 yakogl-netboot: once=rtiproof-691c3e985b1f068d-1791013366
1791013380.792 Saving Environment to MMC... Writing to MMC(0)... OK
1791013380.796 yakogl-netboot: cleared
1791013380.820 ti-udma dma-controller@485c0000: k3_dmaring Ring probed rings:150, sci-dev-id:30
1791013380.820 ti-udma dma-controller@485c0000: dma-ring-reset-quirk: disabled
1791013380.820 am65_cpsw_nuss_port ethernet@8000000port@1: K3 CPSW: rflow_id_base: 19
1791013383.386 ethernet@8000000port@1 Waiting for PHY auto negotiation to complete...... done
1791013383.395 link up on port 1, speed 1000, full duplex
1791013383.395 BOOTP broadcast 1
1791013383.644 BOOTP broadcast 2
1791013384.153 BOOTP broadcast 3
1791013384.176 DHCP client bound to address 192.0.2.9 (781 ms)
1791013384.191 am65_cpsw_nuss_port ethernet@8000000port@1: K3 CPSW: rflow_id_base: 19
1791013384.207 link up on port 1, speed 1000, full duplex
1791013384.207 Using ethernet@8000000port@1 device
1791013384.207 TFTP from server 192.0.2.9; our IP address is 192.0.2.9
1791013384.208 Filename 'yakogl/rtiproof-691c3e985b1f068d-1791013366.bin'.
1791013384.231 Load address: 0x82000000
1791013384.479 Loading: *##################################################  15.5 MiB
1791013384.479 	 63.7 MiB/s
1791013384.479 done
1791013384.484 Bytes transferred = 16238444 (f7c76c hex)
1791013384.484 yakogl-netboot: go 0x82000000
1791013384.489 ## Starting application at 0x82000000 ...
1791013384.559 Hosample: build tree=71145c16 table=1449eb0f
1791013384.573 I: rti_arm: armed preload 0xef rate 32768 Hz: expiry 60000 ms, fed for 0 s
1791013384.584 W: rti_arm: proof: not feeding; the RTI must reset the SoC within 60 s (software backstop at 90 s)
1791013384.584 I: Cadence DSI id 0xcadd5131, IP_CONF 0x948490d0
1791013384.588 I: D-PHY: WIZ at +0xf00 (MOD_VER 0x69924881)
1791013384.617 I: VP clock source 51200000 Hz (pixel clock 51200000 Hz)
1791013384.617 I: DSS rev 0x64040901
1791013384.628 I: DSS ready: VP2 1024x600 @ 51200000 Hz, safety layer ARGB8888 at 0x848973c0
1791013384.628 I: TISCI: power domain 237 on
1791013384.633 I: TISCI: power domain 242 on
1791013384.633 I: TISCI: GPU clock 237/3 = 800000000 Hz
1791013384.641 I: TISCI: GPU clock 237/3 runs at 800000000 Hz
1791013384.657 I: GPU regs 0xfd80000 (512 KiB) mapped at 0xfd80000
1791013384.657 I: CORE_ID = 0x002400350068031c -> BVNC 36.53.104.796
1791013384.657 I: PowerVR BXS-4-64 MC1 alive (BVNC 36.53.104.796)
1791013384.657 I: GPU bring-up milestone 1 complete (power + BVNC readback)
1791013384.768 I: GPU core clock measured: 800 MHz (CR_TIMER 312502 ticks in 100 ms)
1791013384.768 I: GPU carveout phys 0x97800000 (64 MiB) mapped at 0x8e000000
1791013384.770 I: DDK FW: code@0x0 (196608 B), data@0x80000 (135168 B), objs from 0xc0000
1791013384.788 I: DDK FW: 229376-byte image for DDK 25.2 loaded (code 0x0, data 0x80000, rodata 0xa0000)
1791013384.792 I: runtime cfg: ActivePMLatency 5 ms
1791013384.797 I: FWIF structs built (KCCB/FWCCB/sysinit/osinit); heap used to 0xd8d28
1791013384.808 I: MMU: FW heap mapped (4096 KiB main + 192 KiB config of the 32 MiB window), PC root phys=0x97c30000
1791013384.812 I: M3 complete (MMU + FW load + FWIF structs). Booting FW core...
1791013384.817 I: FW boot: firmware_started=0x1 after ~1 ms
1791013384.822 I: GPU bring-up milestone 4 complete (firmware booted)
1791013384.832 I: DDK FW: skipping open-FW healthcheck; render KICK drives the FW
1791013384.846 I: render: scaffolding built; awaiting final cmd_transfer regs (pvr_render_regs.h) before first GPU draw
1791013384.846 I: D-PHY: 307812500 bit/s per lane, band 5
1791013384.846 I: DSI link up: 4 lanes, sync events, line 4047 B, 307812500 bit/s per lane
1791013384.876 I: sn65dsi8x: ID "58ISD" rev 0x01 (attempt 1)
1791013384.967 I: sn65dsi8x: up, DSI 153600 kHz (range 0x1e), LVDS 51200 kHz = /3 (range 1), fmt 0x68
1791013384.978 I: sn65dsi8x: link clean (IRQ_STAT 0x00)
1791013384.978 I: SN65DSI8x up: lvds range 1, dsi range 30, div 2, fmt 0x68, IRQ_STAT 0x00
1791013384.978 *** Booting Zephyr OS build af17c0c8aa2a ***
1791013384.978 Secondary CPU core 1 (MPID:0x1) is up
1791013384.978 Secondary CPU core 2 (MPID:0x2) is up
1791013384.978 Secondary CPU core 3 (MPID:0x3) is up
1791013384.978 gl_test: pvr_debug_masked_clear=1 (clears honour glColorMask)
1791013384.978 gl_test: PVR_BACKEND_OPT_DBIAS_INT=-1 (-1: untouched)
1791013384.978 gl_test: start (case=7 pretransform=0 ppp_ctrl=0x0 debug=0 no_coalesce=0)
1791013384.993 I: render: parameter buffer: carveout 64 MiB -> 448 pages listed, 4096 max, grow 256, pool 0 pages
1791013385.034 I: freelist id=1 np=512 max=4096 backing=1536 fpm=0 ebase=3584 arr.gpu=0xfa00000000 pages_phys=0x98048000 cur_dev=0xfa00003800 pfn[3584]=0x00098048 pfn[4095]=0x00098247
1791013385.077 I: freelist id=2 np=512 max=4096 backing=1536 fpm=0 ebase=3584 arr.gpu=0xfa00004000 pages_phys=0x9864c000 cur_dev=0xfa00007800 pfn[3584]=0x0009864c pfn[4095]=0x0009884b
1791013385.087 I: freelist id=3 np=26 max=32 backing=26 fpm=0 ebase=4 arr.gpu=0xfa00008000 pages_phys=0x98c50000 cur_dev=0xfa00008010 pfn[4]=0x00098c50 pfn[31]=0x00000000
1791013385.108 I: render: pds0: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 f4000002 | @0x80: f0080101 f0090201  usc0: 0580a044 ff000000 00000000 00000000
1791013385.118 I: render: init ok (PB@0x8000000000 border@0x8000200000 eot_usc_off=0x1000 gen_b=0xe2c000 pds_b=0x1000 usc_b=0x6000)
1791013385.118 I: osal(zephyr): ready (regs=0xfd80000)
1791013385.219 I: pvr_backend: init ok (pds arena off 0x1000, gen arena 0x8000e2c000, vdm 0x800122c000, clear_fs off 0x8000)
1791013385.247 I: render: hc dwords: 00000000 003f003f 00000088 00000000 00000088 00000000 00000000 00000080 00003003 00000004 00000000 00080000 00080000 00000000 00000000 00000000
1791013385.256 I: render: hrt[0]: 0000c000 000000fa 00000000 00000000 00000000 00000000 00000000 00000000
1791013385.260 I: render: hrt[8]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013385.273 I: render: hrt[16]: 00000000 c00e9d00 00000000 00000000 c00d8da8 c00d8d28 c00d8d68 00000000
1791013385.283 I: render: hrt[24]: 00000000 00000000 01e8c000 00000080 00000000 00000000 00000000 00000000
1791013385.287 I: render: hrt[32]: 00000000 00000000 00000000 00000000 01e88000 00000080 01e9e000 00000080
1791013385.304 I: render: hrt[40]: 01e8e000 00000080 00000000 00000000 00000000 00000000 00000000 00000000
1791013385.307 I: render: hrt[48]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013385.312 I: render: hrt[56]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013385.329 I: render: HWRT 64x64 x1 te_screen=0x00003003 mtile1=0x00080000 mtile2=0x00080000 isp_mtile=0x00020002 stride=4 rgn=16 tpc=128 teaa=0
1791013385.329 gl_test: px                                 
1791013385.336 gl_test: px                                 
1791013385.340 gl_test: px                                 
1791013385.340 gl_test: px                                 
1791013385.344 gl_test: px                                 
1791013385.349 gl_test: px                 R               
1791013385.358 gl_test: px                RR               
1791013385.359 gl_test: px                RRR              
1791013385.359 gl_test: px               RRRR              
1791013385.365 gl_test: px               RRRRR             
1791013385.369 gl_test: px              RRRRRR             
1791013385.376 gl_test: px              RRRRRRR            
1791013385.381 gl_test: px             RRRRRRRR            
1791013385.387 gl_test: px             RRRRRRRRR           
1791013385.387 gl_test: px            RRRRRRRRRR           
1791013385.391 gl_test: px            RRRRRRRRRRR          
1791013385.391 gl_test: px           RRRRRRRRRRRR          
1791013385.396 gl_test: px           RRRRRRRRRRRRR         
1791013385.401 gl_test: px          RRRRRRRRRRRRRR         
1791013385.412 gl_test: px          RRRRRRRRRRRRRRR        
1791013385.412 gl_test: px         RRRRRRRRRRRRRRRR        
1791013385.413 gl_test: px         RRRRRRRRRRRRRRRRR       
1791013385.417 gl_test: px        RRRRRRRRRRRRRRRRRR       
1791013385.417 gl_test: px        RRRRRRRRRRRRRRRRRRR      
1791013385.427 gl_test: px       RRRRRRRRRRRRRRRRRRRR      
1791013385.439 gl_test: px       RRRRRRRRRRRRRRRRRRRRR     
1791013385.439 gl_test: px      RRRRRRRRRRRRRRRRRRRRRR     
1791013385.439 gl_test: px      RRRRRRRRRRRRRRRRRRRRRRR    
1791013385.443 gl_test: px                                 
1791013385.447 gl_test: px                                 
1791013385.447 gl_test: px                                 
1791013385.453 gl_test: px                                 
1791013385.462 gl_test:   orange centroid              (32,40) = 0xff0080ff  want 0xff0080ff  ok  [check 1]
1791013385.467 gl_test:   orange apex                  (31,12) = 0xff0080ff  want 0xff0080ff  ok  [check 2]
1791013385.480 gl_test:   background                   ( 1, 1) = 0xff000000  want 0xff000000  ok  [check 3]
1791013385.489 I: render: hc dwords: 00000000 003f003f 00000088 00000000 00000088 00000000 00000000 00000080 00003003 00000004 00000000 00080000 00080000 00000000 00000000 00000000
1791013385.501 I: render: hrt[0]: 00010000 000000fa 00000000 00000000 00000000 00000000 00000000 00000000
1791013385.510 I: render: hrt[8]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013385.521 I: render: hrt[16]: 00000000 c00e9e80 00000000 00000000 c00d8da8 c00d8d28 c00d8d68 00000000
1791013385.524 I: render: hrt[24]: 00000000 00000000 01ea3000 00000080 00000000 00000000 00000000 00000000
1791013385.533 I: render: hrt[32]: 00000000 00000000 00000000 00000000 01e9f000 00000080 01eb5000 00000080
1791013385.551 I: render: hrt[40]: 01ea5000 00000080 00000000 00000000 00000000 00000000 00000000 00000000
1791013385.551 I: render: hrt[48]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013385.563 I: render: hrt[56]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013385.567 I: render: HWRT 64x64 x1 te_screen=0x00003003 mtile1=0x00080000 mtile2=0x00080000 isp_mtile=0x00020002 stride=4 rgn=16 tpc=128 teaa=0
1791013385.585 gl_test:   cyan centroid                (32,40) = 0xffffff00  want 0xffffff00  ok  [check 4]
1791013385.585 gl_test:   active attributes 2; [1] = a_color size 1 type 0x8b52
1791013385.594 gl_test:   bound a_pos -> 3, a_color -> 1 (link 1)
1791013385.605 gl_test:   magenta via attribute 3      (32,40) = 0xffff00ff  want 0xffff00ff  ok  [check 5]
1791013385.609 gl_test: px                                 
1791013385.614 gl_test: px                                 
1791013385.614 gl_test: px                                 
1791013385.629 gl_test: px                                 
1791013385.629 gl_test: px                                 
1791013385.630 gl_test: px                                 
1791013385.633 gl_test: px                                 
1791013385.633 gl_test: px                                 
1791013385.643 gl_test: px                                 
1791013385.649 gl_test: px                                 
1791013385.649 gl_test: px                                 
1791013385.656 gl_test: px                 R               
1791013385.671 gl_test: px                RR               
1791013385.671 gl_test: px                RRR              
1791013385.671 gl_test: px               RRRR              
1791013385.671 gl_test: px               RRRRR             
1791013385.671 gl_test: px              RRRRRR             
1791013385.675 gl_test: px              RRRRRRR            
1791013385.680 gl_test: px             RRRRRRRR            
1791013385.685 gl_test: px             RRRRRRRRR           
1791013385.692 gl_test: px            RRRRRRRRRR           
1791013385.698 gl_test: px            RRRRRRRRRRR          
1791013385.703 gl_test: px                                 
1791013385.703 gl_test: px                                 
1791013385.708 gl_test: px                                 
1791013385.708 gl_test: px                                 
1791013385.713 gl_test: px                                 
1791013385.719 gl_test: px                                 
1791013385.725 gl_test: px                                 
1791013385.725 gl_test: px                                 
1791013385.738 gl_test: px                                 
1791013385.738 gl_test: px                                 
1791013385.738 gl_test:   half: inside small tri       (32,40) = 0xff00ffff  want 0xff00ffff  ok  [check 6]
1791013385.750 gl_test:   half: outside small tri      (32,14) = 0xff000000  want 0xff000000  ok  [check 7]
1791013385.759 gl_test:   half: outside small tri      (12,52) = 0xff000000  want 0xff000000  ok  [check 8]
1791013385.776 gl_test:   shift: moved right           (56,40) = 0xff00ffff  want 0xff00ffff  ok  [check 9]
1791013385.777 gl_test:   shift: left now empty        (16,40) = 0xff000000  want 0xff000000  ok  [check 10]
1791013385.785 gl_test:   active uniforms 2; [0] = u_mvp size 1 type 0x8b5c
1791013385.785 gl_test: case 1 uniform: PASS
1791013385.796 gl_test: 2x2 texture @0x8001ebf000: ff0000ff ff00ff00 ffff0000 ffffffff | row1 ffff0000 ffffffff
1791013385.807 gl_test: 2x2 texture on the full-screen quad:
1791013385.807 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.811 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.817 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.821 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.825 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.825 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.831 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.836 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.836 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.846 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.847 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.850 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.850 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.855 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.860 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.870 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.871 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013385.877 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013385.877 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013385.889 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013385.890 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013385.890 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013385.891 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013385.897 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013385.902 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013385.907 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013385.911 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013385.911 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013385.921 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013385.927 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013385.933 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013385.933 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013385.940 gl_test:   top-left = blue              (16,16) = 0xffff0000  want 0xffff0000  ok  [check 1]
1791013385.944 gl_test:   top-right = white            (48,16) = 0xffffffff  want 0xffffffff  ok  [check 2]
1791013385.956 gl_test:   bottom-left = red            (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 3]
1791013385.961 gl_test:   bottom-right = green         (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 4]
1791013385.973 gl_test: 4x4 texture @0x8001ec4000: ff0000ff ff0000ff ff00ff00 ff00ff00 | row1 ff0000ff ff0000ff
1791013385.983 gl_test: 4x4 texture on the full-screen quad:
1791013385.987 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.987 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.994 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013385.994 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.001 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.002 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.007 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.013 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.017 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.017 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.023 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.027 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.033 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.037 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.037 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.043 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.050 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.053 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.053 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.063 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.064 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.067 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.067 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.075 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.075 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.079 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.086 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.091 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.096 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.096 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.101 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.107 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.116 gl_test:   top-left = blue              (16,16) = 0xffff0000  want 0xffff0000  ok  [check 5]
1791013386.121 gl_test:   top-right = white            (48,16) = 0xffffffff  want 0xffffffff  ok  [check 6]
1791013386.133 gl_test:   bottom-left = red            (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 7]
1791013386.137 gl_test:   bottom-right = green         (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 8]
1791013386.147 gl_test: 8x8 texture @0x8001ebf000: ff0000ff ff0000ff ff0000ff ff0000ff | row1 ff0000ff ff0000ff
1791013386.158 gl_test: 8x8 texture on the full-screen quad:
1791013386.158 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.164 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.179 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.179 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.179 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.179 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.179 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.186 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.193 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.198 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.198 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.203 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.208 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.208 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.216 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.229 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.230 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.230 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.232 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.238 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.238 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.243 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.248 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.253 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.253 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.260 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.264 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.269 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.269 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.286 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.287 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.287 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.288 gl_test:   top-left = blue              (16,16) = 0xffff0000  want 0xffff0000  ok  [check 9]
1791013386.302 gl_test:   top-right = white            (48,16) = 0xffffffff  want 0xffffffff  ok  [check 10]
1791013386.307 gl_test:   bottom-left = red            (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 11]
1791013386.317 gl_test:   bottom-right = green         (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 12]
1791013386.322 gl_test: 64x64 texture @0x8001ec5000: ff0000ff ff0000ff ff0000ff ff0000ff | row1 ff0000ff ff0000ff
1791013386.340 gl_test: 64x64 texture on the full-screen quad:
1791013386.340 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.340 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.343 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.347 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.352 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.359 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.366 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.366 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.371 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.371 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.382 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.389 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.389 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.390 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.390 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.393 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013386.398 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.403 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.409 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.415 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.421 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.421 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.425 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.425 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.430 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.437 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.441 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.452 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.452 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.452 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.454 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.460 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013386.468 gl_test:   top-left = blue              (16,16) = 0xffff0000  want 0xffff0000  ok  [check 13]
1791013386.485 gl_test:   top-right = white            (48,16) = 0xffffffff  want 0xffffffff  ok  [check 14]
1791013386.485 gl_test:   bottom-left = red            (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 15]
1791013386.489 gl_test:   bottom-right = green         (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 16]
1791013386.505 gl_test: gradient texture (R = 36 x, G = 36 y, B = 128), 8x8 grid (top-down rows):
1791013386.510 gl_test:  row  4: ff80fc00 ff80fc24 ff80fc48 ff80fc6c ff80fc90 ff80fcb4 ff80fcd8 ff80fcfc
1791013386.519 gl_test:  row 12: ff80d800 ff80d824 ff80d848 ff80d86c ff80d890 ff80d8b4 ff80d8d8 ff80d8fc
1791013386.529 gl_test:  row 20: ff80b400 ff80b424 ff80b448 ff80b46c ff80b490 ff80b4b4 ff80b4d8 ff80b4fc
1791013386.534 gl_test:  row 28: ff809000 ff809024 ff809048 ff80906c ff809090 ff8090b4 ff8090d8 ff8090fc
1791013386.545 gl_test:  row 36: ff806c00 ff806c24 ff806c48 ff806c6c ff806c90 ff806cb4 ff806cd8 ff806cfc
1791013386.551 gl_test:  row 44: ff804800 ff804824 ff804848 ff80486c ff804890 ff8048b4 ff8048d8 ff8048fc
1791013386.567 gl_test:  row 52: ff802400 ff802424 ff802448 ff80246c ff802490 ff8024b4 ff8024d8 ff8024fc
1791013386.568 gl_test:  row 60: ff800000 ff800024 ff800048 ff80006c ff800090 ff8000b4 ff8000d8 ff8000fc
1791013386.574 gl_test:   texel (0,7) at top-left      ( 4, 4) = 0xff80fc00  want 0xff80fc00  ok  [check 17]
1791013386.587 gl_test:   texel (7,0) at bottom-right  (60,60) = 0xff8000fc  want 0xff8000fc  ok  [check 18]
1791013386.591 gl_test:   texel (2,5)                  (20,20) = 0xff80b448  want 0xff80b448  ok  [check 19]
1791013386.606 gl_test:   texel (5,2)                  (44,44) = 0xff8048b4  want 0xff8048b4  ok  [check 20]
1791013386.613 gl_test:   mip: magnified -> level 0    (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 21]
1791013386.621 gl_test:   mip: 2x2 px quad -> level 1  (30,30) = 0xff00ff00  want 0xff00ff00  ok  [check 22]
1791013386.632 gl_test:   mip: 1x1 px quad -> level 2  (40,40) = 0xffff0000  want 0xffff0000  ok  [check 23]
1791013386.647 gl_test:   generated level 2 = average  (40,40) = 0xff800080  want 0xff800080  ok  [check 24]
1791013386.656 gl_test:   trilinear at level 2         (40,40) = 0xff800080  want 0xff800080  ok  [check 25]
1791013386.667 gl_test: 4x4 texture @0x8001ec8000: ff0000ff ff0000ff ff00ff00 ff00ff00 | row1 ff0000ff ff0000ff
1791013386.678 gl_test:   2 units: red*0.5 + yellow*0.25 (16,48) = 0xff0040bf  want 0xff0040bf  ok  [check 26]
1791013386.689 gl_test:   2 units: blue*0.5 + yellow*0.25 (16,16) = 0xff804040  want 0xff804040  ok  [check 27]
1791013386.697 gl_test:   swapped: yellow*0.5 + red*0.25 (16,48) = 0xff0080bf  want 0xff0080bf  ok  [check 28]
1791013386.715 gl_test:   swapped: yellow*0.5 + blue*0.25 (16,16) = 0xff408080  want 0xff408080  ok  [check 29]
1791013386.715 gl_test: case 2 texture: PASS
1791013386.716 gl_test: px                                 
1791013386.720 gl_test: px                                 
1791013386.724 gl_test: px                 R               
1791013386.730 gl_test: px                RR               
1791013386.735 gl_test: px                RRR              
1791013386.741 gl_test: px               RRRR              
1791013386.747 gl_test: px               RRRRR             
1791013386.747 gl_test: px              RRRRRR             
1791013386.752 gl_test: px              RRRRRRR            
1791013386.752 gl_test: px             RRRRRRRR            
1791013386.757 gl_test: px             RRRRRRRRR           
1791013386.763 gl_test: px            RRRRRRRRRR           
1791013386.770 gl_test: px            RRRRRRRRRRR          
1791013386.770 gl_test: px           RRRRRRBRRRRR          
1791013386.774 gl_test: px           RRRRRBBRRRRRR         
1791013386.774 gl_test: px          RRRRRRBBBRRRRR         
1791013386.779 gl_test: px          RRRRRBBBBRRRRRR        
1791013386.797 gl_test: px         RRRRRRBBBBBRRRRR        
1791013386.797 gl_test: px         RRRRRBBBBBBRRRRRR       
1791013386.797 gl_test: px        RRRRRRBBBBBBBRRRRR       
1791013386.797 gl_test: px        RRRRRRRRRRRRRRRRRRR      
1791013386.797 gl_test: px       RRRRRRRRRRRRRRRRRRRR      
1791013386.801 gl_test: px       RRRRRRRRRRRRRRRRRRRRR     
1791013386.811 gl_test: px      RRRRRRRRRRRRRRRRRRRRRR     
1791013386.817 gl_test: px      RRRRRRRRRRRRRRRRRRRRRRR    
1791013386.817 gl_test: px     RRRRRRRRRRRRRRRRRRRRRRRR    
1791013386.824 gl_test: px     RRRRRRRRRRRRRRRRRRRRRRRRR   
1791013386.829 gl_test: px    RRRRRRRRRRRRRRRRRRRRRRRRRR   
1791013386.829 gl_test: px    RRRRRRRRRRRRRRRRRRRRRRRRRRR  
1791013386.867 gl_test: px   RRRRRRRRRRRRRRRRRRRRRRRRRRRR  
1791013386.868 gl_test: px   RRRRRRRRRRRRRRRRRRRRRRRRRRRRR 
1791013386.868 gl_test: px                                 
1791013386.868 gl_test:   depth on: C in front         (32,32) = 0xffff0000  want 0xffff0000  ok  [check 1]
1791013386.868 gl_test:   depth on: B hidden           (32,44) = 0xff0000ff  want 0xff0000ff  ok  [check 2]
1791013386.868 gl_test:   depth on: A only             (32,54) = 0xff0000ff  want 0xff0000ff  ok  [check 3]
1791013386.894 gl_test:   depth off: C last            (32,32) = 0xffff0000  want 0xffff0000  ok  [check 4]
1791013386.894 gl_test:   depth off: B over A          (32,44) = 0xff00ff00  want 0xff00ff00  ok  [check 5]
1791013386.895 gl_test:   depth off: A only            (32,54) = 0xff0000ff  want 0xff0000ff  ok  [check 6]
1791013386.904 gl_test:   greater: B over A, C         (32,32) = 0xff00ff00  want 0xff00ff00  ok  [check 7]
1791013386.917 gl_test:   greater: B over A            (32,44) = 0xff00ff00  want 0xff00ff00  ok  [check 8]
1791013386.921 gl_test:   greater: A only              (32,54) = 0xff0000ff  want 0xff0000ff  ok  [check 9]
1791013386.939 gl_test:   offset -4: D in front of A   (32,54) = 0xff00ff00  want 0xff00ff00  ok  [check 10]
1791013386.945 gl_test:   offset +4: A stays           (32,54) = 0xff0000ff  want 0xff0000ff  ok  [check 11]
1791013386.951 gl_test: case 3 depth: PASS
1791013386.956 gl_test: px                                 
1791013386.961 gl_test: px                                 
1791013386.967 gl_test: px                                 
1791013386.967 gl_test: px                                 
1791013386.971 gl_test: px                                 
1791013386.977 gl_test: px                 R               
1791013386.977 gl_test: px                RR               
1791013386.982 gl_test: px                RRR              
1791013386.986 gl_test: px               RRRR              
1791013386.996 gl_test: px               RRRRR             
1791013386.997 gl_test: px              RRRRRR             
1791013387.000 gl_test: px              RRRRRRR            
1791013387.000 gl_test: px             RRRRRRRR            
1791013387.005 gl_test: px             RRRRRRRRR           
1791013387.015 gl_test: px            RRRRRRRRRR           
1791013387.016 gl_test: px            RRRRRRRRRRR          
1791013387.021 gl_test: px           RRRRRRRRRRRR          
1791013387.021 gl_test: px           RRRRRRRRRRRRR         
1791013387.026 gl_test: px          RRRRRRRRRRRRRR         
1791013387.040 gl_test: px          RRRRRRRRRRRRRRR        
1791013387.041 gl_test: px         RRRRRRRRRRRRRRRR        
1791013387.041 gl_test: px         RRRRRRRRRRRRRRRRR       
1791013387.041 gl_test: px        RRRRRRRRRRRRRRRRRR       
1791013387.043 gl_test: px        RRRRRRRRRRRRRRRRRRR      
1791013387.048 gl_test: px       RRRRRRRRRRRRRRRRRRRR      
1791013387.053 gl_test: px       RRRRRRRRRRRRRRRRRRRRR     
1791013387.061 gl_test: px      RRRRRRRRRRRRRRRRRRRRRR     
1791013387.067 gl_test: px      RRRRRRRRRRRRRRRRRRRRRRR    
1791013387.067 gl_test: px                                 
1791013387.071 gl_test: px                                 
1791013387.153 gl_test: px                                 
1791013387.158 gl_test: px                                 
1791013387.164 gl_test:   ccw front, cull back         (32,40) = 0xff00ffff  want 0xff00ffff  ok  [check 1]
1791013387.164 gl_test:   ccw front, cull front        (32,40) = 0xff000000  want 0xff000000  ok  [check 2]
1791013387.164 gl_test:   cw front, cull back          (32,40) = 0xff000000  want 0xff000000  ok  [check 3]
1791013387.164 gl_test:   cw front, cull front         (32,40) = 0xff00ffff  want 0xff00ffff  ok  [check 4]
1791013387.164 gl_test:   fbo: ccw front, cull back    (32,32) = 0xff00ffff  want 0xff00ffff  ok  [check 5]
1791013387.164 gl_test:   fbo: ccw front, cull front   (32,32) = 0xff000000  want 0xff000000  ok  [check 6]
1791013387.164 gl_test: case 4 cull: PASS
1791013387.164 gl_test: px                                 
1791013387.164 gl_test: px                                 
1791013387.169 gl_test: px                                 
1791013387.169 gl_test: px                                 
1791013387.175 gl_test: px                                 
1791013387.181 gl_test: px                                 
1791013387.181 gl_test: px                                 
1791013387.186 gl_test: px                                 
1791013387.191 gl_test: px                                 
1791013387.191 gl_test: px                                 
1791013387.206 gl_test: px                                 
1791013387.206 gl_test: px                                 
1791013387.206 gl_test: px                                 
1791013387.207 gl_test: px                                 
1791013387.211 gl_test: px                                 
1791013387.216 gl_test: px                                 
1791013387.223 gl_test: px RRRRRRRRRRRRRRRR                
1791013387.229 gl_test: px RRRRRRRRRRRRRRRR                
1791013387.242 gl_test: px RRRRRRRRRRRRRRRR                
1791013387.242 gl_test: px RRRRRRRRRRRRRRRR                
1791013387.243 gl_test: px RRRRRRRRRRRRRRRR                
1791013387.243 gl_test: px RRRRRRRRRRRRRRRR                
1791013387.243 gl_test: px RRRRRRRRRRRRRRRR                
1791013387.247 gl_test: px RRRRRRRRRRRRRRRR                
1791013387.257 gl_test: px RRRRRRRRRRRRRRRR                
1791013387.258 gl_test: px RRRRRRRRRRRRRRRR                
1791013387.262 gl_test: px RRRRRRRRRRRRRRRR                
1791013387.268 gl_test: px RRRRRRRRRRRRRRRR                
1791013387.273 gl_test: px RRRRRRRRRRRRRRRR                
1791013387.273 gl_test: px RRRRRRRRRRRRRRRR                
1791013387.277 gl_test: px RRRRRRRRRRRRRRRR                
1791013387.282 gl_test: px RRRRRRRRRRRRRRRR                
1791013387.289 gl_test:   inside (bottom-left)         (16,48) = 0xff00ffff  want 0xff00ffff  ok  [check 1]
1791013387.304 gl_test:   inside edge                  (31,32) = 0xff00ffff  want 0xff00ffff  ok  [check 2]
1791013387.304 gl_test:   outside right                (32,32) = 0xff000000  want 0xff000000  ok  [check 3]
1791013387.316 gl_test:   outside above                (16,31) = 0xff000000  want 0xff000000  ok  [check 4]
1791013387.321 gl_test:   outside (top-right)          (48,16) = 0xff000000  want 0xff000000  ok  [check 5]
1791013387.321 gl_test: case 5 scissor: PASS
1791013387.327 gl_test: FBO texture contents (via glReadPixels on the FBO):
1791013387.333 gl_test: px                                 
1791013387.338 gl_test: px                                 
1791013387.343 gl_test: px                                 
1791013387.353 gl_test: px                                 
1791013387.354 gl_test: px                                 
1791013387.357 gl_test: px                 R               
1791013387.357 gl_test: px                RR               
1791013387.366 gl_test: px                RRR              
1791013387.371 gl_test: px               RRRR              
1791013387.376 gl_test: px               RRRRR             
1791013387.376 gl_test: px              RRRRRR             
1791013387.382 gl_test: px              RRRRRRR            
1791013387.387 gl_test: px             RRRRRRRR            
1791013387.392 gl_test: px             RRRRRRRRR           
1791013387.397 gl_test: px            RRRRRRRRRR           
1791013387.397 gl_test: px            RRRRRRRRRRR          
1791013387.406 gl_test: px           GRRRRRRRRRRR          
1791013387.409 gl_test: px           GGRRRRRRRRBBB         
1791013387.409 gl_test: px          GGGGGRRRRRBBBB         
1791013387.415 gl_test: px          GGGGGGRRBBBBBBB        
1791013387.421 gl_test: px         GGGGGGGGBBBBBBBB        
1791013387.421 gl_test: px         GGGGGGGGBBBBBBBBB       
1791013387.426 gl_test: px        GGGGGGGGGBBBBBBBBB       
1791013387.431 gl_test: px        GGGGGGGGGBBBBBBBBBB      
1791013387.435 gl_test: px       GGGGGGGGGGBBBBBBBBBB      
1791013387.435 gl_test: px       GGGGGGGGGGBBBBBBBBBBB     
1791013387.440 gl_test: px      GGGGGGGGGGGBBBBBBBBBBB     
1791013387.445 gl_test: px      GGGGGGGGGGGBBBBBBBBBBBB    
1791013387.451 gl_test: px                                 
1791013387.458 gl_test: px                                 
1791013387.458 gl_test: px                                 
1791013387.466 gl_test: px                                 
1791013387.466 gl_test:   v0 red apex                  (31,12) = 0xff090fe7  want 0xff090fe7  ok  [check 1]
1791013387.474 gl_test:   v1 green                     (12,52) = 0xff0fde13  want 0xff0fde13  ok  [check 2]
1791013387.485 gl_test:   v2 blue                      (52,52) = 0xffe30913  want 0xffe30913  ok  [check 3]
1791013387.495 gl_test:   centroid                     (32,40) = 0xff595452  want 0xff595452  ok  [check 4]
1791013387.501 gl_test:   background                   ( 1, 1) = 0xff000000  want 0xff000000  ok  [check 5]
1791013387.510 gl_test: window contents (the texture sampled 1:1):
1791013387.510 gl_test: px                                 
1791013387.520 gl_test: px                                 
1791013387.526 gl_test: px                                 
1791013387.526 gl_test: px                                 
1791013387.531 gl_test: px                                 
1791013387.537 gl_test: px                 R               
1791013387.537 gl_test: px                RR               
1791013387.542 gl_test: px                RRR              
1791013387.547 gl_test: px               RRRR              
1791013387.547 gl_test: px               RRRRR             
1791013387.552 gl_test: px              RRRRRR             
1791013387.558 gl_test: px              RRRRRRR            
1791013387.563 gl_test: px             RRRRRRRR            
1791013387.569 gl_test: px             RRRRRRRRR           
1791013387.569 gl_test: px            RRRRRRRRRR           
1791013387.585 gl_test: px            RRRRRRRRRRR          
1791013387.586 gl_test: px           GRRRRRRRRRRR          
1791013387.586 gl_test: px           GGRRRRRRRRBBB         
1791013387.586 gl_test: px          GGGGGRRRRRBBBB         
1791013387.586 gl_test: px          GGGGGGRRBBBBBBB        
1791013387.590 gl_test: px         GGGGGGGGBBBBBBBB        
1791013387.595 gl_test: px         GGGGGGGGBBBBBBBBB       
1791013387.595 gl_test: px        GGGGGGGGGBBBBBBBBB       
1791013387.611 gl_test: px        GGGGGGGGGBBBBBBBBBB      
1791013387.611 gl_test: px       GGGGGGGGGGBBBBBBBBBB      
1791013387.611 gl_test: px       GGGGGGGGGGBBBBBBBBBBB     
1791013387.625 gl_test: px      GGGGGGGGGGGBBBBBBBBBBB     
1791013387.625 gl_test: px      GGGGGGGGGGGBBBBBBBBBBBB    
1791013387.631 gl_test: px                                 
1791013387.631 gl_test: px                                 
1791013387.631 gl_test: px                                 
1791013387.637 gl_test: px                                 
1791013387.646 gl_test:   v0 red apex                  (31,12) = 0xff090fe7  want 0xff090fe7  ok  [check 6]
1791013387.652 gl_test:   v1 green                     (12,52) = 0xff0fde13  want 0xff0fde13  ok  [check 7]
1791013387.661 gl_test:   v2 blue                      (52,52) = 0xffe30913  want 0xffe30913  ok  [check 8]
1791013387.672 gl_test:   centroid                     (32,40) = 0xff595452  want 0xff595452  ok  [check 9]
1791013387.676 gl_test:   background                   ( 1, 1) = 0xff000000  want 0xff000000  ok  [check 10]
1791013387.681 gl_test:   sampled == rendered          0 pixel(s) differ
1791013387.698 gl_test: case 6 fbo: PASS
1791013387.698 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.698 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.698 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.702 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.714 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.716 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.716 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.721 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.727 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.733 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.733 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.738 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.745 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.745 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.751 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.756 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.756 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.760 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.765 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.771 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.776 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.781 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.781 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.786 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.791 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.791 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.807 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.807 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.807 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.807 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.816 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.816 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013387.825 gl_test:   over: inside triangle        (32,40) = 0xbf008080  want 0xbf008080  ok  [check 1]
1791013387.835 gl_test:   over: outside (red quad)     ( 4, 4) = 0xff0000ff  want 0xff0000ff  ok  [check 2]
1791013387.845 gl_test:   additive: inside triangle    (32,40) = 0xff00ffff  want 0xff00ffff  ok  [check 3]
1791013387.861 gl_test:   premultiplied: inside        (32,40) = 0xff00ff80  want 0xff00ff80  ok  [check 4]
1791013387.872 gl_test:   replace: inside              (32,40) = 0x8000ff00  want 0x8000ff00  ok  [check 5]
1791013387.881 gl_test:   constant colour: inside      (32,40) = 0x800080bf  want 0x800080bf  ok  [check 6]
1791013387.893 gl_test:   mask ga: inside              (32,40) = 0x8000ffff  want 0x8000ffff  ok  [check 7]
1791013387.908 gl_test:   mask ga: outside untouched   ( 4, 4) = 0xff0000ff  want 0xff0000ff  ok  [check 8]
1791013387.923 gl_test:   over, alpha masked: inside   (32,40) = 0xff008080  want 0xff008080  ok  [check 9]
1791013387.923 gl_test: case 8 blend: PASS
1791013387.925 gl_test: px                                 
1791013387.925 gl_test: px                                 
1791013387.929 gl_test: px                                 
1791013387.986 gl_test: px                                 
1791013387.986 gl_test: px                                 
1791013387.986 gl_test: px                 R               
1791013388.031 gl_test: px                RR               
1791013388.031 gl_test: px                RRR              
1791013388.031 gl_test: px               RRRR              
1791013388.031 gl_test: px               RRRRR             
1791013388.031 gl_test: px              RRRRRR             
1791013388.031 gl_test: px              RRRRRRR            
1791013388.031 gl_test: px             RRRRRRRR            
1791013388.031 gl_test: px             RRRRRRRRR           
1791013388.031 gl_test: px            RRRRRRRRRR           
1791013388.031 gl_test: px            RRRRRRRRRRR          
1791013388.031 gl_test: px           RRRRRRRRRRRR          
1791013388.031 gl_test: px           RRRRRRRRRRRRR         
1791013388.035 gl_test: px          RRRRRRRRRRRRRR         
1791013388.042 gl_test: px          RRRRRRRRRRRRRRR        
1791013388.042 gl_test: px         RRRRRRRRRRRRRRRR        
1791013388.042 gl_test: px         RRRRRRRRRRRRRRRRR       
1791013388.042 gl_test: px        RRRRRRRRRRRRRRRRRR       
1791013388.042 gl_test: px        RRRRRRRRRRRRRRRRRRR      
1791013388.042 gl_test: px       RRRRRRRRRRRRRRRRRRRR      
1791013388.042 gl_test: px       RRRRRRRRRRRRRRRRRRRRR     
1791013388.042 gl_test: px      RRRRRRRRRRRRRRRRRRRRRR     
1791013388.042 gl_test: px      RRRRRRRRRRRRRRRRRRRRRRR    
1791013388.042 gl_test: px                                 
1791013388.042 gl_test: px                                 
1791013388.047 gl_test: px                                 
1791013388.047 gl_test: px                                 
1791013388.059 gl_test:   equal 1: inside mask         (32,40) = 0xff00ffff  want 0xff00ffff  ok  [check 1]
1791013388.064 gl_test:   equal 1: outside mask        ( 4, 4) = 0xff000000  want 0xff000000  ok  [check 2]
1791013388.069 gl_test:   mask pass wrote no colour    (32,50) = 0xff00ffff  want 0xff00ffff  ok  [check 3]
1791013388.085 gl_test:   notequal 1: inside mask      (32,40) = 0xff000000  want 0xff000000  ok  [check 4]
1791013388.089 gl_test:   notequal 1: outside mask     ( 4, 4) = 0xff00ffff  want 0xff00ffff  ok  [check 5]
1791013388.104 gl_test:   incr x2: inside == 2         (32,40) = 0xff00ffff  want 0xff00ffff  ok  [check 6]
1791013388.115 gl_test:   incr x2: outside == 0        ( 4, 4) = 0xff000000  want 0xff000000  ok  [check 7]
1791013388.130 gl_test:   cleared to 1: quad everywhere ( 4, 4) = 0xff00ffff  want 0xff00ffff  ok  [check 8]
1791013388.130 gl_test:   write mask 0xfe: nothing     (32,40) = 0xff000000  want 0xff000000  ok  [check 9]
1791013388.134 gl_test: case 9 stencil: PASS
1791013388.149 gl_test:   fan: centre                  (32,32) = 0xffff00ff  want 0xffff00ff  ok  [check 1]
1791013388.160 gl_test:   fan: corner                  ( 2,61) = 0xffff00ff  want 0xffff00ff  ok  [check 2]
1791013388.165 gl_test:   fan: corner                  (61, 2) = 0xffff00ff  want 0xffff00ff  ok  [check 3]
1791013388.180 gl_test:   u8 indices in a VBO          (32,32) = 0xffffff00  want 0xffffff00  ok  [check 4]
1791013388.190 gl_test:   u32 client indices           (32,32) = 0xff00ffff  want 0xff00ffff  ok  [check 5]
1791013388.205 gl_test:   ubyte colours in a VBO       (32,32) = 0xffff0000  want 0xffff0000  ok  [check 6]
1791013388.209 gl_test: px                                 
1791013388.216 gl_test: px                                 
1791013388.216 gl_test: px                                 
1791013388.225 gl_test: px                                 
1791013388.226 gl_test: px                                 
1791013388.226 gl_test: px                                 
1791013388.234 gl_test: px                                 
1791013388.235 gl_test: px                                 
1791013388.243 gl_test: px         GGGGGGGGGGGGGGGGG       
1791013388.243 gl_test: px         G               G       
1791013388.254 gl_test: px         G               G       
1791013388.255 gl_test: px         G               G       
1791013388.255 gl_test: px         G               G       
1791013388.259 gl_test: px         G               G       
1791013388.263 gl_test: px         G               G       
1791013388.270 gl_test: px         G               G       
1791013388.274 gl_test: px         G               G       
1791013388.274 gl_test: px         G               G       
1791013388.279 gl_test: px         G               G       
1791013388.284 gl_test: px         G               G       
1791013388.289 gl_test: px         G               G       
1791013388.289 gl_test: px         G               G       
1791013388.294 gl_test: px         G               G       
1791013388.299 gl_test: px         G               G       
1791013388.305 gl_test: px         GGGGGGGGGGGGGGGG        
1791013388.309 gl_test: px                                 
1791013388.317 gl_test: px                                 
1791013388.317 gl_test: px                                 
1791013388.327 gl_test: px                                 
1791013388.328 gl_test: px                                 
1791013388.328 gl_test: px                                 
1791013388.328 gl_test: px                                 
1791013388.345 gl_test:   loop: left edge              (16,32) = 0xff00ff00  want 0xff00ff00  ok  [check 7]
1791013388.351 gl_test:   loop: top edge               (32,16) = 0xff00ff00  want 0xff00ff00  ok  [check 8]
1791013388.356 gl_test:   loop: right edge             (48,32) = 0xff00ff00  want 0xff00ff00  ok  [check 9]
1791013388.368 gl_test:   loop: bottom edge (closing)  (32,48) = 0xff00ff00  want 0xff00ff00  ok  [check 10]
1791013388.369 gl_test:   loop: 3 px wide              (17,32) = 0xff00ff00  want 0xff00ff00  ok  [check 11]
1791013388.382 gl_test:   loop: inside empty           (32,32) = 0xff000000  want 0xff000000  ok  [check 12]
1791013388.393 gl_test:   loop: outside empty          ( 4, 4) = 0xff000000  want 0xff000000  ok  [check 13]
1791013388.402 gl_test:   lines: on the horizontal     (32,40) = 0xffffffff  want 0xffffffff  ok  [check 14]
1791013388.413 gl_test:   lines: row above empty       (32,38) = 0xff000000  want 0xff000000  ok  [check 15]
1791013388.418 gl_test:   lines: row below empty       (32,42) = 0xff000000  want 0xff000000  ok  [check 16]
1791013388.432 gl_test:   lines: beyond the end        (60,40) = 0xff000000  want 0xff000000  ok  [check 17]
1791013388.432 gl_test:   lines: on the vertical       (20,16) = 0xffffffff  want 0xffffffff  ok  [check 18]
1791013388.438 gl_test:   lines: column beside empty   (22,16) = 0xff000000  want 0xff000000  ok  [check 19]
1791013388.455 gl_test:   strip: first segment         ( 8,32) = 0xff0000ff  want 0xff0000ff  ok  [check 20]
1791013388.462 gl_test:   strip: second segment        (32,56) = 0xff0000ff  want 0xff0000ff  ok  [check 21]
1791013388.480 gl_test:   strip: not closed            (32, 8) = 0xff000000  want 0xff000000  ok  [check 22]
1791013388.480 gl_test:   3 px point at (40,24): .........
1791013388.481 gl_test:   3 px point at (40,24): .........
1791013388.481 gl_test:   3 px point at (40,24): .........
1791013388.485 gl_test:   3 px point at (40,24): ...###...
1791013388.491 gl_test:   3 px point at (40,24): ...###...
1791013388.502 gl_test:   3 px point at (40,24): ...###...
1791013388.502 gl_test:   3 px point at (40,24): .........
1791013388.507 gl_test:   3 px point at (40,24): .........
1791013388.512 gl_test:   3 px point at (40,24): .........
1791013388.517 gl_test:   1 px point: centre           ( 8, 8) = 0xffff0000  want 0xffff0000  ok  [check 23]
1791013388.525 gl_test:   1 px point: neighbour empty  ( 9, 8) = 0xff000000  want 0xff000000  ok  [check 24]
1791013388.533 gl_test:   1 px point: neighbour empty  ( 8, 9) = 0xff000000  want 0xff000000  ok  [check 25]
1791013388.539 gl_test:   3 px point: centre           (40,24) = 0xffff0000  want 0xffff0000  ok  [check 26]
1791013388.551 gl_test:   3 px point: corner           (41,25) = 0xffff0000  want 0xffff0000  ok  [check 27]
1791013388.560 gl_test:   3 px point: corner           (39,23) = 0xffff0000  want 0xffff0000  ok  [check 28]
1791013388.567 gl_test:   3 px point: outside          (42,24) = 0xff000000  want 0xff000000  ok  [check 29]
1791013388.576 gl_test:   3 px point: outside          (40,22) = 0xff000000  want 0xff000000  ok  [check 30]
1791013388.581 gl_test:   5 px point: edge             (58,56) = 0xffff0000  want 0xffff0000  ok  [check 31]
1791013388.594 gl_test:   5 px point: outside          (59,56) = 0xff000000  want 0xff000000  ok  [check 32]
1791013388.594 gl_test: case 10 prims: PASS
1791013388.603 gl_test:   GL_OES_vertex_array_object advertised = 1  want 1  ok  [check 1]
1791013388.611 gl_test:   glGenVertexArrays returns a nonzero name = 1  want 1  ok  [check 2]
1791013388.615 gl_test:   generated name is not a VAO yet = 0  want 0  ok  [check 3]
1791013388.620 gl_test:   bound name is a VAO          = 1  want 1  ok  [check 4]
1791013388.625 gl_test:   VERTEX_ARRAY_BINDING == vao  = 1  want 1  ok  [check 5]
1791013388.636 gl_test:   vao: centroid                (32,40) = 0xff595452  want 0xff595452  ok  [check 6]
1791013388.641 gl_test:   default VAO attr0 disabled   = 0  want 0  ok  [check 7]
1791013388.657 gl_test:   default VAO: centroid empty  (32,40) = 0xff000000  want 0xff000000  ok  [check 8]
1791013388.668 gl_test:   VAO restored attr0 enabled   = 1  want 1  ok  [check 9]
1791013388.672 gl_test:   vao rebound: centroid        (32,40) = 0xff595452  want 0xff595452  ok  [check 10]
1791013388.683 gl_test:   deleted name is not a VAO    = 0  want 0  ok  [check 11]
1791013388.683 gl_test: case 22 vao: PASS
1791013388.694 gl_test: px                                 
1791013388.694 gl_test: px                                 
1791013388.705 gl_test: px   R                             
1791013388.706 gl_test: px                                 
1791013388.708 gl_test: px                                 
1791013388.708 gl_test: px                 R               
1791013388.713 gl_test: px                RR               
1791013388.723 gl_test: px                RRR              
1791013388.723 gl_test: px               RRRR              
1791013388.728 gl_test: px               RRRRR             
1791013388.728 gl_test: px              RRRRRR             
1791013388.737 gl_test: px              RRRRRRR            
1791013388.740 gl_test: px             RRRRRRRR            
1791013388.745 gl_test: px             RRRRRRRRR           
1791013388.745 gl_test: px            RRRRRRRRRR           
1791013388.751 gl_test: px            RRRRRRRRRRR          
1791013388.757 gl_test: px           GRRRRRRRRRRR          
1791013388.761 gl_test: px           GGRRRRRRRRBBB         
1791013388.761 gl_test: px          GGGGGRRRRRBBBB         
1791013388.766 gl_test: px          GGGGGGRRBBBBBBB        
1791013388.778 gl_test: px         GGGGGGGGBBBBBBBB        
1791013388.778 gl_test: px         GGGGGGGGBBBBBBBBB       
1791013388.779 gl_test: px        GGGGGGGGGBBBBBBBBB       
1791013388.785 gl_test: px        GGGGGGGGGBBBBBBBBBB      
1791013388.785 gl_test: px       GGGGGGGGGGBBBBBBBBBB      
1791013388.789 gl_test: px       GGGGGGGGGGBBBBBBBBBBB     
1791013388.796 gl_test: px      GGGGGGGGGGGBBBBBBBBBBB     
1791013388.801 gl_test: px      GGGGGGGGGGGBBBBBBBBBBBB    
1791013388.801 gl_test: px                                 
1791013388.806 gl_test: px                                 
1791013388.818 gl_test: px                                 
1791013388.819 gl_test: px                                 
1791013388.825 gl_test:   earlier frame kept           (32,40) = 0xff595452  want 0xff595452  ok  [check 1]
1791013388.831 gl_test:   earlier frame kept           (31,12) = 0xff090fe7  want 0xff090fe7  ok  [check 2]
1791013388.841 gl_test:   new draw                     ( 4, 4) = 0xff00ffff  want 0xff00ffff  ok  [check 3]
1791013388.845 gl_test:   still clear elsewhere        (60, 4) = 0xff000000  want 0xff000000  ok  [check 4]
1791013388.875 gl_test:   no-clear 113: draw 112       (40,24) = 0xff00ffff  want 0xff00ffff  ok  [check 5]
1791013388.885 gl_test:   no-clear 113: earlier kept   (32,40) = 0xff595452  want 0xff595452  ok  [check 6]
1791013388.918 gl_test: px                                 
1791013388.918 gl_test: px                                 
1791013388.922 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013388.930 gl_test: px                                 
1791013388.942 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013388.942 gl_test: px                                 
1791013388.942 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013388.945 gl_test: px                                 
1791013388.950 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013388.955 gl_test: px                                 
1791013388.961 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013388.967 gl_test: px                                 
1791013388.967 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013388.973 gl_test: px                                 
1791013388.982 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013388.982 gl_test: px                                 
1791013388.987 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013388.994 gl_test: px                                 
1791013388.994 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013388.995 gl_test: px                                 
1791013388.995 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013389.001 gl_test: px                                 
1791013389.006 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013389.011 gl_test: px                                 
1791013389.017 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013389.023 gl_test: px                                 
1791013389.023 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013389.028 gl_test: px                                 
1791013389.033 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013389.033 gl_test: px                                 
1791013389.050 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013389.050 gl_test: px                                 
1791013389.050 gl_test:   draw 0                       ( 4, 4) = 0xff00ffff  want 0xff00ffff  ok  [check 7]
1791013389.057 gl_test:   draw 255 (last of kick 1)    (49,52) = 0xff00ffff  want 0xff00ffff  ok  [check 8]
1791013389.067 gl_test:   draw 256 (first of kick 2)   (52,52) = 0xff00ffff  want 0xff00ffff  ok  [check 9]
1791013389.077 gl_test:   draw 299                     (61,60) = 0xff00ffff  want 0xff00ffff  ok  [check 10]
1791013389.089 gl_test:   triangle from frame 1 cleared (32,40) = 0xff000000  want 0xff000000  ok  [check 11]
1791013389.089 gl_test: case 11 persist: PASS
1791013389.094 gl_test: px                                 
1791013389.102 gl_test: px                                 
1791013389.103 gl_test: px                                 
1791013389.109 gl_test: px                                 
1791013389.114 gl_test: px                                 
1791013389.114 gl_test: px                                 
1791013389.119 gl_test: px                                 
1791013389.123 gl_test: px                                 
1791013389.129 gl_test: px                                 
1791013389.129 gl_test: px         R               G       
1791013389.134 gl_test: px        RR              GG       
1791013389.139 gl_test: px        RRR             GGG      
1791013389.147 gl_test: px        RRR             GGG      
1791013389.156 gl_test: px       RRRR            GGGG      
1791013389.156 gl_test: px       RRRRR           GGGGG     
1791013389.156 gl_test: px      RRRRRR          GGGGGG     
1791013389.160 gl_test: px      RRRRRRR         GGGGGGG    
1791013389.166 gl_test: px      RRRRRRR         GGGGGGG    
1791013389.166 gl_test: px     RRRRRRRR        GGGGGGGG    
1791013389.176 gl_test: px     RRRRRRRRR       GGGGGGGGG   
1791013389.176 gl_test: px    RRRRRRRRRR      GGGGGGGGGG   
1791013389.181 gl_test: px    RRRRRRRRRRR     GGGGGGGGGGG  
1791013389.181 gl_test: px    RRRRRRRRRRR     GGGGGGGGGGG  
1791013389.187 gl_test: px   RRRRRRRRRRRR    GGGGGGGGGGGG  
1791013389.192 gl_test: px                                 
1791013389.197 gl_test: px                                 
1791013389.197 gl_test: px                                 
1791013389.201 gl_test: px                                 
1791013389.211 gl_test: px                                 
1791013389.212 gl_test: px                                 
1791013389.215 gl_test: px                                 
1791013389.220 gl_test: px                                 
1791013389.233 gl_test:   draw 1 keeps orange          (16,40) = 0xff0080ff  want 0xff0080ff  ok  [check 1]
1791013389.238 gl_test:   draw 2 is cyan               (48,40) = 0xffffff00  want 0xffffff00  ok  [check 2]
1791013389.238 gl_test: case 34 unishare: PASS
1791013389.249 gl_test: px                                 
1791013389.250 gl_test: px                                 
1791013389.250 gl_test: px                                 
1791013389.254 gl_test: px                                 
1791013389.268 gl_test: px         R               B       
1791013389.270 gl_test: px         R               B       
1791013389.270 gl_test: px        RR              BB       
1791013389.275 gl_test: px        RR              BB       
1791013389.280 gl_test: px        RRR             BBB      
1791013389.280 gl_test: px        RRR             BBB      
1791013389.285 gl_test: px       RRRR            BBBB      
1791013389.291 gl_test: px       RRRR            BBBB      
1791013389.297 gl_test: px       RRRRR           BBBBB     
1791013389.302 gl_test: px       RRRRR           BBBBB     
1791013389.302 gl_test: px      RRRRRR          BBBBBB     
1791013389.307 gl_test: px      RRRRRR          BBBBBB     
1791013389.307 gl_test: px      RRRRRRR         BBBBBBB    
1791013389.311 gl_test: px      RRRRRRR         BBBBBBB    
1791013389.320 gl_test: px     RRRRRRRR        BBBBBBBB    
1791013389.322 gl_test: px     RRRRRRRR        BBBBBBBB    
1791013389.326 gl_test: px     RRRRRRRRR       BBBBBBBBB   
1791013389.326 gl_test: px     RRRRRRRRR       BBBBBBBBB   
1791013389.339 gl_test: px    RRRRRRRRRR      BBBBBBBBBB   
1791013389.339 gl_test: px    RRRRRRRRRR      BBBBBBBBBB   
1791013389.344 gl_test: px    RRRRRRRRRRR     BBBBBBBBBBB  
1791013389.344 gl_test: px    RRRRRRRRRRR     BBBBBBBBBBB  
1791013389.352 gl_test: px   RRRRRRRRRRRR    BBBBBBBBBBBB  
1791013389.354 gl_test: px   RRRRRRRRRRRR    BBBBBBBBBBBB  
1791013389.359 gl_test: px   RRRRRRRRRRRRR   BBBBBBBBBBBBB 
1791013389.359 gl_test: px                                 
1791013389.364 gl_test: px                                 
1791013389.369 gl_test: px                                 
1791013389.381 gl_test:   ccw front: left (front) == 1 red (16,40) = 0xff0000ff  want 0xff0000ff  ok  [check 1]
1791013389.388 gl_test:   ccw front: right (back) == 2 blue (48,40) = 0xffff0000  want 0xffff0000  ok  [check 2]
1791013389.399 gl_test:   ccw front: outside untouched (32, 4) = 0xff000000  want 0xff000000  ok  [check 3]
1791013389.409 gl_test:   cw front: left (back) == 2 blue (16,40) = 0xffff0000  want 0xffff0000  ok  [check 4]
1791013389.415 gl_test:   cw front: right (front) == 1 red (48,40) = 0xff0000ff  want 0xff0000ff  ok  [check 5]
1791013389.428 gl_test:   fbo ccw front: left == 1 red (16,40) = 0xff0000ff  want 0xff0000ff  ok  [check 6]
1791013389.435 gl_test:   fbo ccw front: right == 2 blue (48,40) = 0xffff0000  want 0xffff0000  ok  [check 7]
1791013389.440 gl_test: case 12 stencil2: PASS
1791013389.445 gl_test: px                                 
1791013389.454 gl_test: px                                 
1791013389.455 gl_test: px                                 
1791013389.461 gl_test: px                                 
1791013389.461 gl_test: px                 R               
1791013389.471 gl_test: px                RR               
1791013389.471 gl_test: px                RRR              
1791013389.475 gl_test: px               RRRR              
1791013389.475 gl_test: px               RRRRR             
1791013389.486 gl_test: px              RRRRRR             
1791013389.486 gl_test: px             RRRRRRRR            
1791013389.495 gl_test: px             RRRR RRRR           
1791013389.495 gl_test: px            RRRR  RRRR           
1791013389.499 gl_test: px            RRRR   RRRR          
1791013389.503 gl_test: px           RRRR    RRRR          
1791013389.508 gl_test: px           RRRR     RRRR         
1791013389.513 gl_test: px          RRRR       RRR         
1791013389.513 gl_test: px          RRR        RRRR        
1791013389.519 gl_test: px         RRRRRRRRRRRRRRRR        
1791013389.524 gl_test: px         RRRRRRRRRRRRRRRRR       
1791013389.529 gl_test: px        RRRRRRRRRRRRRRRRRR       
1791013389.529 gl_test: px        RRRRRRRRRRRRRRRRRRR      
1791013389.539 gl_test: px       RRRRRRRRRRRRRRRRRRRR      
1791013389.542 gl_test: px       RRRRRRRRRRRRRRRRRRRRR     
1791013389.547 gl_test: px      RRRRRRRRRRRRRRRRRRRRRR     
1791013389.547 gl_test: px     RRRRRRRRRRRRRRRRRRRRRRRR    
1791013389.556 gl_test: px     RRRRRRRRRRRRRRRRRRRRRRRRR   
1791013389.557 gl_test: px                                 
1791013389.560 gl_test: px                                 
1791013389.565 gl_test: px                                 
1791013389.565 gl_test: px                                 
1791013389.571 gl_test: px                                 
1791013389.583 gl_test:   winding: outer filled        (19,42) = 0xff00ffff  want 0xff00ffff  ok  [check 1]
1791013389.587 gl_test:   winding: hole empty          (32,30) = 0xff000000  want 0xff000000  ok  [check 2]
1791013389.591 gl_test:   winding: outside empty       ( 4, 4) = 0xff000000  want 0xff000000  ok  [check 3]
1791013389.596 gl_test: case 23 winding: PASS
1791013389.612 gl_test:   clear 0.5: mid quad passes   (32,32) = 0xffff0000  want 0xffff0000  ok  [check 1]
1791013389.618 gl_test:   clear 0.5: far quad fails    ( 4, 4) = 0xffff0000  want 0xffff0000  ok  [check 2]
1791013389.637 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013389.647 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013389.647 gl_test: px GGRGGRGGRGGRGGRGRRGGRGGRGGRGGRGG
1791013389.659 gl_test: px GGGGGGGGGGGGGGGRRGGGGGGGGGGGGGGG
1791013389.659 gl_test: px GGRGGRGGRGGRGGRRRRGGRGGRGGRGGRGG
1791013389.660 gl_test: px GGGGGGGGGGGGGGRRRRGGGGGGGGGGGGGG
1791013389.660 gl_test: px GGRGGRGGRGGRGGRRRRRGRGGRGGRGGRGG
1791013389.669 gl_test: px GGGGGGGGGGGGGRRRRRRGGGGGGGGGGGGG
1791013389.673 gl_test: px GGRGGRGGRGGRGRRRRRRRRGGRGGRGGRGG
1791013389.679 gl_test: px GGGGGGGGGGGGRRRRRRRRGGGGGGGGGGGG
1791013389.679 gl_test: px GGRGGRGGRGGRRRRRRRRRRGGRGGRGGRGG
1791013389.683 gl_test: px GGGGGGGGGGGRRRRRRRRRRGGGGGGGGGGG
1791013389.689 gl_test: px GGRGGRGGRGGRRRRRRRRRRRGRGGRGGRGG
1791013389.694 gl_test: px GGGGGGGGGGRRRRRRRRRRRRGGGGGGGGGG
1791013389.699 gl_test: px GGRGGRGGRGRRRRRRRRRRRRRRGGRGGRGG
1791013389.699 gl_test: px GGGGGGGGGRRRRRRRRRRRRRRGGGGGGGGG
1791013389.704 gl_test: px GGGGGGGGGRRRRRRRRRRRRRRRGGGGGGGG
1791013389.710 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013389.719 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRRGGGGGGG
1791013389.720 gl_test: px GGGGGGGRRRRRRRRRRRRRRRRRRGGGGGGG
1791013389.720 gl_test: px GGGGGGGRRRRRRRRRRRRRRRRRRRGGGGGG
1791013389.725 gl_test: px GGGGGGRRRRRRRRRRRRRRRRRRRRGGGGGG
1791013389.731 gl_test: px GGGGGGRRRRRRRRRRRRRRRRRRRRRGGGGG
1791013389.737 gl_test: px GGGGGRRRRRRRRRRRRRRRRRRRRRRGGGGG
1791013389.745 gl_test: px GGGGGRRRRRRRRRRRRRRRRRRRRRRRGGGG
1791013389.745 gl_test: px GGGGRRRRRRRRRRRRRRRRRRRRRRRRGGGG
1791013389.754 gl_test: px GGGGRRRRRRRRRRRRRRRRRRRRRRRRRGGG
1791013389.754 gl_test: px GGGRRRRRRRRRRRRRRRRRRRRRRRRRRGGG
1791013389.761 gl_test: px GGGRRRRRRRRRRRRRRRRRRRRRRRRRRRGG
1791013389.762 gl_test: px GGRRRRRRRRRRRRRRRRRRRRRRRRRRRRGG
1791013389.762 gl_test: px GGRRRRRRRRRRRRRRRRRRRRRRRRRRRRRG
1791013389.762 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013389.777 gl_test:   split: near triangle kept    (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 3]
1791013389.785 gl_test:   split: kick-1 small tri kept ( 4, 4) = 0xff00ffff  want 0xff00ffff  ok  [check 4]
1791013389.790 gl_test:   split: kick-2 small tri      (61,28) = 0xff00ffff  want 0xff00ffff  ok  [check 5]
1791013389.794 gl_test:   split: far quad elsewhere    (60,40) = 0xff00ff00  want 0xff00ff00  ok  [check 6]
1791013389.810 gl_test:   stencil kept: inside         (32,32) = 0xff00ff00  want 0xff00ff00  ok  [check 7]
1791013389.821 gl_test:   stencil kept: outside        ( 4, 4) = 0xff000000  want 0xff000000  ok  [check 8]
1791013389.832 gl_test:   depth cleared mid-frame: far quad wins (32,32) = 0xff00ff00  want 0xff00ff00  ok  [check 9]
1791013389.844 gl_test:   depth range 0..0.3: far quad wins (32,32) = 0xff00ff00  want 0xff00ff00  ok  [check 10]
1791013389.849 gl_test: case 13 zls: PASS
1791013389.869 gl_test:   client strip: quad 0         ( 6,32) = 0xff0000ff  want 0xff0000ff  ok  [check 1]
1791013389.870 gl_test:   client strip: quad 1         (16,32) = 0xff00ff00  want 0xff00ff00  ok  [check 2]
1791013389.870 gl_test:   client strip: quad 2         (26,32) = 0xffff0000  want 0xffff0000  ok  [check 3]
1791013389.881 gl_test:   client strip: quad 3         (36,32) = 0xff00ffff  want 0xff00ffff  ok  [check 4]
1791013389.891 gl_test:   client strip: quad 4         (46,32) = 0xffffff00  want 0xffffff00  ok  [check 5]
1791013389.901 gl_test:   client strip: quad 5         (56,32) = 0xffff00ff  want 0xffff00ff  ok  [check 6]
1791013389.906 gl_test:   client strip: gap stays clear (11,32) = 0xff000000  want 0xff000000  ok  [check 7]
1791013389.916 gl_test:   ibo strip: quad 0            ( 6,32) = 0xff0000ff  want 0xff0000ff  ok  [check 8]
1791013389.928 gl_test:   ibo strip: quad 1            (16,32) = 0xff00ff00  want 0xff00ff00  ok  [check 9]
1791013389.933 gl_test:   ibo strip: quad 2            (26,32) = 0xffff0000  want 0xffff0000  ok  [check 10]
1791013389.948 gl_test:   ibo strip: quad 3            (36,32) = 0xff00ffff  want 0xff00ffff  ok  [check 11]
1791013389.948 gl_test:   ibo strip: quad 4            (46,32) = 0xffffff00  want 0xffffff00  ok  [check 12]
1791013389.973 gl_test:   ibo strip: quad 5            (56,32) = 0xffff00ff  want 0xffff00ff  ok  [check 13]
1791013389.973 gl_test: case 40 qtstrip: PASS
1791013389.974 gl_test:   constant attribs: cyan       (32,32) = 0xffffff00  want 0xffffff00  ok  [check 1]
1791013389.981 gl_test:   current attrib 1.g           = 1  want 1  ok  [check 2]
1791013389.988 gl_test:   current attrib 1.r           = 0  want 0  ok  [check 3]
1791013390.001 gl_test: 8x8 texture @0x8001f3a000: ff0000ff ff0000ff ff0000ff ff0000ff | row1 ff0000ff ff0000ff
1791013390.007 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013390.011 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013390.011 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013390.021 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013390.021 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013390.025 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013390.031 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013390.031 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013390.036 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013390.041 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013390.041 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013390.046 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013390.057 gl_test: px RRRRRRRRRRRRRRRRWWWWWWWWWWWWWWWW
1791013390.057 gl_test: px RRRRRRRRRRRRRRRRWWWWWWWWWWWWWWWW
1791013390.057 gl_test: px RRRRRRRRRRRRRRRRWWWWWWWWWWWWWWWW
1791013390.058 gl_test: px RRRRRRRRRRRRRRRRWWWWWWWWWWWWWWWW
1791013390.068 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013390.069 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013390.080 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013390.080 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013390.162 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013390.162 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013390.176 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013390.177 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013390.177 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013390.177 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013390.177 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013390.177 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013390.177 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013390.177 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013390.177 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013390.177 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013390.177 gl_test:   sub-image rgba: magenta      (16,16) = 0xffff00ff  want 0xffff00ff  ok  [check 4]
1791013390.177 gl_test:   sub-image rgb align 1: yellow (52,12) = 0xff00ffff  want 0xff00ffff  ok  [check 5]
1791013390.177 gl_test:   sub-image rgb align 4: white (12,52) = 0xffffffff  want 0xffffffff  ok  [check 6]
1791013390.177 gl_test:   untouched quadrant: green    (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 7]
1791013390.177 gl_test:   copied: magenta              (16,16) = 0xffff00ff  want 0xffff00ff  ok  [check 8]
1791013390.178 gl_test:   copied: yellow               (52,12) = 0xff00ffff  want 0xff00ffff  ok  [check 9]
1791013390.193 gl_test:   copied: green                (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 10]
1791013390.203 gl_test:   sub-copied: white patch      (48,16) = 0xffffffff  want 0xffffffff  ok  [check 11]
1791013390.215 gl_test:   sub-copied: red              (60, 4) = 0xff0000ff  want 0xff0000ff  ok  [check 12]
1791013390.220 gl_test:   depth range far              = 1  want 1  ok  [check 13]
1791013390.227 gl_test:   depth writemask              = 1  want 1  ok  [check 14]
1791013390.232 gl_test:   stencil writemask (all ones) = 4294967295  want 4294967295  ok  [check 15]
1791013390.237 gl_test:   glIsTexture(tex2)            = 1  want 1  ok  [check 16]
1791013390.242 gl_test:   glIsTexture(999)             = 0  want 0  ok  [check 17]
1791013390.250 gl_test:   glIsProgram(prog)            = 1  want 1  ok  [check 18]
1791013390.259 gl_test:   glIsShader(0)                = 0  want 0  ok  [check 19]
1791013390.259 gl_test:   buffer size                  = 144  want 144  ok  [check 20]
1791013390.264 gl_test:   attached shaders             = 2  want 2  ok  [check 21]
1791013390.275 gl_test:   attached vs is a shader      = 1  want 1  ok  [check 22]
1791013390.287 gl_test:   shader source prefix         = 35  want 35  ok  [check 23]
1791013390.288 gl_test:   shader source chunk          = 7  want 7  ok  [check 24]
1791013390.288 gl_test:   tex mag filter               = 9728  want 9728  ok  [check 25]
1791013390.297 gl_test:   highp float precision        = 23  want 23  ok  [check 26]
1791013390.301 gl_test: case 14 api: PASS
1791013390.301 I: yakogl: program 48 has no uniform 'u_tex0'
1791013390.313 gl_test:   unused sampler u_tex0 optimized out (loc -1) = -1  want -1  ok  [check 1]
1791013390.318 gl_test:   used sampler u_tex1 has a location = 1  want 1  ok  [check 2]
1791013390.333 gl_test:   compacted sampler reads unit 1 (green) (32,32) = 0xff00ff00  want 0xff00ff00  ok  [check 3]
1791013390.333 gl_test: case 24 compact: PASS
1791013390.338 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013390.344 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013390.349 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013390.354 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013390.359 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013390.359 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013390.364 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013390.375 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013390.376 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013390.377 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013390.383 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013390.383 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013390.387 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013390.395 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013390.398 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013390.405 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013390.405 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013390.414 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013390.414 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013390.420 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013390.423 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013390.423 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013390.428 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013390.435 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013390.440 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013390.440 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013390.458 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013390.459 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013390.459 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013390.459 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013390.459 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013390.460 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013390.478 gl_test:   inside the box               (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 1]
1791013390.481 gl_test:   inside, at a corner          (17,17) = 0xff0000ff  want 0xff0000ff  ok  [check 2]
1791013390.491 gl_test:   left of the box              ( 8,32) = 0xffff0000  want 0xffff0000  ok  [check 3]
1791013390.497 gl_test:   right of the box             (56,32) = 0xffff0000  want 0xffff0000  ok  [check 4]
1791013390.514 gl_test:   one row above the box        (32,15) = 0xffff0000  want 0xffff0000  ok  [check 5]
1791013390.514 gl_test:   one row below the box        (32,48) = 0xffff0000  want 0xffff0000  ok  [check 6]
1791013390.516 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.526 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.531 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.536 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.536 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.541 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.546 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.551 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.556 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013390.556 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013390.563 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013390.568 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013390.580 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013390.580 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013390.581 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013390.583 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013390.583 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013390.588 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013390.593 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013390.597 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013390.603 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013390.603 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013390.608 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013390.626 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013390.629 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.629 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.629 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.629 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.629 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.633 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.638 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.648 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.653 gl_test:   the clear is after the draw  (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 7]
1791013390.659 gl_test:   the draw is everywhere else  ( 8,32) = 0xff00ff00  want 0xff00ff00  ok  [check 8]
1791013390.675 gl_test:   an empty box clears nothing  (32,32) = 0xffff0000  want 0xffff0000  ok  [check 9]
1791013390.675 gl_test: case 58 clearscis: PASS
1791013390.691 gl_test:   a fully masked colour clear does nothing (32,32) = 0xffff0000  want 0xffff0000  ok  [check 1]
1791013390.696 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.701 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.701 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.707 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.712 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.717 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.717 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.721 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.737 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.738 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.738 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.738 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.741 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.746 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.751 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.756 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.762 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.767 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.767 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.773 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.773 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.785 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.786 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.788 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.788 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.795 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.801 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.805 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.805 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.809 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.816 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.819 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013390.829 gl_test:   a masked depth clear does nothing (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 2]
1791013390.846 gl_test:   the masked bits were not cleared (32,32) = 0xffff0000  want 0xffff0000  ok  [check 3]
1791013390.846 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.846 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.852 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.859 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.865 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.871 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.871 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.875 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.879 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.888 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.889 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.895 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.895 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.901 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.906 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.906 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.911 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.917 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.917 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.923 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.927 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.931 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.931 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.948 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.948 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.950 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.950 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.950 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.959 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.966 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.966 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.971 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013390.980 gl_test:   the allowed bits were cleared (32,32) = 0xff00ff00  want 0xff00ff00  ok  [check 4]
1791013390.990 gl_test: case 59 clearmask: PASS
1791013390.990 gl_test: wrap S = repeat:
1791013390.990 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.041 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.047 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.047 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.047 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.047 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.047 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.047 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.047 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.047 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.047 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.047 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.047 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.047 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.092 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.092 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.099 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.105 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.105 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.105 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.105 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.105 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.105 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.105 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.105 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.105 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.105 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.105 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.111 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.111 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.115 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.120 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013391.125 gl_test:   S first period               ( 8,48) = 0xff0000ff  want 0xff0000ff  ok  [check 1]
1791013391.135 gl_test:   S second period, u 1.25      (40,48) = 0xff0000ff  want 0xff0000ff  ok  [check 2]
1791013391.140 gl_test:   S second period, u 1.75      (56,48) = 0xff00ff00  want 0xff00ff00  ok  [check 3]
1791013391.151 gl_test: wrap S = mirror:
1791013391.151 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.156 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.165 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.165 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.165 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.169 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.173 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.183 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.184 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.184 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.197 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.197 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.197 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.201 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.206 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.211 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.217 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.222 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.229 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.229 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.233 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.233 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.237 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.242 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.247 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.255 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.257 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.257 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.262 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.273 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.274 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.276 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013391.281 gl_test:   S first period               ( 8,48) = 0xff0000ff  want 0xff0000ff  ok  [check 4]
1791013391.306 gl_test:   S second period, u 1.25      (40,48) = 0xff00ff00  want 0xff00ff00  ok  [check 5]
1791013391.307 gl_test:   S second period, u 1.75      (56,48) = 0xff0000ff  want 0xff0000ff  ok  [check 6]
1791013391.307 gl_test: wrap S = clamp:
1791013391.309 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.315 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.319 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.325 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.325 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.329 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.334 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.334 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.345 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.345 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.349 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.355 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.359 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.364 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.364 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.369 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.374 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.382 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.383 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.383 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.389 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.400 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.400 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.408 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.408 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.409 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.411 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.418 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.424 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.424 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.428 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.428 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013391.443 gl_test:   S first period               ( 8,48) = 0xff0000ff  want 0xff0000ff  ok  [check 7]
1791013391.448 gl_test:   S second period, u 1.25      (40,48) = 0xff00ff00  want 0xff00ff00  ok  [check 8]
1791013391.459 gl_test:   S second period, u 1.75      (56,48) = 0xff00ff00  want 0xff00ff00  ok  [check 9]
1791013391.463 gl_test: wrap T = repeat:
1791013391.469 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.472 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.477 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.477 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.482 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.491 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.491 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.502 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.503 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.504 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.504 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.509 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.521 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.522 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.525 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.530 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.530 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.535 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.535 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.539 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.545 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.549 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.554 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.561 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.561 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.566 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.571 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.576 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.576 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.581 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.586 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.586 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.606 gl_test:   T first period               ( 8,55) = 0xff0000ff  want 0xff0000ff  ok  [check 10]
1791013391.606 gl_test:   T second period, v 1.25      ( 8,23) = 0xff0000ff  want 0xff0000ff  ok  [check 11]
1791013391.613 gl_test:   T second period, v 1.75      ( 8, 7) = 0xffff0000  want 0xffff0000  ok  [check 12]
1791013391.618 gl_test: wrap T = mirror:
1791013391.634 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.634 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.635 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.637 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.641 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.644 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.649 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.654 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.658 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.663 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.668 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.668 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.673 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.678 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.678 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.686 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.693 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.698 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.698 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.713 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.713 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.714 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.714 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.714 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.717 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.721 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.727 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.733 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.737 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.737 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.746 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.749 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.757 gl_test:   T first period               ( 8,55) = 0xff0000ff  want 0xff0000ff  ok  [check 13]
1791013391.767 gl_test:   T second period, v 1.25      ( 8,23) = 0xffff0000  want 0xffff0000  ok  [check 14]
1791013391.772 gl_test:   T second period, v 1.75      ( 8, 7) = 0xff0000ff  want 0xff0000ff  ok  [check 15]
1791013391.783 gl_test: wrap T = clamp:
1791013391.783 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.788 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.788 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.798 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.798 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.809 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.809 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.817 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.817 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.817 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.820 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.825 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.829 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.835 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.839 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.845 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.850 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.850 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.859 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.860 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.860 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.867 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.871 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.871 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013391.877 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.881 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.887 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.892 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.892 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.897 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.902 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.902 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013391.916 gl_test:   T first period               ( 8,55) = 0xff0000ff  want 0xff0000ff  ok  [check 16]
1791013391.926 gl_test:   T second period, v 1.25      ( 8,23) = 0xffff0000  want 0xffff0000  ok  [check 17]
1791013391.931 gl_test:   T second period, v 1.75      ( 8, 7) = 0xffff0000  want 0xffff0000  ok  [check 18]
1791013391.931 gl_test: case 60 wrapmode: PASS
1791013391.947 gl_test:   complete: the texel          (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 1]
1791013391.962 I: a texture on unit 0 is incomplete and samples (0,0,0,1): a mipmap min filter, but fewer levels uploaded than the chain needs (target 0xde1, 2x2, levels 1 of 2, min filter 0x2703, wrap 0x2901/0x2901)
1791013391.972 gl_test: px                                 
1791013391.978 gl_test: px                                 
1791013391.978 gl_test: px                                 
1791013391.987 gl_test: px                                 
1791013391.987 gl_test: px                                 
1791013391.987 gl_test: px                                 
1791013391.999 gl_test: px                                 
1791013391.999 gl_test: px                                 
1791013392.003 gl_test: px                                 
1791013392.014 gl_test: px                                 
1791013392.015 gl_test: px                                 
1791013392.015 gl_test: px                                 
1791013392.018 gl_test: px                                 
1791013392.024 gl_test: px                                 
1791013392.033 gl_test: px                                 
1791013392.033 gl_test: px                                 
1791013392.036 gl_test: px                                 
1791013392.040 gl_test: px                                 
1791013392.048 gl_test: px                                 
1791013392.053 gl_test: px                                 
1791013392.053 gl_test: px                                 
1791013392.058 gl_test: px                                 
1791013392.063 gl_test: px                                 
1791013392.063 gl_test: px                                 
1791013392.068 gl_test: px                                 
1791013392.073 gl_test: px                                 
1791013392.073 gl_test: px                                 
1791013392.084 gl_test: px                                 
1791013392.084 gl_test: px                                 
1791013392.084 gl_test: px                                 
1791013392.086 gl_test: px                                 
1791013392.097 gl_test: px                                 
1791013392.107 gl_test:   incomplete: opaque black     (16,48) = 0xff000000  want 0xff000000  ok  [check 2]
1791013392.116 gl_test:   incomplete: opaque black     (48,16) = 0xff000000  want 0xff000000  ok  [check 3]
1791013392.120 gl_test:   complete again               (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 4]
1791013392.135 gl_test:   nothing bound: opaque black  (16,48) = 0xff000000  want 0xff000000  ok  [check 5]
1791013392.144 gl_test:   nothing bound: opaque black  (48,16) = 0xff000000  want 0xff000000  ok  [check 6]
1791013392.144 gl_test: case 61 texincomp: PASS
1791013392.149 I: osal(zephyr): ready (regs=0xfd80000)
1791013392.154 gl_test:   eglInitialize                = 1  want 1  ok  [check 1]
1791013392.159 gl_test:   eglChooseConfig              = 1  want 1  ok  [check 2]
1791013392.165 gl_test:   one config                   = 1  want 1  ok  [check 3]
1791013392.176 gl_test:   no 8x MSAA config            = 1  want 1  ok  [check 4]
1791013392.176 gl_test:   config depth size            = 24  want 24  ok  [check 5]
1791013392.186 gl_test:   eglQueryString VENDOR        = 1  want 1  ok  [check 6]
1791013392.186 gl_test:   pbuffer created              = 1  want 1  ok  [check 7]
1791013392.197 gl_test:   context created              = 1  want 1  ok  [check 8]
1791013392.211 gl_test:   eglMakeCurrent               = 1  want 1  ok  [check 9]
1791013392.219 gl_test:   current context              = 1  want 1  ok  [check 10]
1791013392.225 gl_test:   pbuffer width                = 64  want 64  ok  [check 11]
1791013392.232 gl_test:   eglGetProcAddress            = 1  want 1  ok  [check 12]
1791013392.237 gl_test:   eglSwapBuffers               = 1  want 1  ok  [check 13]
1791013392.241 gl_test: px                                 
1791013392.250 gl_test: px                                 
1791013392.251 gl_test: px                                 
1791013392.255 gl_test: px                                 
1791013392.255 gl_test: px                                 
1791013392.260 gl_test: px                 R               
1791013392.270 gl_test: px                RR               
1791013392.275 gl_test: px                RRR              
1791013392.275 gl_test: px               RRRR              
1791013392.280 gl_test: px               RRRRR             
1791013392.280 gl_test: px              RRRRRR             
1791013392.285 gl_test: px              RRRRRRR            
1791013392.291 gl_test: px             RRRRRRRR            
1791013392.296 gl_test: px             RRRRRRRRR           
1791013392.301 gl_test: px            RRRRRRRRRR           
1791013392.309 gl_test: px            RRRRRRRRRRR          
1791013392.309 gl_test: px           GRRRRRRRRRRR          
1791013392.311 gl_test: px           GGRRRRRRRRBBB         
1791013392.321 gl_test: px          GGGGGRRRRRBBBB         
1791013392.322 gl_test: px          GGGGGGRRBBBBBBB        
1791013392.322 gl_test: px         GGGGGGGGBBBBBBBB        
1791013392.326 gl_test: px         GGGGGGGGBBBBBBBBB       
1791013392.331 gl_test: px        GGGGGGGGGBBBBBBBBB       
1791013392.337 gl_test: px        GGGGGGGGGBBBBBBBBBB      
1791013392.341 gl_test: px       GGGGGGGGGGBBBBBBBBBB      
1791013392.341 gl_test: px       GGGGGGGGGGBBBBBBBBBBB     
1791013392.346 gl_test: px      GGGGGGGGGGGBBBBBBBBBBB     
1791013392.359 gl_test: px      GGGGGGGGGGGBBBBBBBBBBBB    
1791013392.359 gl_test: px                                 
1791013392.359 gl_test: px                                 
1791013392.360 gl_test: px                                 
1791013392.365 gl_test: px                                 
1791013392.377 gl_test:   egl pbuffer: centroid        (32,40) = 0xff595452  want 0xff595452  ok  [check 14]
1791013392.382 gl_test:   egl pbuffer: background      ( 1, 1) = 0xff000000  want 0xff000000  ok  [check 15]
1791013392.392 gl_test:   release                      = 1  want 1  ok  [check 16]
1791013392.392 gl_test:   no current context           = 1  want 1  ok  [check 17]
1791013392.403 gl_test:   eglDestroySurface            = 1  want 1  ok  [check 18]
1791013392.407 gl_test:   eglDestroyContext            = 1  want 1  ok  [check 19]
1791013392.412 gl_test:   eglTerminate                 = 1  want 1  ok  [check 20]
1791013392.431 gl_test:   eglGetError clean            = 12288  want 12288  ok  [check 21]
1791013392.431 gl_test: case 15 egl: PASS
1791013392.431 I: osal(zephyr): ready (regs=0xfd80000)
1791013392.431 gl_test:   eglChooseConfig samples 4    = 1  want 1  ok  [check 1]
1791013392.432 gl_test:   config EGL_SAMPLES           = 4  want 4  ok  [check 2]
1791013392.452 gl_test:   GL_SAMPLES x4                = 4  want 4  ok  [check 3]
1791013392.469 I: render: hc dwords: 00000000 003f003f eaa26e26 00000000 2a62aee6 00000000 00000000 00000200 00003003 00000004 00000009 00080000 00080000 00000000 00000000 00000000
1791013392.499 I: render: hrt[0]: 00014000 000000fa 00000000 00000000 00000000 00000000 00000000 00000000
1791013392.512 I: render: hrt[8]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013392.512 I: render: hrt[16]: 00000000 c00ea000 00000000 00000000 c00d8da8 c00d8d28 c00d8d68 00000000
1791013392.513 I: render: hrt[24]: 00000000 00000000 01f84000 00000080 00000000 00000000 00000000 00000000
1791013392.513 I: render: hrt[32]: 00000000 00000000 00000000 00000000 01f80000 00000080 01f96000 00000080
1791013392.526 I: render: hrt[40]: 01f86000 00000080 00000000 00000000 00000000 00000000 00000000 00000000
1791013392.535 I: render: hrt[48]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013392.540 I: render: hrt[56]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013392.546 I: render: HWRT 64x64 x4 te_screen=0x00003003 mtile1=0x00080000 mtile2=0x00080000 isp_mtile=0x00020002 stride=4 rgn=64 tpc=512 teaa=9
1791013392.556 gl_test: px                                 
1791013392.556 gl_test: px                                R
1791013392.567 gl_test: px                               RR
1791013392.567 gl_test: px                              RRR
1791013392.576 gl_test: px                             RRRR
1791013392.576 gl_test: px                            RRRRR
1791013392.577 gl_test: px                           RRRRRR
1791013392.582 gl_test: px                          RRRRRRR
1791013392.588 gl_test: px                         RRRRRRRR
1791013392.592 gl_test: px                        RRRRRRRRR
1791013392.592 gl_test: px                       RRRRRRRRRR
1791013392.597 gl_test: px                      RRRRRRRRRRR
1791013392.602 gl_test: px                     RRRRRRRRRRRR
1791013392.607 gl_test: px                    RRRRRRRRRRRRR
1791013392.612 gl_test: px                   RRRRRRRRRRRRRR
1791013392.612 gl_test: px                  RRRRRRRRRRRRRRR
1791013392.617 gl_test: px                 RRRRRRRRRRRRRRRR
1791013392.628 gl_test: px                RRRRRRRRRRRRRRRRR
1791013392.629 gl_test: px               RRRRRRRRRRRRRRRRRR
1791013392.629 gl_test: px              RRRRRRRRRRRRRRRRRRR
1791013392.630 gl_test: px             RRRRRRRRRRRRRRRRRRRR
1791013392.635 gl_test: px            RRRRRRRRRRRRRRRRRRRRR
1791013392.649 gl_test: px           RRRRRRRRRRRRRRRRRRRRRR
1791013392.649 gl_test: px          RRRRRRRRRRRRRRRRRRRRRRR
1791013392.650 gl_test: px         RRRRRRRRRRRRRRRRRRRRRRRR
1791013392.650 gl_test: px        RRRRRRRRRRRRRRRRRRRRRRRRR
1791013392.654 gl_test: px       RRRRRRRRRRRRRRRRRRRRRRRRRR
1791013392.664 gl_test: px      RRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013392.664 gl_test: px     RRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013392.669 gl_test: px    RRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013392.674 gl_test: px   RRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013392.674 gl_test: px  RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013392.685 gl_test:   x4: inside 0xffff00ff outside 0xff000000 edge 0xff7f007f
1791013392.696 gl_test:   x4 inside                    (40,43) = 0xffff00ff  want 0xffff00ff  ok  [check 4]
1791013392.700 gl_test:   x4 outside                   (20,23) = 0xff000000  want 0xff000000  ok  [check 5]
1791013392.711 gl_test:   x4 edge: half covered        (32,31) = 0xff7f007f  want 0xff800080  ok  [check 6]
1791013392.716 gl_test:   eglChooseConfig default      = 1  want 1  ok  [check 7]
1791013392.719 gl_test:   default config is single-sampled = 0  want 0  ok  [check 8]
1791013392.732 gl_test:   GL_SAMPLES x1                = 0  want 0  ok  [check 9]
1791013392.743 gl_test: px                                 
1791013392.749 gl_test: px                                R
1791013392.760 gl_test: px                               RR
1791013392.760 gl_test: px                              RRR
1791013392.760 gl_test: px                             RRRR
1791013392.762 gl_test: px                            RRRRR
1791013392.767 gl_test: px                           RRRRRR
1791013392.772 gl_test: px                          RRRRRRR
1791013392.779 gl_test: px                         RRRRRRRR
1791013392.784 gl_test: px                        RRRRRRRRR
1791013392.784 gl_test: px                       RRRRRRRRRR
1791013392.793 gl_test: px                      RRRRRRRRRRR
1791013392.793 gl_test: px                     RRRRRRRRRRRR
1791013392.796 gl_test: px                    RRRRRRRRRRRRR
1791013392.796 gl_test: px                   RRRRRRRRRRRRRR
1791013392.807 gl_test: px                  RRRRRRRRRRRRRRR
1791013392.807 gl_test: px                 RRRRRRRRRRRRRRRR
1791013392.813 gl_test: px                RRRRRRRRRRRRRRRRR
1791013392.817 gl_test: px               RRRRRRRRRRRRRRRRRR
1791013392.822 gl_test: px              RRRRRRRRRRRRRRRRRRR
1791013392.822 gl_test: px             RRRRRRRRRRRRRRRRRRRR
1791013392.834 gl_test: px            RRRRRRRRRRRRRRRRRRRRR
1791013392.834 gl_test: px           RRRRRRRRRRRRRRRRRRRRRR
1791013392.835 gl_test: px          RRRRRRRRRRRRRRRRRRRRRRR
1791013392.840 gl_test: px         RRRRRRRRRRRRRRRRRRRRRRRR
1791013392.845 gl_test: px        RRRRRRRRRRRRRRRRRRRRRRRRR
1791013392.851 gl_test: px       RRRRRRRRRRRRRRRRRRRRRRRRRR
1791013392.851 gl_test: px      RRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013392.857 gl_test: px     RRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013392.861 gl_test: px    RRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013392.873 gl_test: px   RRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013392.873 gl_test: px  RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013392.875 gl_test:   x1: inside 0xffff00ff outside 0xff000000 edge 0xffff00ff
1791013392.880 gl_test:   x1 inside                    (40,43) = 0xffff00ff  want 0xffff00ff  ok  [check 10]
1791013392.891 gl_test:   x1 outside                   (20,23) = 0xff000000  want 0xff000000  ok  [check 11]
1791013392.903 gl_test:   x1 edge: full or empty       = 1  want 1  ok  [check 12]
1791013392.903 gl_test: case 16 msaa: PASS
1791013392.904 gl_test:   HelloAPI program links       = 1  want 1  ok  [check 1]
1791013392.911 gl_test:   transformationMatrix found   = 1  want 1  ok  [check 2]
1791013392.921 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013392.926 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013392.926 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013392.936 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013392.936 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013392.940 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013392.940 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013392.944 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013392.949 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013392.957 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013392.961 gl_test: px GGGGGGGGGGGGGGGGRGGGGGGGGGGGGGGG
1791013392.961 gl_test: px GGGGGGGGGGGGGGGRRGGGGGGGGGGGGGGG
1791013392.966 gl_test: px GGGGGGGGGGGGGGGRRRGGGGGGGGGGGGGG
1791013392.970 gl_test: px GGGGGGGGGGGGGGRRRRGGGGGGGGGGGGGG
1791013392.977 gl_test: px GGGGGGGGGGGGGGRRRRRGGGGGGGGGGGGG
1791013392.985 gl_test: px GGGGGGGGGGGGGRRRRRRGGGGGGGGGGGGG
1791013392.985 gl_test: px GGGGGGGGGGGGGRRRRRRRGGGGGGGGGGGG
1791013392.986 gl_test: px GGGGGGGGGGGGRRRRRRRRGGGGGGGGGGGG
1791013392.989 gl_test: px GGGGGGGGGGGGRRRRRRRRRGGGGGGGGGGG
1791013392.993 gl_test: px GGGGGGGGGGGRRRRRRRRRRGGGGGGGGGGG
1791013392.999 gl_test: px GGGGGGGGGGGRRRRRRRRRRRGGGGGGGGGG
1791013393.010 gl_test: px GGGGGGGGGGRRRRRRRRRRRRGGGGGGGGGG
1791013393.010 gl_test: px GGGGGGGGGGRRRRRRRRRRRRRGGGGGGGGG
1791013393.010 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013393.013 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013393.019 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013393.025 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013393.029 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013393.029 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013393.038 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013393.038 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013393.044 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013393.049 gl_test:   HelloAPI: triangle (1, 1, 0.66) (32,36) = 0xffa8ffff  want 0xffa8ffff  ok  [check 3]
1791013393.062 gl_test:   HelloAPI: clear (0, 0.7, 0.67) ( 4, 4) = 0xffabb300  want 0xffabb300  ok  [check 4]
1791013393.067 gl_test:   attribute locations by name  = 1  want 1  ok  [check 5]
1791013393.072 gl_test:   uniform locations by name    = 1  want 1  ok  [check 6]
1791013393.077 gl_test: 8x8 texture @0x8001f4f000: ff0000ff ff0000ff ff0000ff ff0000ff | row1 ff0000ff ff0000ff
1791013393.170 gl_test:   es100 textured: top-left blue (16,16) = 0xffff0000  want 0xffff0000  ok  [check 7]
1791013393.176 gl_test:   es100 textured: top-right white (48,16) = 0xffffffff  want 0xffffffff  ok  [check 8]
1791013393.176 gl_test:   es100 textured: bottom-left red (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 9]
1791013393.176 gl_test:   es100 tint+scale: top-right half green (48,16) = 0xff008000  want 0xff008000  ok  [check 10]
1791013393.176 gl_test:   es100 tint+scale: bottom-left black (16,48) = 0xff000000  want 0xff000000  ok  [check 11]
1791013393.176 gl_test: case 19 es100: PASS
1791013393.176 gl_test:   3 compressed formats         = 3  want 3  ok  [check 1]
1791013393.176 gl_test:   ETC1 listed                  = 36196  want 36196  ok  [check 2]
1791013393.176 gl_test:   GL_OES_compressed_ETC1_RGB8_texture advertised = 1  want 1  ok  [check 3]
1791013393.176 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013393.176 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013393.176 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013393.176 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013393.176 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013393.182 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013393.187 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013393.187 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013393.191 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013393.196 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013393.196 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013393.210 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013393.210 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013393.211 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013393.211 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013393.216 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013393.227 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013393.228 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013393.229 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013393.235 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013393.243 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013393.243 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013393.244 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013393.249 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013393.254 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013393.259 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013393.264 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013393.269 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013393.274 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013393.274 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013393.279 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013393.285 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013393.294 gl_test:   etc1 block (0,0)             ( 8,48) = 0xff0202ff  want 0xff0202ff  ok  [check 4]
1791013393.299 gl_test:   etc1 block (1,0)             (24,48) = 0xff02ff02  want 0xff02ff02  ok  [check 5]
1791013393.311 gl_test:   etc1 block (2,0)             (40,48) = 0xffff0202  want 0xffff0202  ok  [check 6]
1791013393.323 gl_test:   etc1 block (3,0)             (56,48) = 0xffffffff  want 0xffffffff  ok  [check 7]
1791013393.324 gl_test:   etc1 block (0,1)             ( 8,16) = 0xff02ffff  want 0xff02ffff  ok  [check 8]
1791013393.336 gl_test:   etc1 block (1,1)             (24,16) = 0xffffff02  want 0xffffff02  ok  [check 9]
1791013393.346 gl_test:   etc1 block (2,1)             (40,16) = 0xffff02ff  want 0xffff02ff  ok  [check 10]
1791013393.348 gl_test:   etc1 block (3,1)             (56,16) = 0xff8a8a8a  want 0xff8a8a8a  ok  [check 11]
1791013393.358 gl_test:   etc2 rgba block 0            (16,48) = 0xff0202ff  want 0xff0202ff  ok  [check 12]
1791013393.368 gl_test:   etc2 rgba block 1            (48,48) = 0x8002ff02  want 0x8002ff02  ok  [check 13]
1791013393.378 gl_test:   etc2 rgba block 2            (16,16) = 0x40ff0202  want 0x40ff0202  ok  [check 14]
1791013393.384 gl_test:   etc2 rgba block 3            (48,16) = 0x00ffffff  want 0x00ffffff  ok  [check 15]
1791013393.389 I: GL error 0x501 in glCompressedTexImage2D
1791013393.398 gl_test:   wrong imageSize -> INVALID_VALUE = 1281  want 1281  ok  [check 16]
1791013393.398 I: GL error 0x500 in glCompressedTexImage2D
1791013393.403 gl_test:   PVRTC -> INVALID_ENUM        = 1280  want 1280  ok  [check 17]
1791013393.409 gl_test: case 20 etc: PASS
1791013393.413 I:   cube-bind: 0x8001f5d000 unit 0 kick 104 (draw)
1791013393.423 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013393.437 gl_test:   face 0 centre                (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 1]
1791013393.452 gl_test:   face 0 off-centre            (16,16) = 0xff0000ff  want 0xff0000ff  ok  [check 2]
1791013393.452 I:   cube-bind: 0x8001f5d000 unit 0 kick 105 (draw)
1791013393.452 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013393.464 gl_test:   face 1 centre                (32,32) = 0xff00ff00  want 0xff00ff00  ok  [check 3]
1791013393.475 gl_test:   face 1 off-centre            (16,16) = 0xff00ff00  want 0xff00ff00  ok  [check 4]
1791013393.481 I:   cube-bind: 0x8001f5d000 unit 0 kick 106 (draw)
1791013393.485 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013393.497 gl_test:   face 2 centre                (32,32) = 0xffff0000  want 0xffff0000  ok  [check 5]
1791013393.501 gl_test:   face 2 off-centre            (16,16) = 0xffff0000  want 0xffff0000  ok  [check 6]
1791013393.511 I:   cube-bind: 0x8001f5d000 unit 0 kick 107 (draw)
1791013393.516 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013393.526 gl_test:   face 3 centre                (32,32) = 0xff00ffff  want 0xff00ffff  ok  [check 7]
1791013393.535 gl_test:   face 3 off-centre            (16,16) = 0xff00ffff  want 0xff00ffff  ok  [check 8]
1791013393.540 I:   cube-bind: 0x8001f5d000 unit 0 kick 108 (draw)
1791013393.556 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013393.556 gl_test:   face 4 centre                (32,32) = 0xffff00ff  want 0xffff00ff  ok  [check 9]
1791013393.565 gl_test:   face 4 off-centre            (16,16) = 0xffff00ff  want 0xffff00ff  ok  [check 10]
1791013393.571 I:   cube-bind: 0x8001f5d000 unit 0 kick 109 (draw)
1791013393.581 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013393.591 gl_test:   face 5 centre                (32,32) = 0xffffff00  want 0xffffff00  ok  [check 11]
1791013393.596 gl_test:   face 5 off-centre            (16,16) = 0xffffff00  want 0xffffff00  ok  [check 12]
1791013393.617 I: render: hc dwords: 00000000 00070007 00000088 00000000 00000088 00000000 00000000 00000080 00001001 00000004 00000000 00080000 00080000 00000000 00000000 00000000
1791013393.622 I: render: hrt[0]: 00018000 000000fa 00000000 00000000 00000000 00000000 00000000 00000000
1791013393.627 I: render: hrt[8]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013393.637 I: render: hrt[16]: 00000000 c00ea180 00000000 00000000 c00d8da8 c00d8d28 c00d8d68 00000000
1791013393.642 I: render: hrt[24]: 00000000 00000000 01f9b000 00000080 00000000 00000000 00000000 00000000
1791013393.653 I: render: hrt[32]: 00000000 00000000 00000000 00000000 01f97000 00000080 01fad000 00000080
1791013393.670 I: render: hrt[40]: 01f9d000 00000080 00000000 00000000 00000000 00000000 00000000 00000000
1791013393.670 I: render: hrt[48]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013393.678 I: render: hrt[56]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013393.686 I: render: HWRT 8x8 x1 te_screen=0x00001001 mtile1=0x00080000 mtile2=0x00080000 isp_mtile=0x00020002 stride=4 rgn=16 tpc=128 teaa=0
1791013393.697 I:   cube-bind: 0x8001f5d000 unit 0 kick 111 (draw)
1791013393.702 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013393.712 gl_test:   after FBO: face 0            (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 13]
1791013393.717 I:   cube-bind: 0x8001f5d000 unit 0 kick 112 (draw)
1791013393.722 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013393.733 gl_test:   after FBO: face 2            (32,32) = 0xffffffff  want 0xffffffff  ok  [check 14]
1791013393.738 I:   cube-bind: 0x8001f5d000 unit 0 kick 113 (draw)
1791013393.743 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013393.759 gl_test:   after FBO: face 4            (32,32) = 0xffff00ff  want 0xffff00ff  ok  [check 15]
1791013393.777 gl_test: case 41 es3cube: PASS
1791013393.777 I:   cube-bind: 0x8001f5f000 unit 0 kick 114 (draw)
1791013393.777 I:   cube(draw) @0x8001f5f000 8x8 x3: fmt 0, cpu 0x90a0e000 - not dumped
1791013393.778 gl_test:   layer 0 (rgb texture, a texelFetch) (32,32) = 0x6400ff00  want 0x6400ff00  ok  [check 1]
1791013393.791 I:   cube-bind: 0x8001f5f000 unit 0 kick 115 (draw)
1791013393.795 I:   cube(draw) @0x8001f5f000 8x8 x3: fmt 0, cpu 0x90a0e000 - not dumped
1791013393.806 gl_test:   layer 1 (rgb texture, a texelFetch) (32,32) = 0x9628af50  want 0x9628af50  ok  [check 2]
1791013393.811 I:   cube-bind: 0x8001f5f000 unit 0 kick 116 (draw)
1791013393.822 I:   cube(draw) @0x8001f5f000 8x8 x3: fmt 0, cpu 0x90a0e000 - not dumped
1791013393.832 gl_test:   layer 2 (rgb texture, a texelFetch) (32,32) = 0xc8505fa0  want 0xc8505fa0  ok  [check 3]
1791013393.849 I: render: hc dwords: 00000000 00070007 00000088 00000000 00000088 00000000 00000000 00000080 00001001 00000004 00000000 00080000 00080000 00000000 00000000 00000000
1791013393.857 I: render: hrt[0]: 0001c000 000000fa 00000000 00000000 00000000 00000000 00000000 00000000
1791013393.865 I: render: hrt[8]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013393.870 I: render: hrt[16]: 00000000 c00ea300 00000000 00000000 c00d8da8 c00d8d28 c00d8d68 00000000
1791013393.879 I: render: hrt[24]: 00000000 00000000 01fb2000 00000080 00000000 00000000 00000000 00000000
1791013393.886 I: render: hrt[32]: 00000000 00000000 00000000 00000000 01fae000 00000080 01fc4000 00000080
1791013393.892 I: render: hrt[40]: 01fb4000 00000080 00000000 00000000 00000000 00000000 00000000 00000000
1791013393.914 I: render: hrt[48]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013393.922 I: render: hrt[56]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013393.928 I: render: HWRT 8x8 x1 te_screen=0x00001001 mtile1=0x00080000 mtile2=0x00080000 isp_mtile=0x00020002 stride=4 rgn=16 tpc=128 teaa=0
1791013393.928 I:   cube-bind: 0x8001f5f000 unit 0 kick 118 (draw)
1791013393.933 I:   cube(draw) @0x8001f5f000 8x8 x3: fmt 0, cpu 0x90a0e000 - not dumped
1791013393.949 gl_test:   after FBO: layer 0           (32,32) = 0x6400ff00  want 0x6400ff00  ok  [check 4]
1791013393.949 I:   cube-bind: 0x8001f5f000 unit 0 kick 119 (draw)
1791013393.960 I:   cube(draw) @0x8001f5f000 8x8 x3: fmt 0, cpu 0x90a0e000 - not dumped
1791013393.964 gl_test:   after FBO: layer 1           (32,32) = 0xffff0000  want 0xffff0000  ok  [check 5]
1791013393.969 I:   cube-bind: 0x8001f5f000 unit 0 kick 120 (draw)
1791013393.979 I:   cube(draw) @0x8001f5f000 8x8 x3: fmt 0, cpu 0x90a0e000 - not dumped
1791013393.988 gl_test:   after FBO: layer 2           (32,32) = 0xc8505fa0  want 0xc8505fa0  ok  [check 6]
1791013393.999 gl_test: case 42 es3array: PASS
1791013394.000 gl_test: px GGGGGGGGGGGGGGGGRRRRRRRRRRRRRRRR
1791013394.006 gl_test: px GGGGGGGGGGGGGGGGRRRRRRRRRRRRRRRB
1791013394.011 gl_test: px GGGGGGGGGGGGGGGRRRRRRRRRRRRRRRBB
1791013394.016 gl_test: px GGGGGGGGGGGGGGGRRRRRRRRRRRRRRBBB
1791013394.016 gl_test: px GGGGGGGGGGGGGGRRRRRRRRRRRRRRBBBB
1791013394.021 gl_test: px GGGGGGGGGGGGGGRRRRRRRRRRRRRBBBBB
1791013394.027 gl_test: px GGGGGGGGGGGGGRRRRRRRRRRRRRBBBBBB
1791013394.031 gl_test: px GGGGGGGGGGGGGRRRRRRRRRRRRBBBBBBB
1791013394.031 gl_test: px GGGGGGGGGGGGRRRRRRRRRRRRBBBBBBBB
1791013394.036 gl_test: px GGGGGGGGGGGGRRRRRRRRRRRBBBBBBBBB
1791013394.041 gl_test: px GGGGGGGGGGGRRRRRRRRRRRBBBBBBBBBB
1791013394.046 gl_test: px GGGGGGGGGGGRRRRRRRRRRBBBBBBBBBBB
1791013394.051 gl_test: px GGGGGGGGGGRRRRRRRRRRBBBBBBBBBBBB
1791013394.051 gl_test: px GGGGGGGGGGRRRRRRRRRBBBBBBBBBBBBB
1791013394.062 gl_test: px GGGGGGGGGRRRRRRRRRBBBBBBBBBBBBBB
1791013394.112 gl_test: px GGGGGGGGGRRRRRRRRBBBBBBBBBBBBBBB
1791013394.112 gl_test: px GGGGGGGGRRRRRRRRRBBBBBBBBBBBBBBB
1791013394.157 gl_test: px GGGGGGGGRRRRRRRRRRBBBBBBBBBBBBGG
1791013394.157 gl_test: px GGGGGGGRRRRRRRRRRRRBBBBBBBBBGGGG
1791013394.157 gl_test: px GGGGGGGRRRRRRRRRRRRRBBBBBBGGGGGG
1791013394.157 gl_test: px GGGGGGRRRRRRRRRRRRRRRBBBGGGGGGGG
1791013394.157 gl_test: px GGGGGGRRRRRRRRRRRRRRRRGGGGGGGGGG
1791013394.157 gl_test: px GGGGGRRRRRRRRRRRRRRRRGGGGGGGGGGG
1791013394.157 gl_test: px GGGGGRRRRRRRRRRRRRRRRGGGGGGGGGGG
1791013394.157 gl_test: px GGGGRRRRRRRRRRRRRRRRGGGGGGGGGGGG
1791013394.157 gl_test: px WWWWWWWRRRRRRRRRRRRRGGGGGGGGGGGG
1791013394.157 gl_test: px WWWWWWRRRRRRRRRRRRRGGGGGGGGGGGGG
1791013394.157 gl_test: px WWWWWRRRRRRRRRRRRRRGGGGGGGGGGGGG
1791013394.157 gl_test: px WWWWRRRRRRRRRRRRRRGGGGGGGGGGGGGG
1791013394.157 gl_test: px WWWRRRRRRRRRRRRRRRGGGGGGGGGGGGGG
1791013394.165 gl_test: px WWRRRRRRRRRRRRRRRGGGGGGGGGGGGGGG
1791013394.166 gl_test: px WRRRRRRRRRRRRRRRRGGGGGGGGGGGGGGG
1791013394.171 gl_test:   bottom-right ~ vertex 1 green (60,60) = 0xff0ee30e  want 0xff0ee30e  ok  [check 1]
1791013394.171 gl_test:   bottom-left ~ vertex 0 red   ( 4,60) = 0xff0e04ed  want 0xff1200ed  ok  [check 2]
1791013394.171 gl_test:   top-left ~ vertex 5 green    ( 4, 4) = 0xff12ed24  want 0xff12ed24  ok  [check 3]
1791013394.171 gl_test: case 43 es3vid: PASS
1791013394.171 I:   cube-bind: 0x8001e80000 unit 1 kick 122 (draw)
1791013394.171 I:   cube(draw) @0x8001e80000 8x8 x3: fmt 0, cpu 0x908ed000 - not dumped
1791013394.177 gl_test:   textureSize 2D / array       (32,32) = 0x03081808  want 0x03081808  ok  [check 1]
1791013394.181 gl_test: case 46 es3size: PASS
1791013394.191 gl_test:   arrays: instance 0 red       (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 1]
1791013394.197 gl_test:   arrays: instance 1 green     (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 2]
1791013394.207 gl_test:   arrays: instance 2 blue      (16,16) = 0xffff0000  want 0xffff0000  ok  [check 3]
1791013394.212 gl_test:   arrays: instance 3 half white (48,16) = 0x80808080  want 0x80808080  ok  [check 4]
1791013394.225 gl_test:   arrays: between = clear      (32,32) = 0xff000000  want 0xff000000  ok  [check 5]
1791013394.233 gl_test:   elements: instance 0 red     (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 6]
1791013394.245 gl_test:   elements: instance 1 green   (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 7]
1791013394.250 gl_test:   elements: instance 2 blue    (16,16) = 0xffff0000  want 0xffff0000  ok  [check 8]
1791013394.259 gl_test:   elements: instance 3 half white (48,16) = 0x80808080  want 0x80808080  ok  [check 9]
1791013394.267 gl_test:   elements: between = clear    (32,32) = 0xff000000  want 0xff000000  ok  [check 10]
1791013394.268 gl_test: case 47 es3inst: PASS
1791013394.274 I:   cube-bind: 0x8001ebd000 unit 0 kick 125 (draw)
1791013394.279 I:   cube(draw) @0x8001ebd000 4x4 x5: fmt 0, cpu 0x90936000 - not dumped
1791013394.294 gl_test:   slice 2: texture/lod/fetch/depth (32,32) = 0x05466464  want 0x05506464  MISMATCH  [check 1]
1791013394.294 I:   cube-bind: 0x8001ebd000 unit 0 kick 126 (draw)
1791013394.304 I:   cube(draw) @0x8001ebd000 4x4 x5: fmt 0, cpu 0x90936000 - not dumped
1791013394.314 gl_test:   between 0 and 1: blended     (32,32) = 0x05501c1c  want 0x05281c1c  MISMATCH  [check 2]
1791013394.317 gl_test: case 48 es3vol: FAIL
1791013394.337 gl_test:   window, front CCW: front (green) (32,32) = 0xff00ff00  want 0xff00ff00  ok  [check 1]
1791013394.343 gl_test:   window, front CW: back (red) (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 2]
1791013394.359 gl_test:   fbo, front CCW: front (green x0.25) (32,32) = 0x40004000  want 0x40004000  ok  [check 3]
1791013394.370 gl_test:   fbo, front CW: back (red x0.25) (32,32) = 0x40000040  want 0x40000040  ok  [check 4]
1791013394.372 gl_test: case 49 es3facing: PASS
1791013394.388 gl_test:   idx=2 n=3 -> cols[2..4]      (32,32) = 0xbf9900e5  want 0xbf9900e6  ok  [check 1]
1791013394.397 gl_test:   idx=5 n=2 -> cols[5..6]      (32,32) = 0x806600ff  want 0x806600ff  ok  [check 2]
1791013394.402 gl_test: case 45 es3dyn: PASS
1791013394.413 gl_test:   rgba8 clear-only x0.25       (32,32) = 0x40401020  want 0x40401020  ok  [check 1]
1791013394.423 gl_test:   cleared 2.0/0.5/0.25 x0.25   (32,32) = 0x40102080  want 0x40102080  ok  [check 2]
1791013394.437 gl_test:   drawn: bottom-right ~ green x0.25 (60,60) = 0x40043804  want 0x40043904  ok  [check 3]
1791013394.450 gl_test:   drawn: bottom-left ~ red x0.25 ( 4,60) = 0x4004003c  want 0x4005003b  ok  [check 4]
1791013394.458 gl_test:   clear+draw: right half = clear (56,32) = 0x40102080  want 0x40102080  ok  [check 5]
1791013394.472 gl_test:   clear+draw: left half drawn  ( 8,56) = 0x40040834  want 0x4005003b  ok  [check 6]
1791013394.472 gl_test: case 44 es3hdr: PASS
1791013394.473 I: osal(zephyr): ready (regs=0xfd80000)
1791013394.478 gl_test:   a 16-bit config is offered   = 1  want 1  ok  [check 1]
1791013394.483 gl_test:   32-bit configs are offered too = 1  want 1  ok  [check 2]
1791013394.495 gl_test:   green size 6                 = 6  want 6  ok  [check 3]
1791013394.500 gl_test:   32-bit config refuses the window = 1  want 1  ok  [check 4]
1791013394.503 gl_test:   RGB565 window surface        = 1  want 1  ok  [check 5]
1791013394.515 gl_test:   eglSwapBuffers               = 1  want 1  ok  [check 6]
1791013394.520 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.530 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.533 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.538 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.543 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.548 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.555 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.555 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.562 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.562 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.563 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.574 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.578 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.584 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.584 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.589 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.593 gl_test: px RRRRRRRRRRGRRRRRRRRRRRRRRRRRRRRR
1791013394.593 gl_test: px RRRRRRRRRRGGRRRRRRRRRBBRRRRRRRRR
1791013394.598 gl_test: px RRRRRRRRRGGGGRRRRRRRBBBRRRRRRRRR
1791013394.598 gl_test: px RRRRRRRRRGGGGGGRRRBBBBBBRRRRRRRR
1791013394.607 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBRRRRRRRR
1791013394.607 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBBRRRRRRR
1791013394.613 gl_test: px RRRRRRRGGGGGGGGGBBBBBBBBBRRRRRRR
1791013394.618 gl_test: px RRRRRRRGGGGGGGGGBBBBBBBBBBRRRRRR
1791013394.623 gl_test: px RRRRRRGGGGGGGGGGBBBBBBBBBBRRRRRR
1791013394.623 gl_test: px RRRRRRGGGGGGGGGGBBBBBBBBBBBRRRRR
1791013394.627 gl_test: px RRRRRGGGGGGGGGGGBBBBBBBBBBBRRRRR
1791013394.638 gl_test: px RRRRRGGGGGGGGGGGBBBBBBBBBBBBRRRR
1791013394.639 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.641 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.646 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.646 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013394.662 gl_test:   565: centroid (5/6-bit steps) (32,40) = 0xff5a5552  want 0xff595452  ok  [check 7]
1791013394.666 gl_test:   565: red background          ( 1, 1) = 0xff0000ff  want 0xff0000ff  ok  [check 8]
1791013394.684 gl_test:   565: raw red pixel = 0xF800  = 63488  want 63488  ok  [check 9]
1791013394.684 gl_test:   eglSwapBuffers 2             = 1  want 1  ok  [check 10]
1791013394.684 gl_test:   565: earlier frame kept      (32,40) = 0xff5a5552  want 0xff595452  ok  [check 11]
1791013394.694 gl_test:   565: background kept         ( 1, 1) = 0xff0000ff  want 0xff0000ff  ok  [check 12]
1791013394.704 gl_test:   565: corner triangle blue    ( 6,60) = 0xffff0000  want 0xffff0000  ok  [check 13]
1791013394.709 gl_test:   release                      = 1  want 1  ok  [check 14]
1791013394.714 gl_test:   eglDestroySurface            = 1  want 1  ok  [check 15]
1791013394.724 gl_test:   eglDestroyContext            = 1  want 1  ok  [check 16]
1791013394.728 gl_test:   eglTerminate                 = 1  want 1  ok  [check 17]
1791013394.728 gl_test: case 21 rgb565: PASS
1791013394.747 I: osal(zephyr): ready (regs=0xfd80000)
1791013394.749 gl_test:   two contexts created         = 1  want 1  ok  [check 1]
1791013394.757 gl_test:   the context pool refuses past its limit = 1  want 1  ok  [check 2]
1791013394.767 gl_test:   and refuses with EGL_BAD_ALLOC = 1  want 1  ok  [check 3]
1791013394.788 gl_test:   makeCurrent A                = 1  want 1  ok  [check 4]
1791013394.789 gl_test: 4x4 texture @0x8001eea000: ff0000ff ff0000ff ff00ff00 ff00ff00 | row1 ff0000ff ff0000ff
1791013394.800 gl_test:   A: centroid                  (32,40) = 0xff5a5552  want 0xff595452  ok  [check 5]
1791013394.809 gl_test:   A: red background            ( 1, 1) = 0xff0000ff  want 0xff0000ff  ok  [check 6]
1791013394.821 gl_test:   makeCurrent B                = 1  want 1  ok  [check 7]
1791013394.825 gl_test:   B: A's program is unknown    = 0  want 0  ok  [check 8]
1791013394.840 gl_test:   B: A's texture is unknown    = 0  want 0  ok  [check 9]
1791013394.840 gl_test:   B: default clear colour      = 1  want 1  ok  [check 10]
1791013394.857 gl_test:   B: blue quad                 (32,32) = 0xffff0000  want 0xffff0000  ok  [check 11]
1791013394.859 gl_test:   makeCurrent A again          = 1  want 1  ok  [check 12]
1791013394.865 gl_test:   A: program still there       = 1  want 1  ok  [check 13]
1791013394.874 gl_test:   A: texture still there       = 1  want 1  ok  [check 14]
1791013394.881 gl_test:   A: clear colour kept         = 1  want 1  ok  [check 15]
1791013394.890 gl_test:   A: surface kept              (32,40) = 0xff5a5552  want 0xff595452  ok  [check 16]
1791013394.901 gl_test:   A: redraw with its program   (32,40) = 0xff5a5552  want 0xff595452  ok  [check 17]
1791013394.901 gl_test: case 17 ctx2: PASS
1791013394.912 I: osal(zephyr): ready (regs=0xfd80000)
1791013394.921 gl_test:   EGL_KHR_image_base advertised = 1  want 1  ok  [check 1]
1791013394.941 gl_test: 8x8 texture @0x8001ef5000: ff0000ff ff0000ff ff0000ff ff0000ff | row1 ff0000ff ff0000ff
1791013394.953 gl_test:   image from GL texture        = 1  want 1  ok  [check 2]
1791013394.953 gl_test:   image from a bad texture fails = 1  want 1  ok  [check 3]
1791013394.956 gl_test:   image from native buffer     = 1  want 1  ok  [check 4]
1791013394.976 gl_test:   GL_OES_EGL_image advertised  = 1  want 1  ok  [check 5]
1791013394.985 gl_test:   texture image: top-left blue (16,16) = 0xffff0000  want 0xffff0000  ok  [check 6]
1791013394.999 gl_test:   texture image: bottom-right green (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 7]
1791013395.013 gl_test:   native image: bottom row red (32,60) = 0xff0000ff  want 0xff0000ff  ok  [check 8]
1791013395.017 gl_test:   native image: top row white  (32, 4) = 0xffffffff  want 0xffffffff  ok  [check 9]
1791013395.022 gl_test:   native image: second row green (32,44) = 0xff00ff00  want 0xff00ff00  ok  [check 10]
1791013395.039 gl_test:   image kept after source delete (16,16) = 0xffff0000  want 0xffff0000  ok  [check 11]
1791013395.043 gl_test:   eglDestroyImage              = 1  want 1  ok  [check 12]
1791013395.043 gl_test: case 18 image: PASS
1791013395.059 gl_test: all: 1 of 38 cases failed (37 passed; 0 timed out; first failure case 48 at index 31)
1791013395.064 gl_test: done
1791013395.074 osal(zephyr): pending frees: high-water 98/128, waits 0 (opt-in 0), refused 0 (0 unsubmitted, 0 no wait, 0 KiB kept)
1791013395.086 osal(zephyr): ds slots peak 3/8, ds bytes peak 81920, ds refused 0 (slots full 0, no buffer 0)
1791013395.086 gl_test: FAILED (rc=-1)
1791013395.097 I: dss stats before hold t=10519 ms: vsyncs 0 app_frames 0 underflows 0 sync_lost 0 go_timeouts 0 go_waits 0 safety_flips 0 app 0
1791013415.103 I: dss stats after hold t=30531 ms: vsyncs 0 app_frames 0 underflows 0 sync_lost 0 go_timeouts 0 go_waits 0 safety_flips 0 app 0
1791013444.752 �
1791013444.766 U-Boot SPL 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
1791013444.774 SYSFW ABI: 4.0 (firmware rev 0x000b '11.2.5--v11.02.05 (Fancy Rat)')
1791013444.817 Changed A53 CPU frequency to 1250000000Hz (U grade) in DT
1791013444.817 SPL initial stack usage: 17088 bytes
1791013444.831 Trying to boot from MMC1
1791013444.943 Authentication passed
1791013444.956 Authentication passed
1791013444.959 Authentication passed
1791013445.026 Loading Environment from nowhere... OK
1791013445.026 init_env from device 9 not supported!
1791013445.031 Authentication passed
1791013445.046 Authentication passed
1791013445.059 Starting ATF on ARM64 core...
1791013445.059 
1791013445.063 NOTICE:  BL31: v2.13.0(release):v2.13.0-259-ge0c4d3903b-dirty
1791013445.072 NOTICE:  BL31: Built : 00:00:00, Jan  1 1970
1791013445.292 
1791013445.306 U-Boot SPL 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
1791013445.311 SYSFW ABI: 4.0 (firmware rev 0x000b '11.2.5--v11.02.05 (Fancy Rat)')
1791013445.315 SPL initial stack usage: 1920 bytes
1791013445.315 HW CFG: 0x00
1791013445.321 Trying to boot from MMC1
1791013445.408 Authentication passed
1791013445.418 Authentication passed
1791013446.934 
1791013446.934 
1791013446.947 U-Boot 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
1791013446.947 
1791013446.947 SoC:   AM62PX SR1.0 HS-FS
1791013446.951 DRAM:  2 GiB
1791013447.067 optee optee: OP-TEE: revision 4.7 (a9690ae39995af36)
1791013447.072 Core:  161 devices, 34 uclasses, devicetree: separate
1791013447.166 MMC:   mmc@fa10000: 0, mmc@fa00000: 1
1791013447.179 Loading Environment from MMC... OK
1791013447.209 In:    serial@2800000
1791013447.209 Out:   serial@2800000
1791013447.212 Err:   serial@2800000
1791013447.217 Model: Toradex 0099 Verdin AM62P Quad 2GB WB IT V1.0B
1791013447.221 Serial# 00000000
1791013447.221 Carrier: Toradex Dahlia V1.1C, Serial# 00000000
1791013447.239 am65_cpsw_nuss ethernet@8000000: K3 CPSW: nuss_ver: 0x6BA01903 cpsw_ver: 0x6BA81903 ale_ver: 0x00290105 Ports:2
1791013447.290 Net:   eth0: ethernet@8000000port@1 [PRIME]Could not get PHY for mdio@f00: addr 7
1791013447.300 am65_cpsw_nuss_port ethernet@8000000port@2: phy_connect() failed
1791013447.300 
1791013447.311 Hit any key to stop autoboot:  0 
1791013447.367 6188160 bytes read in 55 ms (107.3 MiB/s)
1791013447.402 288 bytes read in 20 ms (13.7 KiB/s)
1791013447.402 Failed to load '/stock/zephyr_a53.bin'
1791013447.402 stock images not found, booting Linux
1791013447.507 7[r[999;999H[6n8MMC: no card present
1791013447.516 No EFI system partition
1791013447.517 No EFI system partition
1791013447.518 Failed to persist EFI variables
1791013447.524 No EFI system partition
1791013447.524 Failed to persist EFI variables
1791013447.524 No EFI system partition
1791013447.529 Failed to persist EFI variables
1791013447.533 ** Booting bootflow '<NULL>' with efi_mgr
1791013447.555 Loading Boot0000 'mmc 0' failed
1791013447.562 EFI boot manager: Cannot load any image
1791013447.562 Boot failed (err=-14)
1791013447.572 MMC: no card present
1791013447.578 MMC: no card present
1791013447.582 MMC: no card present
1791013447.582 MMC: no card present
1791013447.588 ** Booting bootflow 'mmc@fa10000.bootdev.part_1' with script
1791013447.625 6956 bytes read in 21 ms (323.2 KiB/s)
1791013447.638 75135 bytes read in 22 ms (3.3 MiB/s)
1791013447.671 65 bytes read in 21 ms (2.9 KiB/s)
1791013447.675 Working FDT set to 90200000
1791013447.678 Applying Overlay: verdin-am62p_panel-cap-touch-7inch-dsi_overlay.dtbo
1791013447.699 4693 bytes read in 21 ms (217.8 KiB/s)
1791013447.862 11272338 bytes read in 83 ms (129.5 MiB/s)
1791013447.964 13381763 bytes read in 95 ms (134.3 MiB/s)
1791013447.967    Uncompressing Kernel Image to 0
1791013448.372 ## Flattened Device Tree blob at 90200000
1791013448.376    Booting using the fdt blob at 0x90200000
1791013448.380 Working FDT set to 90200000
1791013448.391    Loading Ramdisk to fc1d3000, end fce96083 ... OK
1791013448.421    Loading Device Tree to 00000000fc19d000, end 00000000fc1d2fff ... OK
1791013448.423 Working FDT set to fc19d000
1791013448.443 
1791013448.448 Starting kernel ...
1791013448.448 
1791013449.757 [    1.100424] tps65219 0-0030: Error IRQ trap residual voltage for LDO4
1791013449.760 [    1.134318] debugfs: File ':bus@f0000:bus@48000000:interrupt-controller@48000000' in directory 'domains' already present!
1791013449.780 [    1.134679] debugfs: File ':bus@f0000:bus@4e000000:interrupt-controller@4e400000' in directory 'domains' already present!
1791013450.565 [    1.965293] OF: graph: no port node found in /bus@f0000/dss@30220000/oldi-txes/oldi@1/ports
1791013450.625 [    2.028423] OF: graph: no port node found in /bus@f0000/dss@30220000/oldi-txes/oldi@1/ports
1791013450.636 [    2.038077] cdns-dsi 30500000.dsi: [drm] *ERROR* modalias failure on /bus@f0000/dsi@30500000/port@1
1791013450.649 [    2.047209] cdns-dsi 30500000.dsi: [drm] *ERROR* modalias failure on /bus@f0000/dsi@30500000/port@0
1791013450.659 [    2.059689] OF: graph: no port node found in /bus@f0000/dss@30220000/oldi-txes/oldi@1/ports
1791013450.691 Starting systemd-udevd version 255.21^
1791013451.648 sysroot.readonly configuration value: 0 (fs writable: 1)
1791013451.657 Using legacy ostree bind mount for /
1791013457.114 [    8.514345] omap-mailbox 29000000.mailbox: no available mbox devices found
1791013457.191 [    8.592742] omap-mailbox 29020000.mailbox: no available mbox devices found
1791013457.204 [    8.609835] vdec 30210000.video-codec: OPP table not found in device tree
1791013457.222 [    8.622966] omap-mailbox 29030000.mailbox: no available mbox devices found
1791013457.529 [    8.930509] remoteproc remoteproc0: request_firmware failed: -2
1791013457.837 [    9.240489] platform 78000000.r5f: mbox_request_channel: can't parse "mboxes" property
1791013457.847 [    9.248450] platform 78000000.r5f: error -ENOENT: mbox_request_channel failed
1791013457.859 [    9.257550] k3_r5_rproc bus@f0000:bus@b00000:r5fss@78000000: k3_r5_cluster_rproc_init failed, ret = -2
