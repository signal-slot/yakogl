1791013643.413 [  194.812679] watchdog: watchdog0: watchdog did not stop!
1791013645.831 [  197.232346] systemd-shutdown[1]: Failed to disable hardware watchdog, ignoring: Device or resource busy
1791013645.834 [  197.241976] watchdog: watchdog0: watchdog did not stop!
1791013645.973 [  197.359001] failed to disconnect on suspend
1791013645.973 [  197.365293] reboot: Restarting system
1791013646.144 �
1791013646.158 U-Boot SPL 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
1791013646.162 SYSFW ABI: 4.0 (firmware rev 0x000b '11.2.5--v11.02.05 (Fancy Rat)')
1791013646.205 Changed A53 CPU frequency to 1250000000Hz (U grade) in DT
1791013646.212 SPL initial stack usage: 17088 bytes
1791013646.227 Trying to boot from MMC1
1791013646.334 Authentication passed
1791013646.344 Authentication passed
1791013646.348 Authentication passed
1791013646.413 Loading Environment from nowhere... OK
1791013646.419 init_env from device 9 not supported!
1791013646.422 Authentication passed
1791013646.438 Authentication passed
1791013646.454 Starting ATF on ARM64 core...
1791013646.454 
1791013646.466 NOTICE:  BL31: v2.13.0(release):v2.13.0-259-ge0c4d3903b-dirty
1791013646.466 NOTICE:  BL31: Built : 00:00:00, Jan  1 1970
1791013646.690 
1791013646.690 U-Boot SPL 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
1791013646.700 SYSFW ABI: 4.0 (firmware rev 0x000b '11.2.5--v11.02.05 (Fancy Rat)')
1791013646.709 SPL initial stack usage: 1920 bytes
1791013646.709 HW CFG: 0x00
1791013646.714 Trying to boot from MMC1
1791013646.802 Authentication passed
1791013646.813 Authentication passed
1791013648.330 
1791013648.330 
1791013648.345 U-Boot 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
1791013648.345 
1791013648.345 SoC:   AM62PX SR1.0 HS-FS
1791013648.346 DRAM:  2 GiB
1791013648.464 optee optee: OP-TEE: revision 4.7 (a9690ae39995af36)
1791013648.475 Core:  161 devices, 34 uclasses, devicetree: separate
1791013648.491 MMC:   mmc@fa10000: 0, mmc@fa00000: 1
1791013648.581 Loading Environment from MMC... OK
1791013648.601 In:    serial@2800000
1791013648.605 Out:   serial@2800000
1791013648.609 Err:   serial@2800000
1791013648.616 Model: Toradex 0099 Verdin AM62P Quad 2GB WB IT V1.0B
1791013648.616 Serial# 00000000
1791013648.620 Carrier: Toradex Dahlia V1.1C, Serial# 00000000
1791013648.642 am65_cpsw_nuss ethernet@8000000: K3 CPSW: nuss_ver: 0x6BA01903 cpsw_ver: 0x6BA81903 ale_ver: 0x00290105 Ports:2
1791013648.690 Net:   eth0: ethernet@8000000port@1 [PRIME]Could not get PHY for mdio@f00: addr 7
1791013648.697 am65_cpsw_nuss_port ethernet@8000000port@2: phy_connect() failed
1791013648.697 
1791013648.706 Hit any key to stop autoboot:  0 
1791013648.706 yakogl-netboot: once=selfarmC-f047d34c0b11cb05-1791013634
1791013648.721 Saving Environment to MMC... Writing to MMC(0)... OK
1791013648.726 yakogl-netboot: cleared
1791013648.738 ti-udma dma-controller@485c0000: k3_dmaring Ring probed rings:150, sci-dev-id:30
1791013648.746 ti-udma dma-controller@485c0000: dma-ring-reset-quirk: disabled
1791013648.753 am65_cpsw_nuss_port ethernet@8000000port@1: K3 CPSW: rflow_id_base: 19
1791013651.312 ethernet@8000000port@1 Waiting for PHY auto negotiation to complete...... done
1791013651.321 link up on port 1, speed 1000, full duplex
1791013651.321 BOOTP broadcast 1
1791013651.571 BOOTP broadcast 2
1791013652.072 BOOTP broadcast 3
1791013652.492 DHCP client bound to address 192.0.2.9 (1170 ms)
1791013652.512 am65_cpsw_nuss_port ethernet@8000000port@1: K3 CPSW: rflow_id_base: 19
1791013652.518 link up on port 1, speed 1000, full duplex
1791013652.518 Using ethernet@8000000port@1 device
1791013652.519 TFTP from server 192.0.2.9; our IP address is 192.0.2.9
1791013652.526 Filename 'yakogl/selfarmC-f047d34c0b11cb05-1791013634.bin'.
1791013652.545 Load address: 0x82000000
1791013652.785 Loading: *##################################################  15.5 MiB
1791013652.791 	 64 MiB/s
1791013652.791 done
1791013652.791 Bytes transferred = 16234368 (f7b780 hex)
1791013652.797 yakogl-netboot: go 0x82000000
1791013652.800 ## Starting application at 0x82000000 ...
1791013652.874 Htsample: build tree=71145c16 table=1449eb0f
1791013652.888 I: rti_arm: armed preload 0xef rate 32768 Hz: expiry 60000 ms, fed for 150 s
1791013652.893 I: Cadence DSI id 0xcadd5131, IP_CONF 0x948490d0
1791013652.893 I: D-PHY: WIZ at +0xf00 (MOD_VER 0x69924881)
1791013652.919 I: VP clock source 51200000 Hz (pixel clock 51200000 Hz)
1791013652.919 I: DSS rev 0x64040901
1791013652.925 I: DSS ready: VP2 1024x600 @ 51200000 Hz, safety layer ARGB8888 at 0x84896040
1791013652.936 I: TISCI: power domain 237 on
1791013652.936 I: TISCI: power domain 242 on
1791013652.938 I: TISCI: GPU clock 237/3 = 800000000 Hz
1791013652.944 I: TISCI: GPU clock 237/3 runs at 800000000 Hz
1791013652.949 I: GPU regs 0xfd80000 (512 KiB) mapped at 0xfd80000
1791013652.955 I: CORE_ID = 0x002400350068031c -> BVNC 36.53.104.796
1791013652.967 I: PowerVR BXS-4-64 MC1 alive (BVNC 36.53.104.796)
1791013652.967 I: GPU bring-up milestone 1 complete (power + BVNC readback)
1791013653.067 I: GPU core clock measured: 800 MHz (CR_TIMER 312502 ticks in 100 ms)
1791013653.078 I: GPU carveout phys 0x97800000 (64 MiB) mapped at 0x8e000000
1791013653.078 I: DDK FW: code@0x0 (196608 B), data@0x80000 (135168 B), objs from 0xc0000
1791013653.094 I: DDK FW: 229376-byte image for DDK 25.2 loaded (code 0x0, data 0x80000, rodata 0xa0000)
1791013653.094 I: runtime cfg: ActivePMLatency 5 ms
1791013653.099 I: FWIF structs built (KCCB/FWCCB/sysinit/osinit); heap used to 0xd8d28
1791013653.109 I: MMU: FW heap mapped (4096 KiB main + 192 KiB config of the 32 MiB window), PC root phys=0x97c30000
1791013653.119 I: M3 complete (MMU + FW load + FWIF structs). Booting FW core...
1791013653.124 I: FW boot: firmware_started=0x1 after ~1 ms
1791013653.129 I: GPU bring-up milestone 4 complete (firmware booted)
1791013653.141 I: DDK FW: skipping open-FW healthcheck; render KICK drives the FW
1791013653.144 I: render: scaffolding built; awaiting final cmd_transfer regs (pvr_render_regs.h) before first GPU draw
1791013653.148 I: D-PHY: 307812500 bit/s per lane, band 5
1791013653.152 I: DSI link up: 4 lanes, sync events, line 4047 B, 307812500 bit/s per lane
1791013653.177 I: sn65dsi8x: ID "58ISD" rev 0x01 (attempt 1)
1791013653.224 I: sn65dsi8x: up, DSI 153600 kHz (range 0x1e), LVDS 51200 kHz = /3 (range 1), fmt 0x68
1791013653.236 I: sn65dsi8x: link clean (IRQ_STAT 0x00)
1791013653.249 I: SN65DSI8x up: lvds range 1, dsi range 30, div 2, fmt 0x68, IRQ_STAT 0x00
1791013653.256 *** Booting Zephyr OS build af17c0c8aa2a ***
1791013653.256 Secondary CPU core 1 (MPID:0x1) is up
1791013653.261 Secondary CPU core 2 (MPID:0x2) is up
1791013653.266 Secondary CPU core 3 (MPID:0x3) is up
1791013653.272 gl_test: pvr_debug_masked_clear=1 (clears honour glColorMask)
1791013653.277 gl_test: PVR_BACKEND_OPT_DBIAS_INT=-1 (-1: untouched)
1791013653.280 gl_test: start (case=7 pretransform=0 ppp_ctrl=0x0 debug=0 no_coalesce=0)
1791013653.307 I: render: parameter buffer: carveout 64 MiB -> 448 pages listed, 4096 max, grow 256, pool 0 pages
1791013653.336 I: freelist id=1 np=512 max=4096 backing=1536 fpm=0 ebase=3584 arr.gpu=0xfa00000000 pages_phys=0x98048000 cur_dev=0xfa00003800 pfn[3584]=0x00098048 pfn[4095]=0x00098247
1791013653.382 I: freelist id=2 np=512 max=4096 backing=1536 fpm=0 ebase=3584 arr.gpu=0xfa00004000 pages_phys=0x9864c000 cur_dev=0xfa00007800 pfn[3584]=0x0009864c pfn[4095]=0x0009884b
1791013653.399 I: freelist id=3 np=26 max=32 backing=26 fpm=0 ebase=4 arr.gpu=0xfa00008000 pages_phys=0x98c50000 cur_dev=0xfa00008010 pfn[4]=0x00098c50 pfn[31]=0x00000000
1791013653.420 I: render: pds0: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000 f4000002 | @0x80: f0080101 f0090201  usc0: 0580a044 ff000000 00000000 00000000
1791013653.420 I: render: init ok (PB@0x8000000000 border@0x8000200000 eot_usc_off=0x1000 gen_b=0xe2c000 pds_b=0x1000 usc_b=0x6000)
1791013653.420 I: osal(zephyr): ready (regs=0xfd80000)
1791013653.528 I: pvr_backend: init ok (pds arena off 0x1000, gen arena 0x8000e2c000, vdm 0x800122c000, clear_fs off 0x8000)
1791013653.554 I: render: hc dwords: 00000000 003f003f 00000088 00000000 00000088 00000000 00000000 00000080 00003003 00000004 00000000 00080000 00080000 00000000 00000000 00000000
1791013653.559 I: render: hrt[0]: 0000c000 000000fa 00000000 00000000 00000000 00000000 00000000 00000000
1791013653.569 I: render: hrt[8]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013653.575 I: render: hrt[16]: 00000000 c00e9d00 00000000 00000000 c00d8da8 c00d8d28 c00d8d68 00000000
1791013653.585 I: render: hrt[24]: 00000000 00000000 01e8c000 00000080 00000000 00000000 00000000 00000000
1791013653.590 I: render: hrt[32]: 00000000 00000000 00000000 00000000 01e88000 00000080 01e9e000 00000080
1791013653.604 I: render: hrt[40]: 01e8e000 00000080 00000000 00000000 00000000 00000000 00000000 00000000
1791013653.609 I: render: hrt[48]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013653.614 I: render: hrt[56]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013653.677 I: render: HWRT 64x64 x1 te_screen=0x00003003 mtile1=0x00080000 mtile2=0x00080000 isp_mtile=0x00020002 stride=4 rgn=16 tpc=128 teaa=0
1791013653.682 gl_test: px                                 
1791013653.731 gl_test: px                                 
1791013653.731 gl_test: px                                 
1791013653.731 gl_test: px                                 
1791013653.731 gl_test: px                                 
1791013653.731 gl_test: px                 R               
1791013653.731 gl_test: px                RR               
1791013653.731 gl_test: px                RRR              
1791013653.731 gl_test: px               RRRR              
1791013653.731 gl_test: px               RRRRR             
1791013653.731 gl_test: px              RRRRRR             
1791013653.731 gl_test: px              RRRRRRR            
1791013653.731 gl_test: px             RRRRRRRR            
1791013653.731 gl_test: px             RRRRRRRRR           
1791013653.731 gl_test: px            RRRRRRRRRR           
1791013653.758 gl_test: px            RRRRRRRRRRR          
1791013653.758 gl_test: px           RRRRRRRRRRRR          
1791013653.758 gl_test: px           RRRRRRRRRRRRR         
1791013653.758 gl_test: px          RRRRRRRRRRRRRR         
1791013653.758 gl_test: px          RRRRRRRRRRRRRRR        
1791013653.758 gl_test: px         RRRRRRRRRRRRRRRR        
1791013653.758 gl_test: px         RRRRRRRRRRRRRRRRR       
1791013653.758 gl_test: px        RRRRRRRRRRRRRRRRRR       
1791013653.758 gl_test: px        RRRRRRRRRRRRRRRRRRR      
1791013653.758 gl_test: px       RRRRRRRRRRRRRRRRRRRR      
1791013653.758 gl_test: px       RRRRRRRRRRRRRRRRRRRRR     
1791013653.758 gl_test: px      RRRRRRRRRRRRRRRRRRRRRR     
1791013653.759 gl_test: px      RRRRRRRRRRRRRRRRRRRRRRR    
1791013653.759 gl_test: px                                 
1791013653.759 gl_test: px                                 
1791013653.759 gl_test: px                                 
1791013653.759 gl_test: px                                 
1791013653.760 gl_test:   orange centroid              (32,40) = 0xff0080ff  want 0xff0080ff  ok  [check 1]
1791013653.775 gl_test:   orange apex                  (31,12) = 0xff0080ff  want 0xff0080ff  ok  [check 2]
1791013653.785 gl_test:   background                   ( 1, 1) = 0xff000000  want 0xff000000  ok  [check 3]
1791013653.796 I: render: hc dwords: 00000000 003f003f 00000088 00000000 00000088 00000000 00000000 00000080 00003003 00000004 00000000 00080000 00080000 00000000 00000000 00000000
1791013653.806 I: render: hrt[0]: 00010000 000000fa 00000000 00000000 00000000 00000000 00000000 00000000
1791013653.811 I: render: hrt[8]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013653.822 I: render: hrt[16]: 00000000 c00e9e80 00000000 00000000 c00d8da8 c00d8d28 c00d8d68 00000000
1791013653.835 I: render: hrt[24]: 00000000 00000000 01ea3000 00000080 00000000 00000000 00000000 00000000
1791013653.840 I: render: hrt[32]: 00000000 00000000 00000000 00000000 01e9f000 00000080 01eb5000 00000080
1791013653.847 I: render: hrt[40]: 01ea5000 00000080 00000000 00000000 00000000 00000000 00000000 00000000
1791013653.864 I: render: hrt[48]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013653.865 I: render: hrt[56]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013653.877 I: render: HWRT 64x64 x1 te_screen=0x00003003 mtile1=0x00080000 mtile2=0x00080000 isp_mtile=0x00020002 stride=4 rgn=16 tpc=128 teaa=0
1791013653.887 gl_test:   cyan centroid                (32,40) = 0xffffff00  want 0xffffff00  ok  [check 4]
1791013653.894 gl_test:   active attributes 2; [1] = a_color size 1 type 0x8b52
1791013653.900 gl_test:   bound a_pos -> 3, a_color -> 1 (link 1)
1791013653.908 gl_test:   magenta via attribute 3      (32,40) = 0xffff00ff  want 0xffff00ff  ok  [check 5]
1791013653.916 gl_test: px                                 
1791013653.924 gl_test: px                                 
1791013653.924 gl_test: px                                 
1791013653.929 gl_test: px                                 
1791013653.936 gl_test: px                                 
1791013653.936 gl_test: px                                 
1791013653.941 gl_test: px                                 
1791013653.941 gl_test: px                                 
1791013653.945 gl_test: px                                 
1791013653.962 gl_test: px                                 
1791013653.963 gl_test: px                                 
1791013653.963 gl_test: px                 R               
1791013653.963 gl_test: px                RR               
1791013653.963 gl_test: px                RRR              
1791013653.964 gl_test: px               RRRR              
1791013653.978 gl_test: px               RRRRR             
1791013653.980 gl_test: px              RRRRRR             
1791013653.980 gl_test: px              RRRRRRR            
1791013653.985 gl_test: px             RRRRRRRR            
1791013653.991 gl_test: px             RRRRRRRRR           
1791013653.998 gl_test: px            RRRRRRRRRR           
1791013653.998 gl_test: px            RRRRRRRRRRR          
1791013654.004 gl_test: px                                 
1791013654.008 gl_test: px                                 
1791013654.008 gl_test: px                                 
1791013654.012 gl_test: px                                 
1791013654.018 gl_test: px                                 
1791013654.023 gl_test: px                                 
1791013654.029 gl_test: px                                 
1791013654.034 gl_test: px                                 
1791013654.034 gl_test: px                                 
1791013654.039 gl_test: px                                 
1791013654.048 gl_test:   half: inside small tri       (32,40) = 0xff00ffff  want 0xff00ffff  ok  [check 6]
1791013654.053 gl_test:   half: outside small tri      (32,14) = 0xff000000  want 0xff000000  ok  [check 7]
1791013654.070 gl_test:   half: outside small tri      (12,52) = 0xff000000  want 0xff000000  ok  [check 8]
1791013654.070 gl_test:   shift: moved right           (56,40) = 0xff00ffff  want 0xff00ffff  ok  [check 9]
1791013654.089 gl_test:   shift: left now empty        (16,40) = 0xff000000  want 0xff000000  ok  [check 10]
1791013654.091 gl_test:   active uniforms 2; [0] = u_mvp size 1 type 0x8b5c
1791013654.092 gl_test: case 1 uniform: PASS
1791013654.098 gl_test: 2x2 texture @0x8001ebf000: ff0000ff ff00ff00 ffff0000 ffffffff | row1 ffff0000 ffffffff
1791013654.109 gl_test: 2x2 texture on the full-screen quad:
1791013654.114 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.114 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.125 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.125 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.130 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.130 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.135 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.141 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.146 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.151 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.151 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.156 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.165 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.173 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.173 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.175 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.175 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.175 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.188 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.188 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.193 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.202 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.202 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.202 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.206 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.206 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.215 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.221 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.225 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.225 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.231 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.231 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.249 gl_test:   top-left = blue              (16,16) = 0xffff0000  want 0xffff0000  ok  [check 1]
1791013654.254 gl_test:   top-right = white            (48,16) = 0xffffffff  want 0xffffffff  ok  [check 2]
1791013654.267 gl_test:   bottom-left = red            (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 3]
1791013654.268 gl_test:   bottom-right = green         (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 4]
1791013654.273 gl_test: 4x4 texture @0x8001ec4000: ff0000ff ff0000ff ff00ff00 ff00ff00 | row1 ff0000ff ff0000ff
1791013654.283 gl_test: 4x4 texture on the full-screen quad:
1791013654.288 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.293 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.293 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.304 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.304 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.314 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.315 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.315 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.316 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.321 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.327 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.332 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.337 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.344 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.344 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.349 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.354 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.354 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.359 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.370 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.370 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.370 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.372 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.377 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.382 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.393 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.393 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.399 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.399 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.404 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.410 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.415 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.420 gl_test:   top-left = blue              (16,16) = 0xffff0000  want 0xffff0000  ok  [check 5]
1791013654.427 gl_test:   top-right = white            (48,16) = 0xffffffff  want 0xffffffff  ok  [check 6]
1791013654.432 gl_test:   bottom-left = red            (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 7]
1791013654.447 gl_test:   bottom-right = green         (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 8]
1791013654.452 gl_test: 8x8 texture @0x8001ebf000: ff0000ff ff0000ff ff0000ff ff0000ff | row1 ff0000ff ff0000ff
1791013654.456 gl_test: 8x8 texture on the full-screen quad:
1791013654.472 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.473 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.473 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.477 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.483 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.492 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.492 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.499 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.499 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.500 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.500 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.510 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.510 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.516 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.521 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.528 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.528 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.533 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.540 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.540 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.540 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.545 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.556 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.556 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.561 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.566 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.566 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.574 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.577 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.584 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.584 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.589 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.595 gl_test:   top-left = blue              (16,16) = 0xffff0000  want 0xffff0000  ok  [check 9]
1791013654.608 gl_test:   top-right = white            (48,16) = 0xffffffff  want 0xffffffff  ok  [check 10]
1791013654.608 gl_test:   bottom-left = red            (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 11]
1791013654.623 gl_test:   bottom-right = green         (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 12]
1791013654.628 gl_test: 64x64 texture @0x8001ec5000: ff0000ff ff0000ff ff0000ff ff0000ff | row1 ff0000ff ff0000ff
1791013654.638 gl_test: 64x64 texture on the full-screen quad:
1791013654.638 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.643 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.655 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.655 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.655 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.658 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.663 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.669 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.677 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.718 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.718 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.718 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.721 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.721 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.721 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.721 gl_test: px BBBBBBBBBBBBBBBBWWWWWWWWWWWWWWWW
1791013654.721 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.721 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.721 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.721 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.721 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.726 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.726 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.731 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.738 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.743 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.749 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.749 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.754 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.754 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.758 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.765 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013654.771 gl_test:   top-left = blue              (16,16) = 0xffff0000  want 0xffff0000  ok  [check 13]
1791013654.780 gl_test:   top-right = white            (48,16) = 0xffffffff  want 0xffffffff  ok  [check 14]
1791013654.789 gl_test:   bottom-left = red            (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 15]
1791013654.799 gl_test:   bottom-right = green         (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 16]
1791013654.810 gl_test: gradient texture (R = 36 x, G = 36 y, B = 128), 8x8 grid (top-down rows):
1791013654.814 gl_test:  row  4: ff80fc00 ff80fc24 ff80fc48 ff80fc6c ff80fc90 ff80fcb4 ff80fcd8 ff80fcfc
1791013654.824 gl_test:  row 12: ff80d800 ff80d824 ff80d848 ff80d86c ff80d890 ff80d8b4 ff80d8d8 ff80d8fc
1791013654.833 gl_test:  row 20: ff80b400 ff80b424 ff80b448 ff80b46c ff80b490 ff80b4b4 ff80b4d8 ff80b4fc
1791013654.838 gl_test:  row 28: ff809000 ff809024 ff809048 ff80906c ff809090 ff8090b4 ff8090d8 ff8090fc
1791013654.849 gl_test:  row 36: ff806c00 ff806c24 ff806c48 ff806c6c ff806c90 ff806cb4 ff806cd8 ff806cfc
1791013654.859 gl_test:  row 44: ff804800 ff804824 ff804848 ff80486c ff804890 ff8048b4 ff8048d8 ff8048fc
1791013654.865 gl_test:  row 52: ff802400 ff802424 ff802448 ff80246c ff802490 ff8024b4 ff8024d8 ff8024fc
1791013654.886 gl_test:  row 60: ff800000 ff800024 ff800048 ff80006c ff800090 ff8000b4 ff8000d8 ff8000fc
1791013654.886 gl_test:   texel (0,7) at top-left      ( 4, 4) = 0xff80fc00  want 0xff80fc00  ok  [check 17]
1791013654.886 gl_test:   texel (7,0) at bottom-right  (60,60) = 0xff8000fc  want 0xff8000fc  ok  [check 18]
1791013654.894 gl_test:   texel (2,5)                  (20,20) = 0xff80b448  want 0xff80b448  ok  [check 19]
1791013654.991 gl_test:   texel (5,2)                  (44,44) = 0xff8048b4  want 0xff8048b4  ok  [check 20]
1791013655.000 gl_test:   mip: magnified -> level 0    (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 21]
1791013655.000 gl_test:   mip: 2x2 px quad -> level 1  (30,30) = 0xff00ff00  want 0xff00ff00  ok  [check 22]
1791013655.000 gl_test:   mip: 1x1 px quad -> level 2  (40,40) = 0xffff0000  want 0xffff0000  ok  [check 23]
1791013655.000 gl_test:   generated level 2 = average  (40,40) = 0xff800080  want 0xff800080  ok  [check 24]
1791013655.000 gl_test:   trilinear at level 2         (40,40) = 0xff800080  want 0xff800080  ok  [check 25]
1791013655.000 gl_test: 4x4 texture @0x8001ec8000: ff0000ff ff0000ff ff00ff00 ff00ff00 | row1 ff0000ff ff0000ff
1791013655.000 gl_test:   2 units: red*0.5 + yellow*0.25 (16,48) = 0xff0040bf  want 0xff0040bf  ok  [check 26]
1791013655.000 gl_test:   2 units: blue*0.5 + yellow*0.25 (16,16) = 0xff804040  want 0xff804040  ok  [check 27]
1791013655.015 gl_test:   swapped: yellow*0.5 + red*0.25 (16,48) = 0xff0080bf  want 0xff0080bf  ok  [check 28]
1791013655.022 gl_test:   swapped: yellow*0.5 + blue*0.25 (16,16) = 0xff408080  want 0xff408080  ok  [check 29]
1791013655.022 gl_test: case 2 texture: PASS
1791013655.027 gl_test: px                                 
1791013655.027 gl_test: px                                 
1791013655.034 gl_test: px                 R               
1791013655.034 gl_test: px                RR               
1791013655.041 gl_test: px                RRR              
1791013655.043 gl_test: px               RRRR              
1791013655.043 gl_test: px               RRRRR             
1791013655.049 gl_test: px              RRRRRR             
1791013655.055 gl_test: px              RRRRRRR            
1791013655.059 gl_test: px             RRRRRRRR            
1791013655.059 gl_test: px             RRRRRRRRR           
1791013655.064 gl_test: px            RRRRRRRRRR           
1791013655.069 gl_test: px            RRRRRRRRRRR          
1791013655.073 gl_test: px           RRRRRRBRRRRR          
1791013655.078 gl_test: px           RRRRRBBRRRRRR         
1791013655.087 gl_test: px          RRRRRRBBBRRRRR         
1791013655.087 gl_test: px          RRRRRBBBBRRRRRR        
1791013655.089 gl_test: px         RRRRRRBBBBBRRRRR        
1791013655.094 gl_test: px         RRRRRBBBBBBRRRRRR       
1791013655.103 gl_test: px        RRRRRRBBBBBBBRRRRR       
1791013655.104 gl_test: px        RRRRRRRRRRRRRRRRRRR      
1791013655.104 gl_test: px       RRRRRRRRRRRRRRRRRRRR      
1791013655.110 gl_test: px       RRRRRRRRRRRRRRRRRRRRR     
1791013655.114 gl_test: px      RRRRRRRRRRRRRRRRRRRRRR     
1791013655.120 gl_test: px      RRRRRRRRRRRRRRRRRRRRRRR    
1791013655.120 gl_test: px     RRRRRRRRRRRRRRRRRRRRRRRR    
1791013655.129 gl_test: px     RRRRRRRRRRRRRRRRRRRRRRRRR   
1791013655.129 gl_test: px    RRRRRRRRRRRRRRRRRRRRRRRRRR   
1791013655.134 gl_test: px    RRRRRRRRRRRRRRRRRRRRRRRRRRR  
1791013655.140 gl_test: px   RRRRRRRRRRRRRRRRRRRRRRRRRRRR  
1791013655.140 gl_test: px   RRRRRRRRRRRRRRRRRRRRRRRRRRRRR 
1791013655.150 gl_test: px                                 
1791013655.154 gl_test:   depth on: C in front         (32,32) = 0xffff0000  want 0xffff0000  ok  [check 1]
1791013655.159 gl_test:   depth on: B hidden           (32,44) = 0xff0000ff  want 0xff0000ff  ok  [check 2]
1791013655.170 gl_test:   depth on: A only             (32,54) = 0xff0000ff  want 0xff0000ff  ok  [check 3]
1791013655.189 gl_test:   depth off: C last            (32,32) = 0xffff0000  want 0xffff0000  ok  [check 4]
1791013655.190 gl_test:   depth off: B over A          (32,44) = 0xff00ff00  want 0xff00ff00  ok  [check 5]
1791013655.194 gl_test:   depth off: A only            (32,54) = 0xff0000ff  want 0xff0000ff  ok  [check 6]
1791013655.216 gl_test:   greater: B over A, C         (32,32) = 0xff00ff00  want 0xff00ff00  ok  [check 7]
1791013655.223 gl_test:   greater: B over A            (32,44) = 0xff00ff00  want 0xff00ff00  ok  [check 8]
1791013655.227 gl_test:   greater: A only              (32,54) = 0xff0000ff  want 0xff0000ff  ok  [check 9]
1791013655.237 gl_test:   offset -4: D in front of A   (32,54) = 0xff00ff00  want 0xff00ff00  ok  [check 10]
1791013655.258 gl_test:   offset +4: A stays           (32,54) = 0xff0000ff  want 0xff0000ff  ok  [check 11]
1791013655.258 gl_test: case 3 depth: PASS
1791013655.260 gl_test: px                                 
1791013655.260 gl_test: px                                 
1791013655.266 gl_test: px                                 
1791013655.272 gl_test: px                                 
1791013655.279 gl_test: px                                 
1791013655.282 gl_test: px                 R               
1791013655.282 gl_test: px                RR               
1791013655.291 gl_test: px                RRR              
1791013655.294 gl_test: px               RRRR              
1791013655.298 gl_test: px               RRRRR             
1791013655.298 gl_test: px              RRRRRR             
1791013655.303 gl_test: px              RRRRRRR            
1791013655.310 gl_test: px             RRRRRRRR            
1791013655.314 gl_test: px             RRRRRRRRR           
1791013655.314 gl_test: px            RRRRRRRRRR           
1791013655.319 gl_test: px            RRRRRRRRRRR          
1791013655.328 gl_test: px           RRRRRRRRRRRR          
1791013655.329 gl_test: px           RRRRRRRRRRRRR         
1791013655.334 gl_test: px          RRRRRRRRRRRRRR         
1791013655.334 gl_test: px          RRRRRRRRRRRRRRR        
1791013655.339 gl_test: px         RRRRRRRRRRRRRRRR        
1791013655.345 gl_test: px         RRRRRRRRRRRRRRRRR       
1791013655.351 gl_test: px        RRRRRRRRRRRRRRRRRR       
1791013655.351 gl_test: px        RRRRRRRRRRRRRRRRRRR      
1791013655.355 gl_test: px       RRRRRRRRRRRRRRRRRRRR      
1791013655.359 gl_test: px       RRRRRRRRRRRRRRRRRRRRR     
1791013655.367 gl_test: px      RRRRRRRRRRRRRRRRRRRRRR     
1791013655.368 gl_test: px      RRRRRRRRRRRRRRRRRRRRRRR    
1791013655.373 gl_test: px                                 
1791013655.373 gl_test: px                                 
1791013655.382 gl_test: px                                 
1791013655.382 gl_test: px                                 
1791013655.394 gl_test:   ccw front, cull back         (32,40) = 0xff00ffff  want 0xff00ffff  ok  [check 1]
1791013655.405 gl_test:   ccw front, cull front        (32,40) = 0xff000000  want 0xff000000  ok  [check 2]
1791013655.415 gl_test:   cw front, cull back          (32,40) = 0xff000000  want 0xff000000  ok  [check 3]
1791013655.430 gl_test:   cw front, cull front         (32,40) = 0xff00ffff  want 0xff00ffff  ok  [check 4]
1791013655.441 gl_test:   fbo: ccw front, cull back    (32,32) = 0xff00ffff  want 0xff00ffff  ok  [check 5]
1791013655.449 gl_test:   fbo: ccw front, cull front   (32,32) = 0xff000000  want 0xff000000  ok  [check 6]
1791013655.455 gl_test: case 4 cull: PASS
1791013655.465 gl_test: px                                 
1791013655.465 gl_test: px                                 
1791013655.470 gl_test: px                                 
1791013655.477 gl_test: px                                 
1791013655.479 gl_test: px                                 
1791013655.479 gl_test: px                                 
1791013655.484 gl_test: px                                 
1791013655.489 gl_test: px                                 
1791013655.496 gl_test: px                                 
1791013655.501 gl_test: px                                 
1791013655.501 gl_test: px                                 
1791013655.507 gl_test: px                                 
1791013655.512 gl_test: px                                 
1791013655.512 gl_test: px                                 
1791013655.518 gl_test: px                                 
1791013655.523 gl_test: px                                 
1791013655.523 gl_test: px RRRRRRRRRRRRRRRR                
1791013655.528 gl_test: px RRRRRRRRRRRRRRRR                
1791013655.532 gl_test: px RRRRRRRRRRRRRRRR                
1791013655.538 gl_test: px RRRRRRRRRRRRRRRR                
1791013655.543 gl_test: px RRRRRRRRRRRRRRRR                
1791013655.554 gl_test: px RRRRRRRRRRRRRRRR                
1791013655.554 gl_test: px RRRRRRRRRRRRRRRR                
1791013655.554 gl_test: px RRRRRRRRRRRRRRRR                
1791013655.556 gl_test: px RRRRRRRRRRRRRRRR                
1791013655.561 gl_test: px RRRRRRRRRRRRRRRR                
1791013655.567 gl_test: px RRRRRRRRRRRRRRRR                
1791013655.572 gl_test: px RRRRRRRRRRRRRRRR                
1791013655.572 gl_test: px RRRRRRRRRRRRRRRR                
1791013655.577 gl_test: px RRRRRRRRRRRRRRRR                
1791013655.584 gl_test: px RRRRRRRRRRRRRRRR                
1791013655.587 gl_test: px RRRRRRRRRRRRRRRR                
1791013655.592 gl_test:   inside (bottom-left)         (16,48) = 0xff00ffff  want 0xff00ffff  ok  [check 1]
1791013655.602 gl_test:   inside edge                  (31,32) = 0xff00ffff  want 0xff00ffff  ok  [check 2]
1791013655.615 gl_test:   outside right                (32,32) = 0xff000000  want 0xff000000  ok  [check 3]
1791013655.620 gl_test:   outside above                (16,31) = 0xff000000  want 0xff000000  ok  [check 4]
1791013655.624 gl_test:   outside (top-right)          (48,16) = 0xff000000  want 0xff000000  ok  [check 5]
1791013655.629 gl_test: case 5 scissor: PASS
1791013655.640 gl_test: FBO texture contents (via glReadPixels on the FBO):
1791013655.640 gl_test: px                                 
1791013655.645 gl_test: px                                 
1791013655.649 gl_test: px                                 
1791013655.654 gl_test: px                                 
1791013655.659 gl_test: px                                 
1791013655.669 gl_test: px                 R               
1791013655.669 gl_test: px                RR               
1791013655.669 gl_test: px                RRR              
1791013655.673 gl_test: px               RRRR              
1791013655.678 gl_test: px               RRRRR             
1791013655.683 gl_test: px              RRRRRR             
1791013655.692 gl_test: px              RRRRRRR            
1791013655.692 gl_test: px             RRRRRRRR            
1791013655.693 gl_test: px             RRRRRRRRR           
1791013655.701 gl_test: px            RRRRRRRRRR           
1791013655.701 gl_test: px            RRRRRRRRRRR          
1791013655.706 gl_test: px           GRRRRRRRRRRR          
1791013655.716 gl_test: px           GGRRRRRRRRBBB         
1791013655.722 gl_test: px          GGGGGRRRRRBBBB         
1791013655.722 gl_test: px          GGGGGGRRBBBBBBB        
1791013655.728 gl_test: px         GGGGGGGGBBBBBBBB        
1791013655.728 gl_test: px         GGGGGGGGBBBBBBBBB       
1791013655.734 gl_test: px        GGGGGGGGGBBBBBBBBB       
1791013655.734 gl_test: px        GGGGGGGGGBBBBBBBBBB      
1791013655.738 gl_test: px       GGGGGGGGGGBBBBBBBBBB      
1791013655.743 gl_test: px       GGGGGGGGGGBBBBBBBBBBB     
1791013655.743 gl_test: px      GGGGGGGGGGGBBBBBBBBBBB     
1791013655.749 gl_test: px      GGGGGGGGGGGBBBBBBBBBBBB    
1791013655.755 gl_test: px                                 
1791013655.760 gl_test: px                                 
1791013655.760 gl_test: px                                 
1791013655.764 gl_test: px                                 
1791013655.779 gl_test:   v0 red apex                  (31,12) = 0xff090fe7  want 0xff090fe7  ok  [check 1]
1791013655.779 gl_test:   v1 green                     (12,52) = 0xff0fde13  want 0xff0fde13  ok  [check 2]
1791013655.788 gl_test:   v2 blue                      (52,52) = 0xffe30913  want 0xffe30913  ok  [check 3]
1791013655.808 gl_test:   centroid                     (32,40) = 0xff595452  want 0xff595452  ok  [check 4]
1791013655.808 gl_test:   background                   ( 1, 1) = 0xff000000  want 0xff000000  ok  [check 5]
1791013655.812 gl_test: window contents (the texture sampled 1:1):
1791013655.816 gl_test: px                                 
1791013655.828 gl_test: px                                 
1791013655.828 gl_test: px                                 
1791013655.832 gl_test: px                                 
1791013655.837 gl_test: px                                 
1791013655.845 gl_test: px                 R               
1791013655.845 gl_test: px                RR               
1791013655.849 gl_test: px                RRR              
1791013655.853 gl_test: px               RRRR              
1791013655.853 gl_test: px               RRRRR             
1791013655.858 gl_test: px              RRRRRR             
1791013655.863 gl_test: px              RRRRRRR            
1791013655.869 gl_test: px             RRRRRRRR            
1791013655.874 gl_test: px             RRRRRRRRR           
1791013655.874 gl_test: px            RRRRRRRRRR           
1791013655.879 gl_test: px            RRRRRRRRRRR          
1791013655.884 gl_test: px           GRRRRRRRRRRR          
1791013655.891 gl_test: px           GGRRRRRRRRBBB         
1791013655.891 gl_test: px          GGGGGRRRRRBBBB         
1791013655.913 gl_test: px          GGGGGGRRBBBBBBB        
1791013655.913 gl_test: px         GGGGGGGGBBBBBBBB        
1791013655.913 gl_test: px         GGGGGGGGBBBBBBBBB       
1791013655.913 gl_test: px        GGGGGGGGGBBBBBBBBB       
1791013655.913 gl_test: px        GGGGGGGGGBBBBBBBBBB      
1791013655.913 gl_test: px       GGGGGGGGGGBBBBBBBBBB      
1791013655.918 gl_test: px       GGGGGGGGGGBBBBBBBBBBB     
1791013655.922 gl_test: px      GGGGGGGGGGGBBBBBBBBBBB     
1791013655.929 gl_test: px      GGGGGGGGGGGBBBBBBBBBBBB    
1791013655.937 gl_test: px                                 
1791013655.937 gl_test: px                                 
1791013655.942 gl_test: px                                 
1791013655.942 gl_test: px                                 
1791013655.953 gl_test:   v0 red apex                  (31,12) = 0xff090fe7  want 0xff090fe7  ok  [check 6]
1791013655.959 gl_test:   v1 green                     (12,52) = 0xff0fde13  want 0xff0fde13  ok  [check 7]
1791013655.969 gl_test:   v2 blue                      (52,52) = 0xffe30913  want 0xffe30913  ok  [check 8]
1791013655.974 gl_test:   centroid                     (32,40) = 0xff595452  want 0xff595452  ok  [check 9]
1791013655.983 gl_test:   background                   ( 1, 1) = 0xff000000  want 0xff000000  ok  [check 10]
1791013655.988 gl_test:   sampled == rendered          0 pixel(s) differ
1791013655.993 gl_test: case 6 fbo: PASS
1791013656.011 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.020 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.020 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.020 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.020 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.020 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.020 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.026 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.031 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.037 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.042 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.042 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.047 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.055 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.061 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.061 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.066 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.066 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.071 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.078 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.081 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.081 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.087 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.087 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.097 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.097 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.102 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.102 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.118 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.118 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.118 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.118 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013656.131 gl_test:   over: inside triangle        (32,40) = 0xbf008080  want 0xbf008080  ok  [check 1]
1791013656.141 gl_test:   over: outside (red quad)     ( 4, 4) = 0xff0000ff  want 0xff0000ff  ok  [check 2]
1791013656.151 gl_test:   additive: inside triangle    (32,40) = 0xff00ffff  want 0xff00ffff  ok  [check 3]
1791013656.162 gl_test:   premultiplied: inside        (32,40) = 0xff00ff80  want 0xff00ff80  ok  [check 4]
1791013656.173 gl_test:   replace: inside              (32,40) = 0x8000ff00  want 0x8000ff00  ok  [check 5]
1791013656.186 gl_test:   constant colour: inside      (32,40) = 0x800080bf  want 0x800080bf  ok  [check 6]
1791013656.197 gl_test:   mask ga: inside              (32,40) = 0x8000ffff  want 0x8000ffff  ok  [check 7]
1791013656.213 gl_test:   mask ga: outside untouched   ( 4, 4) = 0xff0000ff  want 0xff0000ff  ok  [check 8]
1791013656.216 gl_test:   over, alpha masked: inside   (32,40) = 0xff008080  want 0xff008080  ok  [check 9]
1791013656.216 gl_test: case 8 blend: PASS
1791013656.236 gl_test: px                                 
1791013656.236 gl_test: px                                 
1791013656.236 gl_test: px                                 
1791013656.239 gl_test: px                                 
1791013656.239 gl_test: px                                 
1791013656.250 gl_test: px                 R               
1791013656.255 gl_test: px                RR               
1791013656.255 gl_test: px                RRR              
1791013656.262 gl_test: px               RRRR              
1791013656.266 gl_test: px               RRRRR             
1791013656.266 gl_test: px              RRRRRR             
1791013656.272 gl_test: px              RRRRRRR            
1791013656.277 gl_test: px             RRRRRRRR            
1791013656.281 gl_test: px             RRRRRRRRR           
1791013656.281 gl_test: px            RRRRRRRRRR           
1791013656.287 gl_test: px            RRRRRRRRRRR          
1791013656.293 gl_test: px           RRRRRRRRRRRR          
1791013656.297 gl_test: px           RRRRRRRRRRRRR         
1791013656.297 gl_test: px          RRRRRRRRRRRRRR         
1791013656.303 gl_test: px          RRRRRRRRRRRRRRR        
1791013656.315 gl_test: px         RRRRRRRRRRRRRRRR        
1791013656.316 gl_test: px         RRRRRRRRRRRRRRRRR       
1791013656.316 gl_test: px        RRRRRRRRRRRRRRRRRR       
1791013656.316 gl_test: px        RRRRRRRRRRRRRRRRRRR      
1791013656.320 gl_test: px       RRRRRRRRRRRRRRRRRRRR      
1791013656.331 gl_test: px       RRRRRRRRRRRRRRRRRRRRR     
1791013656.331 gl_test: px      RRRRRRRRRRRRRRRRRRRRRR     
1791013656.349 gl_test: px      RRRRRRRRRRRRRRRRRRRRRRR    
1791013656.349 gl_test: px                                 
1791013656.349 gl_test: px                                 
1791013656.349 gl_test: px                                 
1791013656.349 gl_test: px                                 
1791013656.357 gl_test:   equal 1: inside mask         (32,40) = 0xff00ffff  want 0xff00ffff  ok  [check 1]
1791013656.367 gl_test:   equal 1: outside mask        ( 4, 4) = 0xff000000  want 0xff000000  ok  [check 2]
1791013656.377 gl_test:   mask pass wrote no colour    (32,50) = 0xff00ffff  want 0xff00ffff  ok  [check 3]
1791013656.389 gl_test:   notequal 1: inside mask      (32,40) = 0xff000000  want 0xff000000  ok  [check 4]
1791013656.394 gl_test:   notequal 1: outside mask     ( 4, 4) = 0xff00ffff  want 0xff00ffff  ok  [check 5]
1791013656.408 gl_test:   incr x2: inside == 2         (32,40) = 0xff00ffff  want 0xff00ffff  ok  [check 6]
1791013656.419 gl_test:   incr x2: outside == 0        ( 4, 4) = 0xff000000  want 0xff000000  ok  [check 7]
1791013656.427 gl_test:   cleared to 1: quad everywhere ( 4, 4) = 0xff00ffff  want 0xff00ffff  ok  [check 8]
1791013656.440 gl_test:   write mask 0xfe: nothing     (32,40) = 0xff000000  want 0xff000000  ok  [check 9]
1791013656.445 gl_test: case 9 stencil: PASS
1791013656.461 gl_test:   fan: centre                  (32,32) = 0xffff00ff  want 0xffff00ff  ok  [check 1]
1791013656.461 gl_test:   fan: corner                  ( 2,61) = 0xffff00ff  want 0xffff00ff  ok  [check 2]
1791013656.467 gl_test:   fan: corner                  (61, 2) = 0xffff00ff  want 0xffff00ff  ok  [check 3]
1791013656.482 gl_test:   u8 indices in a VBO          (32,32) = 0xffffff00  want 0xffffff00  ok  [check 4]
1791013656.501 gl_test:   u32 client indices           (32,32) = 0xff00ffff  want 0xff00ffff  ok  [check 5]
1791013656.506 gl_test:   ubyte colours in a VBO       (32,32) = 0xffff0000  want 0xffff0000  ok  [check 6]
1791013656.511 gl_test: px                                 
1791013656.520 gl_test: px                                 
1791013656.521 gl_test: px                                 
1791013656.524 gl_test: px                                 
1791013656.529 gl_test: px                                 
1791013656.534 gl_test: px                                 
1791013656.539 gl_test: px                                 
1791013656.539 gl_test: px                                 
1791013656.549 gl_test: px         GGGGGGGGGGGGGGGGG       
1791013656.549 gl_test: px         G               G       
1791013656.567 gl_test: px         G               G       
1791013656.568 gl_test: px         G               G       
1791013656.568 gl_test: px         G               G       
1791013656.568 gl_test: px         G               G       
1791013656.568 gl_test: px         G               G       
1791013656.572 gl_test: px         G               G       
1791013656.575 gl_test: px         G               G       
1791013656.581 gl_test: px         G               G       
1791013656.586 gl_test: px         G               G       
1791013656.593 gl_test: px         G               G       
1791013656.593 gl_test: px         G               G       
1791013656.596 gl_test: px         G               G       
1791013656.603 gl_test: px         G               G       
1791013656.610 gl_test: px         G               G       
1791013656.623 gl_test: px         GGGGGGGGGGGGGGGG        
1791013656.623 gl_test: px                                 
1791013656.623 gl_test: px                                 
1791013656.623 gl_test: px                                 
1791013656.623 gl_test: px                                 
1791013656.623 gl_test: px                                 
1791013656.627 gl_test: px                                 
1791013656.637 gl_test: px                                 
1791013656.650 gl_test:   loop: left edge              (16,32) = 0xff00ff00  want 0xff00ff00  ok  [check 7]
1791013656.655 gl_test:   loop: top edge               (32,16) = 0xff00ff00  want 0xff00ff00  ok  [check 8]
1791013656.661 gl_test:   loop: right edge             (48,32) = 0xff00ff00  want 0xff00ff00  ok  [check 9]
1791013656.670 gl_test:   loop: bottom edge (closing)  (32,48) = 0xff00ff00  want 0xff00ff00  ok  [check 10]
1791013656.673 gl_test:   loop: 3 px wide              (17,32) = 0xff00ff00  want 0xff00ff00  ok  [check 11]
1791013656.687 gl_test:   loop: inside empty           (32,32) = 0xff000000  want 0xff000000  ok  [check 12]
1791013656.691 gl_test:   loop: outside empty          ( 4, 4) = 0xff000000  want 0xff000000  ok  [check 13]
1791013656.709 gl_test:   lines: on the horizontal     (32,40) = 0xffffffff  want 0xffffffff  ok  [check 14]
1791013656.714 gl_test:   lines: row above empty       (32,38) = 0xff000000  want 0xff000000  ok  [check 15]
1791013656.768 gl_test:   lines: row below empty       (32,42) = 0xff000000  want 0xff000000  ok  [check 16]
1791013656.778 gl_test:   lines: beyond the end        (60,40) = 0xff000000  want 0xff000000  ok  [check 17]
1791013656.778 gl_test:   lines: on the vertical       (20,16) = 0xffffffff  want 0xffffffff  ok  [check 18]
1791013656.778 gl_test:   lines: column beside empty   (22,16) = 0xff000000  want 0xff000000  ok  [check 19]
1791013656.778 gl_test:   strip: first segment         ( 8,32) = 0xff0000ff  want 0xff0000ff  ok  [check 20]
1791013656.778 gl_test:   strip: second segment        (32,56) = 0xff0000ff  want 0xff0000ff  ok  [check 21]
1791013656.778 gl_test:   strip: not closed            (32, 8) = 0xff000000  want 0xff000000  ok  [check 22]
1791013656.827 gl_test:   3 px point at (40,24): .........
1791013656.827 gl_test:   3 px point at (40,24): .........
1791013656.830 gl_test:   3 px point at (40,24): .........
1791013656.830 gl_test:   3 px point at (40,24): ...###...
1791013656.830 gl_test:   3 px point at (40,24): ...###...
1791013656.830 gl_test:   3 px point at (40,24): ...###...
1791013656.830 gl_test:   3 px point at (40,24): .........
1791013656.830 gl_test:   3 px point at (40,24): .........
1791013656.830 gl_test:   3 px point at (40,24): .........
1791013656.830 gl_test:   1 px point: centre           ( 8, 8) = 0xffff0000  want 0xffff0000  ok  [check 23]
1791013656.830 gl_test:   1 px point: neighbour empty  ( 9, 8) = 0xff000000  want 0xff000000  ok  [check 24]
1791013656.841 gl_test:   1 px point: neighbour empty  ( 8, 9) = 0xff000000  want 0xff000000  ok  [check 25]
1791013656.847 gl_test:   3 px point: centre           (40,24) = 0xffff0000  want 0xffff0000  ok  [check 26]
1791013656.857 gl_test:   3 px point: corner           (41,25) = 0xffff0000  want 0xffff0000  ok  [check 27]
1791013656.862 gl_test:   3 px point: corner           (39,23) = 0xffff0000  want 0xffff0000  ok  [check 28]
1791013656.872 gl_test:   3 px point: outside          (42,24) = 0xff000000  want 0xff000000  ok  [check 29]
1791013656.877 gl_test:   3 px point: outside          (40,22) = 0xff000000  want 0xff000000  ok  [check 30]
1791013656.887 gl_test:   5 px point: edge             (58,56) = 0xffff0000  want 0xffff0000  ok  [check 31]
1791013656.905 gl_test:   5 px point: outside          (59,56) = 0xff000000  want 0xff000000  ok  [check 32]
1791013656.905 gl_test: case 10 prims: PASS
1791013656.906 gl_test:   GL_OES_vertex_array_object advertised = 1  want 1  ok  [check 1]
1791013656.909 gl_test:   glGenVertexArrays returns a nonzero name = 1  want 1  ok  [check 2]
1791013656.919 gl_test:   generated name is not a VAO yet = 0  want 0  ok  [check 3]
1791013656.930 gl_test:   bound name is a VAO          = 1  want 1  ok  [check 4]
1791013656.930 gl_test:   VERTEX_ARRAY_BINDING == vao  = 1  want 1  ok  [check 5]
1791013656.940 gl_test:   vao: centroid                (32,40) = 0xff595452  want 0xff595452  ok  [check 6]
1791013656.945 gl_test:   default VAO attr0 disabled   = 0  want 0  ok  [check 7]
1791013656.957 gl_test:   default VAO: centroid empty  (32,40) = 0xff000000  want 0xff000000  ok  [check 8]
1791013656.961 gl_test:   VAO restored attr0 enabled   = 1  want 1  ok  [check 9]
1791013656.974 gl_test:   vao rebound: centroid        (32,40) = 0xff595452  want 0xff595452  ok  [check 10]
1791013656.982 gl_test:   deleted name is not a VAO    = 0  want 0  ok  [check 11]
1791013656.982 gl_test: case 22 vao: PASS
1791013656.995 gl_test: px                                 
1791013656.998 gl_test: px                                 
1791013657.003 gl_test: px   R                             
1791013657.008 gl_test: px                                 
1791013657.018 gl_test: px                                 
1791013657.018 gl_test: px                 R               
1791013657.023 gl_test: px                RR               
1791013657.023 gl_test: px                RRR              
1791013657.032 gl_test: px               RRRR              
1791013657.033 gl_test: px               RRRRR             
1791013657.033 gl_test: px              RRRRRR             
1791013657.039 gl_test: px              RRRRRRR            
1791013657.043 gl_test: px             RRRRRRRR            
1791013657.049 gl_test: px             RRRRRRRRR           
1791013657.055 gl_test: px            RRRRRRRRRR           
1791013657.055 gl_test: px            RRRRRRRRRRR          
1791013657.061 gl_test: px           GRRRRRRRRRRR          
1791013657.066 gl_test: px           GGRRRRRRRRBBB         
1791013657.066 gl_test: px          GGGGGRRRRRBBBB         
1791013657.070 gl_test: px          GGGGGGRRBBBBBBB        
1791013657.076 gl_test: px         GGGGGGGGBBBBBBBB        
1791013657.081 gl_test: px         GGGGGGGGBBBBBBBBB       
1791013657.086 gl_test: px        GGGGGGGGGBBBBBBBBB       
1791013657.091 gl_test: px        GGGGGGGGGBBBBBBBBBB      
1791013657.091 gl_test: px       GGGGGGGGGGBBBBBBBBBB      
1791013657.103 gl_test: px       GGGGGGGGGGBBBBBBBBBBB     
1791013657.104 gl_test: px      GGGGGGGGGGGBBBBBBBBBBB     
1791013657.104 gl_test: px      GGGGGGGGGGGBBBBBBBBBBBB    
1791013657.104 gl_test: px                                 
1791013657.107 gl_test: px                                 
1791013657.117 gl_test: px                                 
1791013657.123 gl_test: px                                 
1791013657.138 gl_test:   earlier frame kept           (32,40) = 0xff595452  want 0xff595452  ok  [check 1]
1791013657.138 gl_test:   earlier frame kept           (31,12) = 0xff090fe7  want 0xff090fe7  ok  [check 2]
1791013657.142 gl_test:   new draw                     ( 4, 4) = 0xff00ffff  want 0xff00ffff  ok  [check 3]
1791013657.150 gl_test:   still clear elsewhere        (60, 4) = 0xff000000  want 0xff000000  ok  [check 4]
1791013657.171 gl_test:   no-clear 113: draw 112       (40,24) = 0xff00ffff  want 0xff00ffff  ok  [check 5]
1791013657.185 gl_test:   no-clear 113: earlier kept   (32,40) = 0xff595452  want 0xff595452  ok  [check 6]
1791013657.222 gl_test: px                                 
1791013657.230 gl_test: px                                 
1791013657.244 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013657.244 gl_test: px                                 
1791013657.244 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013657.244 gl_test: px                                 
1791013657.244 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013657.254 gl_test: px                                 
1791013657.259 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013657.266 gl_test: px                                 
1791013657.266 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013657.270 gl_test: px                                 
1791013657.275 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013657.275 gl_test: px                                 
1791013657.281 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013657.288 gl_test: px                                 
1791013657.288 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013657.292 gl_test: px                                 
1791013657.297 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013657.297 gl_test: px                                 
1791013657.307 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013657.307 gl_test: px                                 
1791013657.312 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013657.320 gl_test: px                                 
1791013657.320 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013657.322 gl_test: px                                 
1791013657.328 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013657.339 gl_test: px                                 
1791013657.340 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013657.340 gl_test: px                                 
1791013657.341 gl_test: px   R  R  R  R  R  R  R  R  R  R  
1791013657.347 gl_test: px                                 
1791013657.358 gl_test:   draw 0                       ( 4, 4) = 0xff00ffff  want 0xff00ffff  ok  [check 7]
1791013657.369 gl_test:   draw 255 (last of kick 1)    (49,52) = 0xff00ffff  want 0xff00ffff  ok  [check 8]
1791013657.376 gl_test:   draw 256 (first of kick 2)   (52,52) = 0xff00ffff  want 0xff00ffff  ok  [check 9]
1791013657.384 gl_test:   draw 299                     (61,60) = 0xff00ffff  want 0xff00ffff  ok  [check 10]
1791013657.390 gl_test:   triangle from frame 1 cleared (32,40) = 0xff000000  want 0xff000000  ok  [check 11]
1791013657.394 gl_test: case 11 persist: PASS
1791013657.399 gl_test: px                                 
1791013657.405 gl_test: px                                 
1791013657.411 gl_test: px                                 
1791013657.411 gl_test: px                                 
1791013657.415 gl_test: px                                 
1791013657.421 gl_test: px                                 
1791013657.429 gl_test: px                                 
1791013657.430 gl_test: px                                 
1791013657.430 gl_test: px                                 
1791013657.442 gl_test: px         R               G       
1791013657.442 gl_test: px        RR              GG       
1791013657.444 gl_test: px        RRR             GGG      
1791013657.444 gl_test: px        RRR             GGG      
1791013657.449 gl_test: px       RRRR            GGGG      
1791013657.461 gl_test: px       RRRRR           GGGGG     
1791013657.461 gl_test: px      RRRRRR          GGGGGG     
1791013657.469 gl_test: px      RRRRRRR         GGGGGGG    
1791013657.469 gl_test: px      RRRRRRR         GGGGGGG    
1791013657.469 gl_test: px     RRRRRRRR        GGGGGGGG    
1791013657.475 gl_test: px     RRRRRRRRR       GGGGGGGGG   
1791013657.482 gl_test: px    RRRRRRRRRR      GGGGGGGGGG   
1791013657.486 gl_test: px    RRRRRRRRRRR     GGGGGGGGGGG  
1791013657.486 gl_test: px    RRRRRRRRRRR     GGGGGGGGGGG  
1791013657.491 gl_test: px   RRRRRRRRRRRR    GGGGGGGGGGGG  
1791013657.498 gl_test: px                                 
1791013657.501 gl_test: px                                 
1791013657.501 gl_test: px                                 
1791013657.506 gl_test: px                                 
1791013657.511 gl_test: px                                 
1791013657.517 gl_test: px                                 
1791013657.522 gl_test: px                                 
1791013657.522 gl_test: px                                 
1791013657.544 gl_test:   draw 1 keeps orange          (16,40) = 0xff0080ff  want 0xff0080ff  ok  [check 1]
1791013657.545 gl_test:   draw 2 is cyan               (48,40) = 0xffffff00  want 0xffffff00  ok  [check 2]
1791013657.545 gl_test: case 34 unishare: PASS
1791013657.547 gl_test: px                                 
1791013657.554 gl_test: px                                 
1791013657.567 gl_test: px                                 
1791013657.567 gl_test: px                                 
1791013657.567 gl_test: px         R               B       
1791013657.572 gl_test: px         R               B       
1791013657.581 gl_test: px        RR              BB       
1791013657.581 gl_test: px        RR              BB       
1791013657.582 gl_test: px        RRR             BBB      
1791013657.590 gl_test: px        RRR             BBB      
1791013657.590 gl_test: px       RRRR            BBBB      
1791013657.595 gl_test: px       RRRR            BBBB      
1791013657.601 gl_test: px       RRRRR           BBBBB     
1791013657.607 gl_test: px       RRRRR           BBBBB     
1791013657.613 gl_test: px      RRRRRR          BBBBBB     
1791013657.613 gl_test: px      RRRRRR          BBBBBB     
1791013657.618 gl_test: px      RRRRRRR         BBBBBBB    
1791013657.618 gl_test: px      RRRRRRR         BBBBBBB    
1791013657.624 gl_test: px     RRRRRRRR        BBBBBBBB    
1791013657.629 gl_test: px     RRRRRRRR        BBBBBBBB    
1791013657.629 gl_test: px     RRRRRRRRR       BBBBBBBBB   
1791013657.634 gl_test: px     RRRRRRRRR       BBBBBBBBB   
1791013657.639 gl_test: px    RRRRRRRRRR      BBBBBBBBBB   
1791013657.653 gl_test: px    RRRRRRRRRR      BBBBBBBBBB   
1791013657.653 gl_test: px    RRRRRRRRRRR     BBBBBBBBBBB  
1791013657.654 gl_test: px    RRRRRRRRRRR     BBBBBBBBBBB  
1791013657.654 gl_test: px   RRRRRRRRRRRR    BBBBBBBBBBBB  
1791013657.656 gl_test: px   RRRRRRRRRRRR    BBBBBBBBBBBB  
1791013657.661 gl_test: px   RRRRRRRRRRRRR   BBBBBBBBBBBBB 
1791013657.667 gl_test: px                                 
1791013657.675 gl_test: px                                 
1791013657.675 gl_test: px                                 
1791013657.685 gl_test:   ccw front: left (front) == 1 red (16,40) = 0xff0000ff  want 0xff0000ff  ok  [check 1]
1791013657.696 gl_test:   ccw front: right (back) == 2 blue (48,40) = 0xffff0000  want 0xffff0000  ok  [check 2]
1791013657.703 gl_test:   ccw front: outside untouched (32, 4) = 0xff000000  want 0xff000000  ok  [check 3]
1791013657.712 gl_test:   cw front: left (back) == 2 blue (16,40) = 0xffff0000  want 0xffff0000  ok  [check 4]
1791013657.716 gl_test:   cw front: right (front) == 1 red (48,40) = 0xff0000ff  want 0xff0000ff  ok  [check 5]
1791013657.733 gl_test:   fbo ccw front: left == 1 red (16,40) = 0xff0000ff  want 0xff0000ff  ok  [check 6]
1791013657.756 gl_test:   fbo ccw front: right == 2 blue (48,40) = 0xffff0000  want 0xffff0000  ok  [check 7]
1791013657.756 gl_test: case 12 stencil2: PASS
1791013657.756 gl_test: px                                 
1791013657.756 gl_test: px                                 
1791013657.756 gl_test: px                                 
1791013657.759 gl_test: px                                 
1791013657.768 gl_test: px                 R               
1791013657.779 gl_test: px                RR               
1791013657.779 gl_test: px                RRR              
1791013657.779 gl_test: px               RRRR              
1791013657.788 gl_test: px               RRRRR             
1791013657.793 gl_test: px              RRRRRR             
1791013657.793 gl_test: px             RRRRRRRR            
1791013657.798 gl_test: px             RRRR RRRR           
1791013657.798 gl_test: px            RRRR  RRRR           
1791013657.806 gl_test: px            RRRR   RRRR          
1791013657.808 gl_test: px           RRRR    RRRR          
1791013657.813 gl_test: px           RRRR     RRRR         
1791013657.813 gl_test: px          RRRR       RRR         
1791013657.817 gl_test: px          RRR        RRRR        
1791013657.822 gl_test: px         RRRRRRRRRRRRRRRR        
1791013657.829 gl_test: px         RRRRRRRRRRRRRRRRR       
1791013657.834 gl_test: px        RRRRRRRRRRRRRRRRRR       
1791013657.834 gl_test: px        RRRRRRRRRRRRRRRRRRR      
1791013657.839 gl_test: px       RRRRRRRRRRRRRRRRRRRR      
1791013657.844 gl_test: px       RRRRRRRRRRRRRRRRRRRRR     
1791013657.852 gl_test: px      RRRRRRRRRRRRRRRRRRRRRR     
1791013657.852 gl_test: px     RRRRRRRRRRRRRRRRRRRRRRRR    
1791013657.852 gl_test: px     RRRRRRRRRRRRRRRRRRRRRRRRR   
1791013657.855 gl_test: px                                 
1791013657.864 gl_test: px                                 
1791013657.871 gl_test: px                                 
1791013657.871 gl_test: px                                 
1791013657.877 gl_test: px                                 
1791013657.883 gl_test:   winding: outer filled        (19,42) = 0xff00ffff  want 0xff00ffff  ok  [check 1]
1791013657.981 gl_test:   winding: hole empty          (32,30) = 0xff000000  want 0xff000000  ok  [check 2]
1791013657.986 gl_test:   winding: outside empty       ( 4, 4) = 0xff000000  want 0xff000000  ok  [check 3]
1791013657.992 gl_test: case 23 winding: PASS
1791013657.992 gl_test:   clear 0.5: mid quad passes   (32,32) = 0xffff0000  want 0xffff0000  ok  [check 1]
1791013657.992 gl_test:   clear 0.5: far quad fails    ( 4, 4) = 0xffff0000  want 0xffff0000  ok  [check 2]
1791013657.992 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013657.992 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013657.992 gl_test: px GGRGGRGGRGGRGGRGRRGGRGGRGGRGGRGG
1791013657.992 gl_test: px GGGGGGGGGGGGGGGRRGGGGGGGGGGGGGGG
1791013657.992 gl_test: px GGRGGRGGRGGRGGRRRRGGRGGRGGRGGRGG
1791013657.992 gl_test: px GGGGGGGGGGGGGGRRRRGGGGGGGGGGGGGG
1791013657.992 gl_test: px GGRGGRGGRGGRGGRRRRRGRGGRGGRGGRGG
1791013657.997 gl_test: px GGGGGGGGGGGGGRRRRRRGGGGGGGGGGGGG
1791013657.997 gl_test: px GGRGGRGGRGGRGRRRRRRRRGGRGGRGGRGG
1791013657.997 gl_test: px GGGGGGGGGGGGRRRRRRRRGGGGGGGGGGGG
1791013657.997 gl_test: px GGRGGRGGRGGRRRRRRRRRRGGRGGRGGRGG
1791013657.997 gl_test: px GGGGGGGGGGGRRRRRRRRRRGGGGGGGGGGG
1791013657.997 gl_test: px GGRGGRGGRGGRRRRRRRRRRRGRGGRGGRGG
1791013658.002 gl_test: px GGGGGGGGGGRRRRRRRRRRRRGGGGGGGGGG
1791013658.002 gl_test: px GGRGGRGGRGRRRRRRRRRRRRRRGGRGGRGG
1791013658.008 gl_test: px GGGGGGGGGRRRRRRRRRRRRRRGGGGGGGGG
1791013658.008 gl_test: px GGGGGGGGGRRRRRRRRRRRRRRRGGGGGGGG
1791013658.013 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013658.019 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRRGGGGGGG
1791013658.023 gl_test: px GGGGGGGRRRRRRRRRRRRRRRRRRGGGGGGG
1791013658.023 gl_test: px GGGGGGGRRRRRRRRRRRRRRRRRRRGGGGGG
1791013658.032 gl_test: px GGGGGGRRRRRRRRRRRRRRRRRRRRGGGGGG
1791013658.034 gl_test: px GGGGGGRRRRRRRRRRRRRRRRRRRRRGGGGG
1791013658.039 gl_test: px GGGGGRRRRRRRRRRRRRRRRRRRRRRGGGGG
1791013658.039 gl_test: px GGGGGRRRRRRRRRRRRRRRRRRRRRRRGGGG
1791013658.056 gl_test: px GGGGRRRRRRRRRRRRRRRRRRRRRRRRGGGG
1791013658.056 gl_test: px GGGGRRRRRRRRRRRRRRRRRRRRRRRRRGGG
1791013658.057 gl_test: px GGGRRRRRRRRRRRRRRRRRRRRRRRRRRGGG
1791013658.057 gl_test: px GGGRRRRRRRRRRRRRRRRRRRRRRRRRRRGG
1791013658.057 gl_test: px GGRRRRRRRRRRRRRRRRRRRRRRRRRRRRGG
1791013658.063 gl_test: px GGRRRRRRRRRRRRRRRRRRRRRRRRRRRRRG
1791013658.069 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013658.083 gl_test:   split: near triangle kept    (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 3]
1791013658.083 gl_test:   split: kick-1 small tri kept ( 4, 4) = 0xff00ffff  want 0xff00ffff  ok  [check 4]
1791013658.095 gl_test:   split: kick-2 small tri      (61,28) = 0xff00ffff  want 0xff00ffff  ok  [check 5]
1791013658.107 gl_test:   split: far quad elsewhere    (60,40) = 0xff00ff00  want 0xff00ff00  ok  [check 6]
1791013658.116 gl_test:   stencil kept: inside         (32,32) = 0xff00ff00  want 0xff00ff00  ok  [check 7]
1791013658.120 gl_test:   stencil kept: outside        ( 4, 4) = 0xff000000  want 0xff000000  ok  [check 8]
1791013658.135 gl_test:   depth cleared mid-frame: far quad wins (32,32) = 0xff00ff00  want 0xff00ff00  ok  [check 9]
1791013658.150 gl_test:   depth range 0..0.3: far quad wins (32,32) = 0xff00ff00  want 0xff00ff00  ok  [check 10]
1791013658.150 gl_test: case 13 zls: PASS
1791013658.165 gl_test:   client strip: quad 0         ( 6,32) = 0xff0000ff  want 0xff0000ff  ok  [check 1]
1791013658.171 gl_test:   client strip: quad 1         (16,32) = 0xff00ff00  want 0xff00ff00  ok  [check 2]
1791013658.180 gl_test:   client strip: quad 2         (26,32) = 0xffff0000  want 0xffff0000  ok  [check 3]
1791013658.191 gl_test:   client strip: quad 3         (36,32) = 0xff00ffff  want 0xff00ffff  ok  [check 4]
1791013658.194 gl_test:   client strip: quad 4         (46,32) = 0xffffff00  want 0xffffff00  ok  [check 5]
1791013658.203 gl_test:   client strip: quad 5         (56,32) = 0xffff00ff  want 0xffff00ff  ok  [check 6]
1791013658.209 gl_test:   client strip: gap stays clear (11,32) = 0xff000000  want 0xff000000  ok  [check 7]
1791013658.225 gl_test:   ibo strip: quad 0            ( 6,32) = 0xff0000ff  want 0xff0000ff  ok  [check 8]
1791013658.231 gl_test:   ibo strip: quad 1            (16,32) = 0xff00ff00  want 0xff00ff00  ok  [check 9]
1791013658.241 gl_test:   ibo strip: quad 2            (26,32) = 0xffff0000  want 0xffff0000  ok  [check 10]
1791013658.247 gl_test:   ibo strip: quad 3            (36,32) = 0xff00ffff  want 0xff00ffff  ok  [check 11]
1791013658.264 gl_test:   ibo strip: quad 4            (46,32) = 0xffffff00  want 0xffffff00  ok  [check 12]
1791013658.264 gl_test:   ibo strip: quad 5            (56,32) = 0xffff00ff  want 0xffff00ff  ok  [check 13]
1791013658.264 gl_test: case 40 qtstrip: PASS
1791013658.281 gl_test:   constant attribs: cyan       (32,32) = 0xffffff00  want 0xffffff00  ok  [check 1]
1791013658.287 gl_test:   current attrib 1.g           = 1  want 1  ok  [check 2]
1791013658.297 gl_test:   current attrib 1.r           = 0  want 0  ok  [check 3]
1791013658.298 gl_test: 8x8 texture @0x8001f3a000: ff0000ff ff0000ff ff0000ff ff0000ff | row1 ff0000ff ff0000ff
1791013658.306 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013658.311 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013658.319 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013658.323 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013658.323 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013658.329 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013658.333 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013658.333 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013658.340 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013658.344 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013658.349 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013658.349 gl_test: px RRRRRRRRRRRRRRRRWWWWRRRRRRRRRRRR
1791013658.354 gl_test: px RRRRRRRRRRRRRRRRWWWWWWWWWWWWWWWW
1791013658.370 gl_test: px RRRRRRRRRRRRRRRRWWWWWWWWWWWWWWWW
1791013658.370 gl_test: px RRRRRRRRRRRRRRRRWWWWWWWWWWWWWWWW
1791013658.370 gl_test: px RRRRRRRRRRRRRRRRWWWWWWWWWWWWWWWW
1791013658.370 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013658.370 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013658.379 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013658.384 gl_test: px RRRRRRRRRRRRRRRRGGGGGGGGGGGGGGGG
1791013658.389 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013658.389 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013658.395 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013658.406 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013658.406 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013658.406 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013658.410 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013658.414 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013658.423 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013658.426 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013658.426 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013658.432 gl_test: px WWWWWWWWWWWWRRRRGGGGGGGGGGGGGGGG
1791013658.441 gl_test:   sub-image rgba: magenta      (16,16) = 0xffff00ff  want 0xffff00ff  ok  [check 4]
1791013658.447 gl_test:   sub-image rgb align 1: yellow (52,12) = 0xff00ffff  want 0xff00ffff  ok  [check 5]
1791013658.456 gl_test:   sub-image rgb align 4: white (12,52) = 0xffffffff  want 0xffffffff  ok  [check 6]
1791013658.466 gl_test:   untouched quadrant: green    (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 7]
1791013658.482 gl_test:   copied: magenta              (16,16) = 0xffff00ff  want 0xffff00ff  ok  [check 8]
1791013658.485 gl_test:   copied: yellow               (52,12) = 0xff00ffff  want 0xff00ffff  ok  [check 9]
1791013658.495 gl_test:   copied: green                (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 10]
1791013658.514 gl_test:   sub-copied: white patch      (48,16) = 0xffffffff  want 0xffffffff  ok  [check 11]
1791013658.515 gl_test:   sub-copied: red              (60, 4) = 0xff0000ff  want 0xff0000ff  ok  [check 12]
1791013658.521 gl_test:   depth range far              = 1  want 1  ok  [check 13]
1791013658.527 gl_test:   depth writemask              = 1  want 1  ok  [check 14]
1791013658.537 gl_test:   stencil writemask (all ones) = 4294967295  want 4294967295  ok  [check 15]
1791013658.543 gl_test:   glIsTexture(tex2)            = 1  want 1  ok  [check 16]
1791013658.548 gl_test:   glIsTexture(999)             = 0  want 0  ok  [check 17]
1791013658.553 gl_test:   glIsProgram(prog)            = 1  want 1  ok  [check 18]
1791013658.558 gl_test:   glIsShader(0)                = 0  want 0  ok  [check 19]
1791013658.569 gl_test:   buffer size                  = 144  want 144  ok  [check 20]
1791013658.569 gl_test:   attached shaders             = 2  want 2  ok  [check 21]
1791013658.575 gl_test:   attached vs is a shader      = 1  want 1  ok  [check 22]
1791013658.587 gl_test:   shader source prefix         = 35  want 35  ok  [check 23]
1791013658.595 gl_test:   shader source chunk          = 7  want 7  ok  [check 24]
1791013658.598 gl_test:   tex mag filter               = 9728  want 9728  ok  [check 25]
1791013658.602 gl_test:   highp float precision        = 23  want 23  ok  [check 26]
1791013658.602 gl_test: case 14 api: PASS
1791013658.614 I: yakogl: program 48 has no uniform 'u_tex0'
1791013658.623 gl_test:   unused sampler u_tex0 optimized out (loc -1) = -1  want -1  ok  [check 1]
1791013658.624 gl_test:   used sampler u_tex1 has a location = 1  want 1  ok  [check 2]
1791013658.637 gl_test:   compacted sampler reads unit 1 (green) (32,32) = 0xff00ff00  want 0xff00ff00  ok  [check 3]
1791013658.637 gl_test: case 24 compact: PASS
1791013658.649 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013658.654 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013658.654 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013658.660 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013658.660 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013658.671 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013658.671 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013658.672 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013658.677 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013658.683 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013658.689 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013658.689 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013658.695 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013658.708 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013658.708 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013658.709 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013658.709 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013658.714 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013658.719 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013658.719 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013658.725 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013658.732 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013658.735 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013658.739 gl_test: px BBBBBBBBRRRRRRRRRRRRRRRRBBBBBBBB
1791013658.739 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013658.745 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013658.750 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013658.755 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013658.755 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013658.761 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013658.773 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013658.775 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013658.781 gl_test:   inside the box               (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 1]
1791013658.787 gl_test:   inside, at a corner          (17,17) = 0xff0000ff  want 0xff0000ff  ok  [check 2]
1791013658.792 gl_test:   left of the box              ( 8,32) = 0xffff0000  want 0xffff0000  ok  [check 3]
1791013658.801 gl_test:   right of the box             (56,32) = 0xffff0000  want 0xffff0000  ok  [check 4]
1791013658.813 gl_test:   one row above the box        (32,15) = 0xffff0000  want 0xffff0000  ok  [check 5]
1791013658.822 gl_test:   one row below the box        (32,48) = 0xffff0000  want 0xffff0000  ok  [check 6]
1791013658.822 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013658.829 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013658.840 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013658.840 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013658.844 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013658.844 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013658.849 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013658.855 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013658.859 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013658.859 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013658.865 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013658.875 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013658.876 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013658.880 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013658.880 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013658.885 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013658.891 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013658.897 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013658.902 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013658.902 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013658.902 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013658.907 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013658.913 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013658.918 gl_test: px GGGGGGGGRRRRRRRRRRRRRRRRGGGGGGGG
1791013658.918 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013658.933 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013658.933 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013658.933 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013658.934 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013658.940 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013658.949 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013658.949 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013658.961 gl_test:   the clear is after the draw  (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 7]
1791013658.965 gl_test:   the draw is everywhere else  ( 8,32) = 0xff00ff00  want 0xff00ff00  ok  [check 8]
1791013658.978 gl_test:   an empty box clears nothing  (32,32) = 0xffff0000  want 0xffff0000  ok  [check 9]
1791013658.979 gl_test: case 58 clearscis: PASS
1791013658.995 gl_test:   a fully masked colour clear does nothing (32,32) = 0xffff0000  want 0xffff0000  ok  [check 1]
1791013659.000 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.005 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.012 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.015 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.015 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.021 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.025 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.031 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.031 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.035 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.045 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.046 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.049 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.060 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.060 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.064 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.064 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.069 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.073 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.073 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.083 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.088 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.093 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.093 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.099 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.106 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.106 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.110 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.115 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.115 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.119 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.126 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.135 gl_test:   a masked depth clear does nothing (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 2]
1791013659.146 gl_test:   the masked bits were not cleared (32,32) = 0xffff0000  want 0xffff0000  ok  [check 3]
1791013659.151 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.158 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.166 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.166 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.169 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.174 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.183 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.183 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.183 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.189 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.194 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.199 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.199 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.212 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.212 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.217 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.217 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.222 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.226 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.226 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.231 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.237 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.243 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.247 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.253 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.257 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.257 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.262 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.262 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.277 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.278 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.278 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.286 gl_test:   the allowed bits were cleared (32,32) = 0xff00ff00  want 0xff00ff00  ok  [check 4]
1791013659.286 gl_test: case 59 clearmask: PASS
1791013659.289 gl_test: wrap S = repeat:
1791013659.295 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.300 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.307 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.312 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.312 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.317 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.323 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.323 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.333 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.333 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.339 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.344 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.344 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.349 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.354 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.354 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.359 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.364 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.369 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.374 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.396 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.397 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.397 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.397 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.397 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.397 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.397 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.405 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.409 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.417 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.424 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.424 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRGGGGGGGG
1791013659.428 gl_test:   S first period               ( 8,48) = 0xff0000ff  want 0xff0000ff  ok  [check 1]
1791013659.438 gl_test:   S second period, u 1.25      (40,48) = 0xff0000ff  want 0xff0000ff  ok  [check 2]
1791013659.447 gl_test:   S second period, u 1.75      (56,48) = 0xff00ff00  want 0xff00ff00  ok  [check 3]
1791013659.453 gl_test: wrap S = mirror:
1791013659.453 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.459 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.465 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.469 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.473 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.473 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.478 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.496 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.497 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.497 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.497 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.497 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.503 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.509 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.513 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.513 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.519 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.526 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.531 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.531 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.536 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.541 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.541 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.547 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.551 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.557 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.557 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.562 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.567 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.573 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.578 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.583 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGRRRRRRRR
1791013659.592 gl_test:   S first period               ( 8,48) = 0xff0000ff  want 0xff0000ff  ok  [check 4]
1791013659.593 gl_test:   S second period, u 1.25      (40,48) = 0xff00ff00  want 0xff00ff00  ok  [check 5]
1791013659.601 gl_test:   S second period, u 1.75      (56,48) = 0xff0000ff  want 0xff0000ff  ok  [check 6]
1791013659.613 gl_test: wrap S = clamp:
1791013659.613 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.617 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.623 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.628 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.628 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.633 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.639 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.644 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.644 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.648 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.653 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.658 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.664 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.664 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.669 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.675 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.679 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.679 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.683 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.687 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.695 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.696 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.708 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.709 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.710 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.710 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.721 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.722 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.725 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.732 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.738 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.738 gl_test: px RRRRRRRRGGGGGGGGGGGGGGGGGGGGGGGG
1791013659.743 gl_test:   S first period               ( 8,48) = 0xff0000ff  want 0xff0000ff  ok  [check 7]
1791013659.755 gl_test:   S second period, u 1.25      (40,48) = 0xff00ff00  want 0xff00ff00  ok  [check 8]
1791013659.759 gl_test:   S second period, u 1.75      (56,48) = 0xff00ff00  want 0xff00ff00  ok  [check 9]
1791013659.765 gl_test: wrap T = repeat:
1791013659.769 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.775 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.821 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.826 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.873 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.873 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.873 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.873 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.873 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.873 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.873 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.873 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.873 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.873 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.873 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.873 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.879 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.879 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.885 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.885 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.885 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.885 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.885 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.885 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.885 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.885 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.885 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.885 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.885 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.891 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.891 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.899 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.902 gl_test:   T first period               ( 8,55) = 0xff0000ff  want 0xff0000ff  ok  [check 10]
1791013659.912 gl_test:   T second period, v 1.25      ( 8,23) = 0xff0000ff  want 0xff0000ff  ok  [check 11]
1791013659.916 gl_test:   T second period, v 1.75      ( 8, 7) = 0xffff0000  want 0xffff0000  ok  [check 12]
1791013659.925 gl_test: wrap T = mirror:
1791013659.928 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.933 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.937 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.947 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.947 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.947 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.953 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.959 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013659.963 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.963 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.968 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.973 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.979 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.979 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.989 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013659.989 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.002 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.002 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.002 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.002 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.006 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.011 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.034 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.034 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.038 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.038 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.038 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.047 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.052 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.052 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.052 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.059 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.060 gl_test:   T first period               ( 8,55) = 0xff0000ff  want 0xff0000ff  ok  [check 13]
1791013660.069 gl_test:   T second period, v 1.25      ( 8,23) = 0xffff0000  want 0xffff0000  ok  [check 14]
1791013660.074 gl_test:   T second period, v 1.75      ( 8, 7) = 0xff0000ff  want 0xff0000ff  ok  [check 15]
1791013660.085 gl_test: wrap T = clamp:
1791013660.085 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.090 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.095 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.104 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.104 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.110 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.110 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.115 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.121 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.126 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.132 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.132 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.143 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.143 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.143 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.144 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.149 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.157 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.161 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.172 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.172 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.172 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.175 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.175 gl_test: px BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB
1791013660.185 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.190 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.194 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.199 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.199 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.207 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.210 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.215 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.221 gl_test:   T first period               ( 8,55) = 0xff0000ff  want 0xff0000ff  ok  [check 16]
1791013660.227 gl_test:   T second period, v 1.25      ( 8,23) = 0xffff0000  want 0xffff0000  ok  [check 17]
1791013660.236 gl_test:   T second period, v 1.75      ( 8, 7) = 0xffff0000  want 0xffff0000  ok  [check 18]
1791013660.236 gl_test: case 60 wrapmode: PASS
1791013660.252 gl_test:   complete: the texel          (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 1]
1791013660.269 I: a texture on unit 0 is incomplete and samples (0,0,0,1): a mipmap min filter, but fewer levels uploaded than the chain needs (target 0xde1, 2x2, levels 1 of 2, min filter 0x2703, wrap 0x2901/0x2901)
1791013660.275 gl_test: px                                 
1791013660.284 gl_test: px                                 
1791013660.286 gl_test: px                                 
1791013660.290 gl_test: px                                 
1791013660.295 gl_test: px                                 
1791013660.295 gl_test: px                                 
1791013660.299 gl_test: px                                 
1791013660.309 gl_test: px                                 
1791013660.309 gl_test: px                                 
1791013660.310 gl_test: px                                 
1791013660.315 gl_test: px                                 
1791013660.319 gl_test: px                                 
1791013660.327 gl_test: px                                 
1791013660.331 gl_test: px                                 
1791013660.331 gl_test: px                                 
1791013660.336 gl_test: px                                 
1791013660.336 gl_test: px                                 
1791013660.347 gl_test: px                                 
1791013660.347 gl_test: px                                 
1791013660.351 gl_test: px                                 
1791013660.360 gl_test: px                                 
1791013660.361 gl_test: px                                 
1791013660.361 gl_test: px                                 
1791013660.368 gl_test: px                                 
1791013660.372 gl_test: px                                 
1791013660.376 gl_test: px                                 
1791013660.376 gl_test: px                                 
1791013660.386 gl_test: px                                 
1791013660.386 gl_test: px                                 
1791013660.397 gl_test: px                                 
1791013660.398 gl_test: px                                 
1791013660.398 gl_test: px                                 
1791013660.412 gl_test:   incomplete: opaque black     (16,48) = 0xff000000  want 0xff000000  ok  [check 2]
1791013660.413 gl_test:   incomplete: opaque black     (48,16) = 0xff000000  want 0xff000000  ok  [check 3]
1791013660.423 gl_test:   complete again               (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 4]
1791013660.440 gl_test:   nothing bound: opaque black  (16,48) = 0xff000000  want 0xff000000  ok  [check 5]
1791013660.457 gl_test:   nothing bound: opaque black  (48,16) = 0xff000000  want 0xff000000  ok  [check 6]
1791013660.457 gl_test: case 61 texincomp: PASS
1791013660.457 I: osal(zephyr): ready (regs=0xfd80000)
1791013660.457 gl_test:   eglInitialize                = 1  want 1  ok  [check 1]
1791013660.468 gl_test:   eglChooseConfig              = 1  want 1  ok  [check 2]
1791013660.469 gl_test:   one config                   = 1  want 1  ok  [check 3]
1791013660.475 gl_test:   no 8x MSAA config            = 1  want 1  ok  [check 4]
1791013660.485 gl_test:   config depth size            = 24  want 24  ok  [check 5]
1791013660.491 gl_test:   eglQueryString VENDOR        = 1  want 1  ok  [check 6]
1791013660.495 gl_test:   pbuffer created              = 1  want 1  ok  [check 7]
1791013660.500 gl_test:   context created              = 1  want 1  ok  [check 8]
1791013660.517 gl_test:   eglMakeCurrent               = 1  want 1  ok  [check 9]
1791013660.521 gl_test:   current context              = 1  want 1  ok  [check 10]
1791013660.532 gl_test:   pbuffer width                = 64  want 64  ok  [check 11]
1791013660.539 gl_test:   eglGetProcAddress            = 1  want 1  ok  [check 12]
1791013660.545 gl_test:   eglSwapBuffers               = 1  want 1  ok  [check 13]
1791013660.549 gl_test: px                                 
1791013660.549 gl_test: px                                 
1791013660.554 gl_test: px                                 
1791013660.559 gl_test: px                                 
1791013660.564 gl_test: px                                 
1791013660.570 gl_test: px                 R               
1791013660.577 gl_test: px                RR               
1791013660.577 gl_test: px                RRR              
1791013660.577 gl_test: px               RRRR              
1791013660.581 gl_test: px               RRRRR             
1791013660.585 gl_test: px              RRRRRR             
1791013660.591 gl_test: px              RRRRRRR            
1791013660.596 gl_test: px             RRRRRRRR            
1791013660.603 gl_test: px             RRRRRRRRR           
1791013660.608 gl_test: px            RRRRRRRRRR           
1791013660.608 gl_test: px            RRRRRRRRRRR          
1791013660.623 gl_test: px           GRRRRRRRRRRR          
1791013660.623 gl_test: px           GGRRRRRRRRBBB         
1791013660.623 gl_test: px          GGGGGRRRRRBBBB         
1791013660.623 gl_test: px          GGGGGGRRBBBBBBB        
1791013660.623 gl_test: px         GGGGGGGGBBBBBBBB        
1791013660.629 gl_test: px         GGGGGGGGBBBBBBBBB       
1791013660.634 gl_test: px        GGGGGGGGGBBBBBBBBB       
1791013660.639 gl_test: px        GGGGGGGGGBBBBBBBBBB      
1791013660.644 gl_test: px       GGGGGGGGGGBBBBBBBBBB      
1791013660.649 gl_test: px       GGGGGGGGGGBBBBBBBBBBB     
1791013660.649 gl_test: px      GGGGGGGGGGGBBBBBBBBBBB     
1791013660.655 gl_test: px      GGGGGGGGGGGBBBBBBBBBBBB    
1791013660.661 gl_test: px                                 
1791013660.665 gl_test: px                                 
1791013660.665 gl_test: px                                 
1791013660.669 gl_test: px                                 
1791013660.685 gl_test:   egl pbuffer: centroid        (32,40) = 0xff595452  want 0xff595452  ok  [check 14]
1791013660.689 gl_test:   egl pbuffer: background      ( 1, 1) = 0xff000000  want 0xff000000  ok  [check 15]
1791013660.693 gl_test:   release                      = 1  want 1  ok  [check 16]
1791013660.701 gl_test:   no current context           = 1  want 1  ok  [check 17]
1791013660.710 gl_test:   eglDestroySurface            = 1  want 1  ok  [check 18]
1791013660.719 gl_test:   eglDestroyContext            = 1  want 1  ok  [check 19]
1791013660.720 gl_test:   eglTerminate                 = 1  want 1  ok  [check 20]
1791013660.725 gl_test:   eglGetError clean            = 12288  want 12288  ok  [check 21]
1791013660.737 gl_test: case 15 egl: PASS
1791013660.737 I: osal(zephyr): ready (regs=0xfd80000)
1791013660.737 gl_test:   eglChooseConfig samples 4    = 1  want 1  ok  [check 1]
1791013660.739 gl_test:   config EGL_SAMPLES           = 4  want 4  ok  [check 2]
1791013660.757 gl_test:   GL_SAMPLES x4                = 4  want 4  ok  [check 3]
1791013660.777 I: render: hc dwords: 00000000 003f003f eaa26e26 00000000 2a62aee6 00000000 00000000 00000200 00003003 00000004 00000009 00080000 00080000 00000000 00000000 00000000
1791013660.784 I: render: hrt[0]: 00014000 000000fa 00000000 00000000 00000000 00000000 00000000 00000000
1791013660.794 I: render: hrt[8]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013660.796 I: render: hrt[16]: 00000000 c00ea000 00000000 00000000 c00d8da8 c00d8d28 c00d8d68 00000000
1791013660.807 I: render: hrt[24]: 00000000 00000000 01f84000 00000080 00000000 00000000 00000000 00000000
1791013660.821 I: render: hrt[32]: 00000000 00000000 00000000 00000000 01f80000 00000080 01f96000 00000080
1791013660.822 I: render: hrt[40]: 01f86000 00000080 00000000 00000000 00000000 00000000 00000000 00000000
1791013660.835 I: render: hrt[48]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013660.848 I: render: hrt[56]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013660.850 I: render: HWRT 64x64 x4 te_screen=0x00003003 mtile1=0x00080000 mtile2=0x00080000 isp_mtile=0x00020002 stride=4 rgn=64 tpc=512 teaa=9
1791013660.860 gl_test: px                                 
1791013660.868 gl_test: px                                R
1791013660.868 gl_test: px                               RR
1791013660.873 gl_test: px                              RRR
1791013660.879 gl_test: px                             RRRR
1791013660.879 gl_test: px                            RRRRR
1791013660.883 gl_test: px                           RRRRRR
1791013660.888 gl_test: px                          RRRRRRR
1791013660.893 gl_test: px                         RRRRRRRR
1791013660.975 gl_test: px                        RRRRRRRRR
1791013660.975 gl_test: px                       RRRRRRRRRR
1791013660.981 gl_test: px                      RRRRRRRRRRR
1791013660.988 gl_test: px                     RRRRRRRRRRRR
1791013660.988 gl_test: px                    RRRRRRRRRRRRR
1791013660.988 gl_test: px                   RRRRRRRRRRRRRR
1791013660.988 gl_test: px                  RRRRRRRRRRRRRRR
1791013660.988 gl_test: px                 RRRRRRRRRRRRRRRR
1791013660.988 gl_test: px                RRRRRRRRRRRRRRRRR
1791013660.988 gl_test: px               RRRRRRRRRRRRRRRRRR
1791013660.988 gl_test: px              RRRRRRRRRRRRRRRRRRR
1791013660.988 gl_test: px             RRRRRRRRRRRRRRRRRRRR
1791013660.988 gl_test: px            RRRRRRRRRRRRRRRRRRRRR
1791013660.993 gl_test: px           RRRRRRRRRRRRRRRRRRRRRR
1791013660.993 gl_test: px          RRRRRRRRRRRRRRRRRRRRRRR
1791013660.993 gl_test: px         RRRRRRRRRRRRRRRRRRRRRRRR
1791013660.993 gl_test: px        RRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.993 gl_test: px       RRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.993 gl_test: px      RRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.993 gl_test: px     RRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.993 gl_test: px    RRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.993 gl_test: px   RRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.993 gl_test: px  RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013660.993 gl_test:   x4: inside 0xffff00ff outside 0xff000000 edge 0xff7f007f
1791013660.998 gl_test:   x4 inside                    (40,43) = 0xffff00ff  want 0xffff00ff  ok  [check 4]
1791013661.011 gl_test:   x4 outside                   (20,23) = 0xff000000  want 0xff000000  ok  [check 5]
1791013661.012 gl_test:   x4 edge: half covered        (32,31) = 0xff7f007f  want 0xff800080  ok  [check 6]
1791013661.015 gl_test:   eglChooseConfig default      = 1  want 1  ok  [check 7]
1791013661.026 gl_test:   default config is single-sampled = 0  want 0  ok  [check 8]
1791013661.041 gl_test:   GL_SAMPLES x1                = 0  want 0  ok  [check 9]
1791013661.053 gl_test: px                                 
1791013661.058 gl_test: px                                R
1791013661.058 gl_test: px                               RR
1791013661.058 gl_test: px                              RRR
1791013661.062 gl_test: px                             RRRR
1791013661.074 gl_test: px                            RRRRR
1791013661.075 gl_test: px                           RRRRRR
1791013661.075 gl_test: px                          RRRRRRR
1791013661.078 gl_test: px                         RRRRRRRR
1791013661.083 gl_test: px                        RRRRRRRRR
1791013661.089 gl_test: px                       RRRRRRRRRR
1791013661.095 gl_test: px                      RRRRRRRRRRR
1791013661.100 gl_test: px                     RRRRRRRRRRRR
1791013661.100 gl_test: px                    RRRRRRRRRRRRR
1791013661.105 gl_test: px                   RRRRRRRRRRRRRR
1791013661.111 gl_test: px                  RRRRRRRRRRRRRRR
1791013661.120 gl_test: px                 RRRRRRRRRRRRRRRR
1791013661.121 gl_test: px                RRRRRRRRRRRRRRRRR
1791013661.121 gl_test: px               RRRRRRRRRRRRRRRRRR
1791013661.129 gl_test: px              RRRRRRRRRRRRRRRRRRR
1791013661.130 gl_test: px             RRRRRRRRRRRRRRRRRRRR
1791013661.130 gl_test: px            RRRRRRRRRRRRRRRRRRRRR
1791013661.135 gl_test: px           RRRRRRRRRRRRRRRRRRRRRR
1791013661.141 gl_test: px          RRRRRRRRRRRRRRRRRRRRRRR
1791013661.145 gl_test: px         RRRRRRRRRRRRRRRRRRRRRRRR
1791013661.150 gl_test: px        RRRRRRRRRRRRRRRRRRRRRRRRR
1791013661.150 gl_test: px       RRRRRRRRRRRRRRRRRRRRRRRRRR
1791013661.157 gl_test: px      RRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013661.167 gl_test: px     RRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013661.167 gl_test: px    RRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013661.167 gl_test: px   RRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013661.173 gl_test: px  RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013661.178 gl_test:   x1: inside 0xffff00ff outside 0xff000000 edge 0xffff00ff
1791013661.187 gl_test:   x1 inside                    (40,43) = 0xffff00ff  want 0xffff00ff  ok  [check 10]
1791013661.199 gl_test:   x1 outside                   (20,23) = 0xff000000  want 0xff000000  ok  [check 11]
1791013661.204 gl_test:   x1 edge: full or empty       = 1  want 1  ok  [check 12]
1791013661.204 gl_test: case 16 msaa: PASS
1791013661.207 gl_test:   HelloAPI program links       = 1  want 1  ok  [check 1]
1791013661.212 gl_test:   transformationMatrix found   = 1  want 1  ok  [check 2]
1791013661.236 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.236 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.236 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.236 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.236 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.243 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.249 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.255 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.255 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.261 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.265 gl_test: px GGGGGGGGGGGGGGGGRGGGGGGGGGGGGGGG
1791013661.265 gl_test: px GGGGGGGGGGGGGGGRRGGGGGGGGGGGGGGG
1791013661.269 gl_test: px GGGGGGGGGGGGGGGRRRGGGGGGGGGGGGGG
1791013661.274 gl_test: px GGGGGGGGGGGGGGRRRRGGGGGGGGGGGGGG
1791013661.281 gl_test: px GGGGGGGGGGGGGGRRRRRGGGGGGGGGGGGG
1791013661.288 gl_test: px GGGGGGGGGGGGGRRRRRRGGGGGGGGGGGGG
1791013661.288 gl_test: px GGGGGGGGGGGGGRRRRRRRGGGGGGGGGGGG
1791013661.291 gl_test: px GGGGGGGGGGGGRRRRRRRRGGGGGGGGGGGG
1791013661.299 gl_test: px GGGGGGGGGGGGRRRRRRRRRGGGGGGGGGGG
1791013661.299 gl_test: px GGGGGGGGGGGRRRRRRRRRRGGGGGGGGGGG
1791013661.305 gl_test: px GGGGGGGGGGGRRRRRRRRRRRGGGGGGGGGG
1791013661.310 gl_test: px GGGGGGGGGGRRRRRRRRRRRRGGGGGGGGGG
1791013661.310 gl_test: px GGGGGGGGGGRRRRRRRRRRRRRGGGGGGGGG
1791013661.315 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.321 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.326 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.340 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.340 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.341 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.341 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.341 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.344 gl_test: px GGGGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
1791013661.360 gl_test:   HelloAPI: triangle (1, 1, 0.66) (32,36) = 0xffa8ffff  want 0xffa8ffff  ok  [check 3]
1791013661.365 gl_test:   HelloAPI: clear (0, 0.7, 0.67) ( 4, 4) = 0xffabb300  want 0xffabb300  ok  [check 4]
1791013661.371 gl_test:   attribute locations by name  = 1  want 1  ok  [check 5]
1791013661.376 gl_test:   uniform locations by name    = 1  want 1  ok  [check 6]
1791013661.382 gl_test: 8x8 texture @0x8001f4f000: ff0000ff ff0000ff ff0000ff ff0000ff | row1 ff0000ff ff0000ff
1791013661.400 gl_test:   es100 textured: top-left blue (16,16) = 0xffff0000  want 0xffff0000  ok  [check 7]
1791013661.403 gl_test:   es100 textured: top-right white (48,16) = 0xffffffff  want 0xffffffff  ok  [check 8]
1791013661.414 gl_test:   es100 textured: bottom-left red (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 9]
1791013661.436 gl_test:   es100 tint+scale: top-right half green (48,16) = 0xff008000  want 0xff008000  ok  [check 10]
1791013661.436 gl_test:   es100 tint+scale: bottom-left black (16,48) = 0xff000000  want 0xff000000  ok  [check 11]
1791013661.436 gl_test: case 19 es100: PASS
1791013661.445 gl_test:   3 compressed formats         = 3  want 3  ok  [check 1]
1791013661.452 gl_test:   ETC1 listed                  = 36196  want 36196  ok  [check 2]
1791013661.453 gl_test:   GL_OES_compressed_ETC1_RGB8_texture advertised = 1  want 1  ok  [check 3]
1791013661.473 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013661.474 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013661.474 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013661.474 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013661.477 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013661.487 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013661.491 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013661.497 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013661.497 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013661.502 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013661.507 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013661.513 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013661.513 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013661.523 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013661.524 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013661.524 gl_test: px RRRRRRRRGGGGGGGGRRRRRRRRRRRRRRRR
1791013661.528 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013661.538 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013661.541 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013661.542 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013661.554 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013661.554 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013661.554 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013661.557 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013661.562 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013661.568 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013661.568 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013661.573 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013661.578 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013661.578 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013661.583 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013661.589 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBWWWWWWWW
1791013661.600 gl_test:   etc1 block (0,0)             ( 8,48) = 0xff0202ff  want 0xff0202ff  ok  [check 4]
1791013661.605 gl_test:   etc1 block (1,0)             (24,48) = 0xff02ff02  want 0xff02ff02  ok  [check 5]
1791013661.616 gl_test:   etc1 block (2,0)             (40,48) = 0xffff0202  want 0xffff0202  ok  [check 6]
1791013661.621 gl_test:   etc1 block (3,0)             (56,48) = 0xffffffff  want 0xffffffff  ok  [check 7]
1791013661.643 gl_test:   etc1 block (0,1)             ( 8,16) = 0xff02ffff  want 0xff02ffff  ok  [check 8]
1791013661.643 gl_test:   etc1 block (1,1)             (24,16) = 0xffffff02  want 0xffffff02  ok  [check 9]
1791013661.643 gl_test:   etc1 block (2,1)             (40,16) = 0xffff02ff  want 0xffff02ff  ok  [check 10]
1791013661.649 gl_test:   etc1 block (3,1)             (56,16) = 0xff8a8a8a  want 0xff8a8a8a  ok  [check 11]
1791013661.663 gl_test:   etc2 rgba block 0            (16,48) = 0xff0202ff  want 0xff0202ff  ok  [check 12]
1791013661.675 gl_test:   etc2 rgba block 1            (48,48) = 0x8002ff02  want 0x8002ff02  ok  [check 13]
1791013661.687 gl_test:   etc2 rgba block 2            (16,16) = 0x40ff0202  want 0x40ff0202  ok  [check 14]
1791013661.697 gl_test:   etc2 rgba block 3            (48,16) = 0x00ffffff  want 0x00ffffff  ok  [check 15]
1791013661.697 I: GL error 0x501 in glCompressedTexImage2D
1791013661.703 gl_test:   wrong imageSize -> INVALID_VALUE = 1281  want 1281  ok  [check 16]
1791013661.708 I: GL error 0x500 in glCompressedTexImage2D
1791013661.712 gl_test:   PVRTC -> INVALID_ENUM        = 1280  want 1280  ok  [check 17]
1791013661.712 gl_test: case 20 etc: PASS
1791013661.722 I:   cube-bind: 0x8001f5d000 unit 0 kick 104 (draw)
1791013661.727 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013661.753 gl_test:   face 0 centre                (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 1]
1791013661.753 gl_test:   face 0 off-centre            (16,16) = 0xff0000ff  want 0xff0000ff  ok  [check 2]
1791013661.753 I:   cube-bind: 0x8001f5d000 unit 0 kick 105 (draw)
1791013661.754 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013661.771 gl_test:   face 1 centre                (32,32) = 0xff00ff00  want 0xff00ff00  ok  [check 3]
1791013661.779 gl_test:   face 1 off-centre            (16,16) = 0xff00ff00  want 0xff00ff00  ok  [check 4]
1791013661.783 I:   cube-bind: 0x8001f5d000 unit 0 kick 106 (draw)
1791013661.789 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013661.799 gl_test:   face 2 centre                (32,32) = 0xffff0000  want 0xffff0000  ok  [check 5]
1791013661.811 gl_test:   face 2 off-centre            (16,16) = 0xffff0000  want 0xffff0000  ok  [check 6]
1791013661.816 I:   cube-bind: 0x8001f5d000 unit 0 kick 107 (draw)
1791013661.820 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013661.845 gl_test:   face 3 centre                (32,32) = 0xff00ffff  want 0xff00ffff  ok  [check 7]
1791013661.846 gl_test:   face 3 off-centre            (16,16) = 0xff00ffff  want 0xff00ffff  ok  [check 8]
1791013661.846 I:   cube-bind: 0x8001f5d000 unit 0 kick 108 (draw)
1791013661.848 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013661.862 gl_test:   face 4 centre                (32,32) = 0xffff00ff  want 0xffff00ff  ok  [check 9]
1791013661.873 gl_test:   face 4 off-centre            (16,16) = 0xffff00ff  want 0xffff00ff  ok  [check 10]
1791013661.881 I:   cube-bind: 0x8001f5d000 unit 0 kick 109 (draw)
1791013661.882 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013661.893 gl_test:   face 5 centre                (32,32) = 0xffffff00  want 0xffffff00  ok  [check 11]
1791013661.905 gl_test:   face 5 off-centre            (16,16) = 0xffffff00  want 0xffffff00  ok  [check 12]
1791013661.916 I: render: hc dwords: 00000000 00070007 00000088 00000000 00000088 00000000 00000000 00000080 00001001 00000004 00000000 00080000 00080000 00000000 00000000 00000000
1791013661.926 I: render: hrt[0]: 00018000 000000fa 00000000 00000000 00000000 00000000 00000000 00000000
1791013661.932 I: render: hrt[8]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013661.947 I: render: hrt[16]: 00000000 c00ea180 00000000 00000000 c00d8da8 c00d8d28 c00d8d68 00000000
1791013661.948 I: render: hrt[24]: 00000000 00000000 01f9b000 00000080 00000000 00000000 00000000 00000000
1791013661.955 I: render: hrt[32]: 00000000 00000000 00000000 00000000 01f97000 00000080 01fad000 00000080
1791013661.974 I: render: hrt[40]: 01f9d000 00000080 00000000 00000000 00000000 00000000 00000000 00000000
1791013661.975 I: render: hrt[48]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013661.978 I: render: hrt[56]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013661.994 I: render: HWRT 8x8 x1 te_screen=0x00001001 mtile1=0x00080000 mtile2=0x00080000 isp_mtile=0x00020002 stride=4 rgn=16 tpc=128 teaa=0
1791013662.002 I:   cube-bind: 0x8001f5d000 unit 0 kick 111 (draw)
1791013662.007 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013662.019 gl_test:   after FBO: face 0            (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 13]
1791013662.023 I:   cube-bind: 0x8001f5d000 unit 0 kick 112 (draw)
1791013662.023 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013662.075 gl_test:   after FBO: face 2            (32,32) = 0xffffffff  want 0xffffffff  ok  [check 14]
1791013662.102 I:   cube-bind: 0x8001f5d000 unit 0 kick 113 (draw)
1791013662.102 I:   cube(draw) @0x8001f5d000 8x8 x6: fmt 0, cpu 0x90a0c000 - not dumped
1791013662.102 gl_test:   after FBO: face 4            (32,32) = 0xffff00ff  want 0xffff00ff  ok  [check 15]
1791013662.102 gl_test: case 41 es3cube: PASS
1791013662.102 I:   cube-bind: 0x8001f5f000 unit 0 kick 114 (draw)
1791013662.102 I:   cube(draw) @0x8001f5f000 8x8 x3: fmt 0, cpu 0x90a0e000 - not dumped
1791013662.103 gl_test:   layer 0 (rgb texture, a texelFetch) (32,32) = 0x6400ff00  want 0x6400ff00  ok  [check 1]
1791013662.103 I:   cube-bind: 0x8001f5f000 unit 0 kick 115 (draw)
1791013662.103 I:   cube(draw) @0x8001f5f000 8x8 x3: fmt 0, cpu 0x90a0e000 - not dumped
1791013662.109 gl_test:   layer 1 (rgb texture, a texelFetch) (32,32) = 0x9628af50  want 0x9628af50  ok  [check 2]
1791013662.119 I:   cube-bind: 0x8001f5f000 unit 0 kick 116 (draw)
1791013662.124 I:   cube(draw) @0x8001f5f000 8x8 x3: fmt 0, cpu 0x90a0e000 - not dumped
1791013662.134 gl_test:   layer 2 (rgb texture, a texelFetch) (32,32) = 0xc8505fa0  want 0xc8505fa0  ok  [check 3]
1791013662.152 I: render: hc dwords: 00000000 00070007 00000088 00000000 00000088 00000000 00000000 00000080 00001001 00000004 00000000 00080000 00080000 00000000 00000000 00000000
1791013662.155 I: render: hrt[0]: 0001c000 000000fa 00000000 00000000 00000000 00000000 00000000 00000000
1791013662.167 I: render: hrt[8]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013662.181 I: render: hrt[16]: 00000000 c00ea300 00000000 00000000 c00d8da8 c00d8d28 c00d8d68 00000000
1791013662.181 I: render: hrt[24]: 00000000 00000000 01fb2000 00000080 00000000 00000000 00000000 00000000
1791013662.191 I: render: hrt[32]: 00000000 00000000 00000000 00000000 01fae000 00000080 01fc4000 00000080
1791013662.208 I: render: hrt[40]: 01fb4000 00000080 00000000 00000000 00000000 00000000 00000000 00000000
1791013662.209 I: render: hrt[48]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013662.215 I: render: hrt[56]: 00000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000
1791013662.227 I: render: HWRT 8x8 x1 te_screen=0x00001001 mtile1=0x00080000 mtile2=0x00080000 isp_mtile=0x00020002 stride=4 rgn=16 tpc=128 teaa=0
1791013662.229 I:   cube-bind: 0x8001f5f000 unit 0 kick 118 (draw)
1791013662.242 I:   cube(draw) @0x8001f5f000 8x8 x3: fmt 0, cpu 0x90a0e000 - not dumped
1791013662.255 gl_test:   after FBO: layer 0           (32,32) = 0x6400ff00  want 0x6400ff00  ok  [check 4]
1791013662.255 I:   cube-bind: 0x8001f5f000 unit 0 kick 119 (draw)
1791013662.259 I:   cube(draw) @0x8001f5f000 8x8 x3: fmt 0, cpu 0x90a0e000 - not dumped
1791013662.269 gl_test:   after FBO: layer 1           (32,32) = 0xffff0000  want 0xffff0000  ok  [check 5]
1791013662.274 I:   cube-bind: 0x8001f5f000 unit 0 kick 120 (draw)
1791013662.285 I:   cube(draw) @0x8001f5f000 8x8 x3: fmt 0, cpu 0x90a0e000 - not dumped
1791013662.294 gl_test:   after FBO: layer 2           (32,32) = 0xc8505fa0  want 0xc8505fa0  ok  [check 6]
1791013662.294 gl_test: case 42 es3array: PASS
1791013662.301 gl_test: px GGGGGGGGGGGGGGGGRRRRRRRRRRRRRRRR
1791013662.319 gl_test: px GGGGGGGGGGGGGGGGRRRRRRRRRRRRRRRB
1791013662.319 gl_test: px GGGGGGGGGGGGGGGRRRRRRRRRRRRRRRBB
1791013662.319 gl_test: px GGGGGGGGGGGGGGGRRRRRRRRRRRRRRBBB
1791013662.319 gl_test: px GGGGGGGGGGGGGGRRRRRRRRRRRRRRBBBB
1791013662.320 gl_test: px GGGGGGGGGGGGGGRRRRRRRRRRRRRBBBBB
1791013662.329 gl_test: px GGGGGGGGGGGGGRRRRRRRRRRRRRBBBBBB
1791013662.335 gl_test: px GGGGGGGGGGGGGRRRRRRRRRRRRBBBBBBB
1791013662.341 gl_test: px GGGGGGGGGGGGRRRRRRRRRRRRBBBBBBBB
1791013662.341 gl_test: px GGGGGGGGGGGGRRRRRRRRRRRBBBBBBBBB
1791013662.347 gl_test: px GGGGGGGGGGGRRRRRRRRRRRBBBBBBBBBB
1791013662.357 gl_test: px GGGGGGGGGGGRRRRRRRRRRBBBBBBBBBBB
1791013662.357 gl_test: px GGGGGGGGGGRRRRRRRRRRBBBBBBBBBBBB
1791013662.358 gl_test: px GGGGGGGGGGRRRRRRRRRBBBBBBBBBBBBB
1791013662.363 gl_test: px GGGGGGGGGRRRRRRRRRBBBBBBBBBBBBBB
1791013662.369 gl_test: px GGGGGGGGGRRRRRRRRBBBBBBBBBBBBBBB
1791013662.373 gl_test: px GGGGGGGGRRRRRRRRRBBBBBBBBBBBBBBB
1791013662.373 gl_test: px GGGGGGGGRRRRRRRRRRBBBBBBBBBBBBGG
1791013662.379 gl_test: px GGGGGGGRRRRRRRRRRRRBBBBBBBBBGGGG
1791013662.379 gl_test: px GGGGGGGRRRRRRRRRRRRRBBBBBBGGGGGG
1791013662.385 gl_test: px GGGGGGRRRRRRRRRRRRRRRBBBGGGGGGGG
1791013662.389 gl_test: px GGGGGGRRRRRRRRRRRRRRRRGGGGGGGGGG
1791013662.394 gl_test: px GGGGGRRRRRRRRRRRRRRRRGGGGGGGGGGG
1791013662.399 gl_test: px GGGGGRRRRRRRRRRRRRRRRGGGGGGGGGGG
1791013662.405 gl_test: px GGGGRRRRRRRRRRRRRRRRGGGGGGGGGGGG
1791013662.405 gl_test: px WWWWWWWRRRRRRRRRRRRRGGGGGGGGGGGG
1791013662.410 gl_test: px WWWWWWRRRRRRRRRRRRRGGGGGGGGGGGGG
1791013662.410 gl_test: px WWWWWRRRRRRRRRRRRRRGGGGGGGGGGGGG
1791013662.429 gl_test: px WWWWRRRRRRRRRRRRRRGGGGGGGGGGGGGG
1791013662.430 gl_test: px WWWRRRRRRRRRRRRRRRGGGGGGGGGGGGGG
1791013662.430 gl_test: px WWRRRRRRRRRRRRRRRGGGGGGGGGGGGGGG
1791013662.430 gl_test: px WRRRRRRRRRRRRRRRRGGGGGGGGGGGGGGG
1791013662.437 gl_test:   bottom-right ~ vertex 1 green (60,60) = 0xff0ee30e  want 0xff0ee30e  ok  [check 1]
1791013662.448 gl_test:   bottom-left ~ vertex 0 red   ( 4,60) = 0xff0e04ed  want 0xff1200ed  ok  [check 2]
1791013662.453 gl_test:   top-left ~ vertex 5 green    ( 4, 4) = 0xff12ed24  want 0xff12ed24  ok  [check 3]
1791013662.460 gl_test: case 43 es3vid: PASS
1791013662.461 I:   cube-bind: 0x8001e80000 unit 1 kick 122 (draw)
1791013662.466 I:   cube(draw) @0x8001e80000 8x8 x3: fmt 0, cpu 0x908ed000 - not dumped
1791013662.481 gl_test:   textureSize 2D / array       (32,32) = 0x03081808  want 0x03081808  ok  [check 1]
1791013662.481 gl_test: case 46 es3size: PASS
1791013662.497 gl_test:   arrays: instance 0 red       (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 1]
1791013662.503 gl_test:   arrays: instance 1 green     (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 2]
1791013662.508 gl_test:   arrays: instance 2 blue      (16,16) = 0xffff0000  want 0xffff0000  ok  [check 3]
1791013662.522 gl_test:   arrays: instance 3 half white (48,16) = 0x80808080  want 0x80808080  ok  [check 4]
1791013662.538 gl_test:   arrays: between = clear      (32,32) = 0xff000000  want 0xff000000  ok  [check 5]
1791013662.540 gl_test:   elements: instance 0 red     (16,48) = 0xff0000ff  want 0xff0000ff  ok  [check 6]
1791013662.550 gl_test:   elements: instance 1 green   (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 7]
1791013662.563 gl_test:   elements: instance 2 blue    (16,16) = 0xffff0000  want 0xffff0000  ok  [check 8]
1791013662.563 gl_test:   elements: instance 3 half white (48,16) = 0x80808080  want 0x80808080  ok  [check 9]
1791013662.571 gl_test:   elements: between = clear    (32,32) = 0xff000000  want 0xff000000  ok  [check 10]
1791013662.571 gl_test: case 47 es3inst: PASS
1791013662.581 I:   cube-bind: 0x8001ebd000 unit 0 kick 125 (draw)
1791013662.586 I:   cube(draw) @0x8001ebd000 4x4 x5: fmt 0, cpu 0x90936000 - not dumped
1791013662.596 gl_test:   slice 2: texture/lod/fetch/depth (32,32) = 0x05466464  want 0x05506464  MISMATCH  [check 1]
1791013662.606 I:   cube-bind: 0x8001ebd000 unit 0 kick 126 (draw)
1791013662.611 I:   cube(draw) @0x8001ebd000 4x4 x5: fmt 0, cpu 0x90936000 - not dumped
1791013662.621 gl_test:   between 0 and 1: blended     (32,32) = 0x05501c1c  want 0x05281c1c  MISMATCH  [check 2]
1791013662.621 gl_test: case 48 es3vol: FAIL
1791013662.639 gl_test:   window, front CCW: front (green) (32,32) = 0xff00ff00  want 0xff00ff00  ok  [check 1]
1791013662.650 gl_test:   window, front CW: back (red) (32,32) = 0xff0000ff  want 0xff0000ff  ok  [check 2]
1791013662.665 gl_test:   fbo, front CCW: front (green x0.25) (32,32) = 0x40004000  want 0x40004000  ok  [check 3]
1791013662.673 gl_test:   fbo, front CW: back (red x0.25) (32,32) = 0x40000040  want 0x40000040  ok  [check 4]
1791013662.680 gl_test: case 49 es3facing: PASS
1791013662.688 gl_test:   idx=2 n=3 -> cols[2..4]      (32,32) = 0xbf9900e5  want 0xbf9900e6  ok  [check 1]
1791013662.698 gl_test:   idx=5 n=2 -> cols[5..6]      (32,32) = 0x806600ff  want 0x806600ff  ok  [check 2]
1791013662.703 gl_test: case 45 es3dyn: PASS
1791013662.718 gl_test:   rgba8 clear-only x0.25       (32,32) = 0x40401020  want 0x40401020  ok  [check 1]
1791013662.728 gl_test:   cleared 2.0/0.5/0.25 x0.25   (32,32) = 0x40102080  want 0x40102080  ok  [check 2]
1791013662.748 gl_test:   drawn: bottom-right ~ green x0.25 (60,60) = 0x40043804  want 0x40043904  ok  [check 3]
1791013662.748 gl_test:   drawn: bottom-left ~ red x0.25 ( 4,60) = 0x4004003c  want 0x4005003b  ok  [check 4]
1791013662.772 gl_test:   clear+draw: right half = clear (56,32) = 0x40102080  want 0x40102080  ok  [check 5]
1791013662.772 gl_test:   clear+draw: left half drawn  ( 8,56) = 0x40040834  want 0x4005003b  ok  [check 6]
1791013662.772 gl_test: case 44 es3hdr: PASS
1791013662.780 I: osal(zephyr): ready (regs=0xfd80000)
1791013662.785 gl_test:   a 16-bit config is offered   = 1  want 1  ok  [check 1]
1791013662.790 gl_test:   32-bit configs are offered too = 1  want 1  ok  [check 2]
1791013662.800 gl_test:   green size 6                 = 6  want 6  ok  [check 3]
1791013662.804 gl_test:   32-bit config refuses the window = 1  want 1  ok  [check 4]
1791013662.809 gl_test:   RGB565 window surface        = 1  want 1  ok  [check 5]
1791013662.826 gl_test:   eglSwapBuffers               = 1  want 1  ok  [check 6]
1791013662.831 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.881 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.886 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.886 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.931 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.931 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.931 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.931 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.931 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.931 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.931 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.931 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.931 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.931 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.931 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.931 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.935 gl_test: px RRRRRRRRRRGRRRRRRRRRRRRRRRRRRRRR
1791013662.935 gl_test: px RRRRRRRRRRGGRRRRRRRRRBBRRRRRRRRR
1791013662.943 gl_test: px RRRRRRRRRGGGGRRRRRRRBBBRRRRRRRRR
1791013662.943 gl_test: px RRRRRRRRRGGGGGGRRRBBBBBBRRRRRRRR
1791013662.943 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBRRRRRRRR
1791013662.943 gl_test: px RRRRRRRRGGGGGGGGBBBBBBBBBRRRRRRR
1791013662.943 gl_test: px RRRRRRRGGGGGGGGGBBBBBBBBBRRRRRRR
1791013662.943 gl_test: px RRRRRRRGGGGGGGGGBBBBBBBBBBRRRRRR
1791013662.943 gl_test: px RRRRRRGGGGGGGGGGBBBBBBBBBBRRRRRR
1791013662.943 gl_test: px RRRRRRGGGGGGGGGGBBBBBBBBBBBRRRRR
1791013662.943 gl_test: px RRRRRGGGGGGGGGGGBBBBBBBBBBBRRRRR
1791013662.943 gl_test: px RRRRRGGGGGGGGGGGBBBBBBBBBBBBRRRR
1791013662.948 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.948 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.953 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.972 gl_test: px RRRRRRRRRRRRRRRRRRRRRRRRRRRRRRRR
1791013662.972 gl_test:   565: centroid (5/6-bit steps) (32,40) = 0xff5a5552  want 0xff595452  ok  [check 7]
1791013662.972 gl_test:   565: red background          ( 1, 1) = 0xff0000ff  want 0xff0000ff  ok  [check 8]
1791013662.976 gl_test:   565: raw red pixel = 0xF800  = 63488  want 63488  ok  [check 9]
1791013662.988 gl_test:   eglSwapBuffers 2             = 1  want 1  ok  [check 10]
1791013662.993 gl_test:   565: earlier frame kept      (32,40) = 0xff5a5552  want 0xff595452  ok  [check 11]
1791013663.003 gl_test:   565: background kept         ( 1, 1) = 0xff0000ff  want 0xff0000ff  ok  [check 12]
1791013663.012 gl_test:   565: corner triangle blue    ( 6,60) = 0xffff0000  want 0xffff0000  ok  [check 13]
1791013663.018 gl_test:   release                      = 1  want 1  ok  [check 14]
1791013663.025 gl_test:   eglDestroySurface            = 1  want 1  ok  [check 15]
1791013663.029 gl_test:   eglDestroyContext            = 1  want 1  ok  [check 16]
1791013663.034 gl_test:   eglTerminate                 = 1  want 1  ok  [check 17]
1791013663.034 gl_test: case 21 rgb565: PASS
1791013663.047 I: osal(zephyr): ready (regs=0xfd80000)
1791013663.055 gl_test:   two contexts created         = 1  want 1  ok  [check 1]
1791013663.065 gl_test:   the context pool refuses past its limit = 1  want 1  ok  [check 2]
1791013663.080 gl_test:   and refuses with EGL_BAD_ALLOC = 1  want 1  ok  [check 3]
1791013663.083 gl_test:   makeCurrent A                = 1  want 1  ok  [check 4]
1791013663.092 gl_test: 4x4 texture @0x8001eea000: ff0000ff ff0000ff ff00ff00 ff00ff00 | row1 ff0000ff ff0000ff
1791013663.106 gl_test:   A: centroid                  (32,40) = 0xff5a5552  want 0xff595452  ok  [check 5]
1791013663.114 gl_test:   A: red background            ( 1, 1) = 0xff0000ff  want 0xff0000ff  ok  [check 6]
1791013663.134 gl_test:   makeCurrent B                = 1  want 1  ok  [check 7]
1791013663.137 gl_test:   B: A's program is unknown    = 0  want 0  ok  [check 8]
1791013663.142 gl_test:   B: A's texture is unknown    = 0  want 0  ok  [check 9]
1791013663.154 gl_test:   B: default clear colour      = 1  want 1  ok  [check 10]
1791013663.157 gl_test:   B: blue quad                 (32,32) = 0xffff0000  want 0xffff0000  ok  [check 11]
1791013663.162 gl_test:   makeCurrent A again          = 1  want 1  ok  [check 12]
1791013663.183 gl_test:   A: program still there       = 1  want 1  ok  [check 13]
1791013663.183 gl_test:   A: texture still there       = 1  want 1  ok  [check 14]
1791013663.183 gl_test:   A: clear colour kept         = 1  want 1  ok  [check 15]
1791013663.191 gl_test:   A: surface kept              (32,40) = 0xff5a5552  want 0xff595452  ok  [check 16]
1791013663.201 gl_test:   A: redraw with its program   (32,40) = 0xff5a5552  want 0xff595452  ok  [check 17]
1791013663.205 gl_test: case 17 ctx2: PASS
1791013663.218 I: osal(zephyr): ready (regs=0xfd80000)
1791013663.227 gl_test:   EGL_KHR_image_base advertised = 1  want 1  ok  [check 1]
1791013663.248 gl_test: 8x8 texture @0x8001ef5000: ff0000ff ff0000ff ff0000ff ff0000ff | row1 ff0000ff ff0000ff
1791013663.255 gl_test:   image from GL texture        = 1  want 1  ok  [check 2]
1791013663.258 gl_test:   image from a bad texture fails = 1  want 1  ok  [check 3]
1791013663.262 gl_test:   image from native buffer     = 1  want 1  ok  [check 4]
1791013663.280 gl_test:   GL_OES_EGL_image advertised  = 1  want 1  ok  [check 5]
1791013663.291 gl_test:   texture image: top-left blue (16,16) = 0xffff0000  want 0xffff0000  ok  [check 6]
1791013663.304 gl_test:   texture image: bottom-right green (48,48) = 0xff00ff00  want 0xff00ff00  ok  [check 7]
1791013663.312 gl_test:   native image: bottom row red (32,60) = 0xff0000ff  want 0xff0000ff  ok  [check 8]
1791013663.326 gl_test:   native image: top row white  (32, 4) = 0xffffffff  want 0xffffffff  ok  [check 9]
1791013663.327 gl_test:   native image: second row green (32,44) = 0xff00ff00  want 0xff00ff00  ok  [check 10]
1791013663.337 gl_test:   image kept after source delete (16,16) = 0xffff0000  want 0xffff0000  ok  [check 11]
1791013663.342 gl_test:   eglDestroyImage              = 1  want 1  ok  [check 12]
1791013663.347 gl_test: case 18 image: PASS
1791013663.367 gl_test: all: 1 of 38 cases failed (37 passed; 0 timed out; first failure case 48 at index 31)
1791013663.367 gl_test: done
1791013663.381 osal(zephyr): pending frees: high-water 98/128, waits 0 (opt-in 0), refused 0 (0 unsubmitted, 0 no wait, 0 KiB kept)
1791013663.399 osal(zephyr): ds slots peak 3/8, ds bytes peak 81920, ds refused 0 (slots full 0, no buffer 0)
1791013663.399 gl_test: FAILED (rc=-1)
1791013663.399 I: dss stats before hold t=10511 ms: vsyncs 0 app_frames 0 underflows 0 sync_lost 0 go_timeouts 0 go_waits 0 safety_flips 0 app 0
1791013683.412 I: dss stats after hold t=30523 ms: vsyncs 0 app_frames 0 underflows 0 sync_lost 0 go_timeouts 0 go_waits 0 safety_flips 0 app 0
1791013683.412 I: sample done (-1): resetting the SoC
1791013683.601 �
1791013683.612 U-Boot SPL 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
1791013683.621 SYSFW ABI: 4.0 (firmware rev 0x000b '11.2.5--v11.02.05 (Fancy Rat)')
1791013683.665 Changed A53 CPU frequency to 1250000000Hz (U grade) in DT
1791013683.670 SPL initial stack usage: 17088 bytes
1791013683.680 Trying to boot from MMC1
1791013683.790 Authentication passed
1791013683.795 Authentication passed
1791013683.806 Authentication passed
1791013683.868 Loading Environment from nowhere... OK
1791013683.872 init_env from device 9 not supported!
1791013683.878 Authentication passed
1791013683.897 Authentication passed
1791013683.914 Starting ATF on ARM64 core...
1791013683.914 
1791013683.917 NOTICE:  BL31: v2.13.0(release):v2.13.0-259-ge0c4d3903b-dirty
1791013683.919 NOTICE:  BL31: Built : 00:00:00, Jan  1 1970
1791013684.140 
1791013684.150 U-Boot SPL 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
1791013684.171 SYSFW ABI: 4.0 (firmware rev 0x000b '11.2.5--v11.02.05 (Fancy Rat)')
1791013684.172 SPL initial stack usage: 1920 bytes
1791013684.172 HW CFG: 0x00
1791013684.172 Trying to boot from MMC1
1791013684.278 Authentication passed
1791013684.278 Authentication passed
1791013685.786 
1791013685.786 
1791013685.800 U-Boot 2024.04-ti-g11cb0162a795 (Jan 01 1970 - 00:00:00 +0000)
1791013685.800 
1791013685.808 SoC:   AM62PX SR1.0 HS-FS
1791013685.808 DRAM:  2 GiB
1791013685.917 optee optee: OP-TEE: revision 4.7 (a9690ae39995af36)
1791013685.928 Core:  161 devices, 34 uclasses, devicetree: separate
1791013685.945 MMC:   mmc@fa10000: 0, mmc@fa00000: 1
1791013686.032 Loading Environment from MMC... OK
1791013686.056 In:    serial@2800000
1791013686.060 Out:   serial@2800000
1791013686.067 Err:   serial@2800000
1791013686.086 Model: Toradex 0099 Verdin AM62P Quad 2GB WB IT V1.0B
1791013686.089 Serial# 00000000
1791013686.089 Carrier: Toradex Dahlia V1.1C, Serial# 00000000
1791013686.089 am65_cpsw_nuss ethernet@8000000: K3 CPSW: nuss_ver: 0x6BA01903 cpsw_ver: 0x6BA81903 ale_ver: 0x00290105 Ports:2
1791013686.145 Net:   eth0: ethernet@8000000port@1 [PRIME]Could not get PHY for mdio@f00: addr 7
1791013686.154 am65_cpsw_nuss_port ethernet@8000000port@2: phy_connect() failed
1791013686.154 
1791013686.158 Hit any key to stop autoboot:  0 
1791013686.219 6188160 bytes read in 55 ms (107.3 MiB/s)
1791013686.248 288 bytes read in 20 ms (13.7 KiB/s)
1791013686.253 Failed to load '/stock/zephyr_a53.bin'
1791013686.260 stock images not found, booting Linux
1791013686.362 7[r[999;999H[6n8MMC: no card present
1791013686.362 No EFI system partition
1791013686.368 No EFI system partition
1791013686.368 Failed to persist EFI variables
1791013686.368 No EFI system partition
1791013686.376 Failed to persist EFI variables
1791013686.376 No EFI system partition
1791013686.380 Failed to persist EFI variables
1791013686.385 ** Booting bootflow '<NULL>' with efi_mgr
1791013686.432 Loading Boot0000 'mmc 0' failed
1791013686.434 EFI boot manager: Cannot load any image
1791013686.434 Boot failed (err=-14)
1791013686.434 MMC: no card present
1791013686.434 MMC: no card present
1791013686.434 MMC: no card present
1791013686.434 MMC: no card present
1791013686.440 ** Booting bootflow 'mmc@fa10000.bootdev.part_1' with script
1791013686.465 6956 bytes read in 21 ms (323.2 KiB/s)
1791013686.492 75135 bytes read in 21 ms (3.4 MiB/s)
1791013686.533 65 bytes read in 21 ms (2.9 KiB/s)
1791013686.533 Working FDT set to 90200000
1791013686.533 Applying Overlay: verdin-am62p_panel-cap-touch-7inch-dsi_overlay.dtbo
1791013686.554 4693 bytes read in 22 ms (208 KiB/s)
1791013686.715 11272338 bytes read in 83 ms (129.5 MiB/s)
1791013686.814 13381763 bytes read in 94 ms (135.8 MiB/s)
1791013686.819    Uncompressing Kernel Image to 0
1791013687.223 ## Flattened Device Tree blob at 90200000
1791013687.232    Booting using the fdt blob at 0x90200000
1791013687.232 Working FDT set to 90200000
1791013687.241    Loading Ramdisk to fc1d3000, end fce96083 ... OK
1791013687.344    Loading Device Tree to 00000000fc19d000, end 00000000fc1d2fff ... OK
1791013687.344 Working FDT set to fc19d000
1791013687.344 
1791013687.344 Starting kernel ...
1791013687.344 
1791013688.557 [    1.058430] tps65219 0-0030: Error IRQ trap residual voltage for LDO4
1791013688.564 [    1.083755] debugfs: File ':bus@f0000:bus@48000000:interrupt-controller@48000000' in directory 'domains' already present!
1791013688.582 [    1.084091] debugfs: File ':bus@f0000:bus@4e000000:interrupt-controller@4e400000' in directory 'domains' already present!
1791013689.128 [    1.669923] OF: graph: no port node found in /bus@f0000/dss@30220000/oldi-txes/oldi@1/ports
1791013689.187 [    1.735508] OF: graph: no port node found in /bus@f0000/dss@30220000/oldi-txes/oldi@1/ports
1791013689.198 [    1.745285] cdns-dsi 30500000.dsi: [drm] *ERROR* modalias failure on /bus@f0000/dsi@30500000/port@1
1791013689.210 [    1.754478] cdns-dsi 30500000.dsi: [drm] *ERROR* modalias failure on /bus@f0000/dsi@30500000/port@0
1791013689.217 [    1.769419] OF: graph: no port node found in /bus@f0000/dss@30220000/oldi-txes/oldi@1/ports
1791013689.254 Starting systemd-udevd version 255.21^
1791013690.217 sysroot.readonly configuration value: 0 (fs writable: 1)
1791013690.220 Using legacy ostree bind mount for /
1791013695.543 [    8.093108] omap-mailbox 29000000.mailbox: no available mbox devices found
1791013695.678 [    8.230162] omap-mailbox 29020000.mailbox: no available mbox devices found
1791013695.710 [    8.261020] omap-mailbox 29030000.mailbox: no available mbox devices found
1791013695.719 [    8.270122] vdec 30210000.video-codec: OPP table not found in device tree
1791013696.385 [    8.941395] remoteproc remoteproc0: request_firmware failed: -2
1791013696.475 [    8.963478] platform 78000000.r5f: mbox_request_channel: can't parse "mboxes" property
1791013696.498 [    9.050238] platform 78000000.r5f: error -ENOENT: mbox_request_channel failed
1791013696.508 [    9.058709] k3_r5_rproc bus@f0000:bus@b00000:r5fss@78000000: k3_r5_cluster_rproc_init failed, ret = -2
