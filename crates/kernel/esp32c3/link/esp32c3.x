/* Linker script for ESP32-C3 — VeerOS
 *
 * The ESP-IDF 2nd-stage bootloader expects the app image to contain:
 *   - Exactly 2 flash-mapped segments (DROM + IROM)
 *   - 1+ SRAM-loaded segments (data)
 *
 * Code (.text) and read-only data (.rodata) run from flash via the
 * instruction / data cache.  Only mutable data (.data, .bss, stacks,
 * heap) is placed in DRAM.
 *
 * Memory map (ESP32-C3):
 *   0x4000_0000 .. 0x4005_FFFF   384 KB  IROM (instruction cache, flash-mapped)
 *   0x3C00_0000 .. 0x3C3F_FFFF     4 MB  DROM (data cache, flash-mapped)
 *   0x4037_C000 .. 0x403D_FFFF   400 KB  IRAM (instruction SRAM)
 *   0x3FC8_0000 .. 0x3FCE_3FFF   400 KB  DRAM (data SRAM, mirrors IRAM)
 *
 * The 0x20 offset on FLASH accounts for the ESP image header (24 B)
 * plus the segment header (8 B) that espflash prepends to each segment.
 */

ENTRY(_start);

MEMORY
{
    /* Flash-mapped data cache (read-only data, app descriptor).
       Placed first in the image at 0x3C00_0020. */
    DROM  (r)  : ORIGIN = 0x3C000020, LENGTH = 2M

    /* Flash-mapped instruction cache (code).
       Starts at 0x4200_0020 so espflash generates two distinct
       ROM segments for the bootloader. */
    IROM  (rx) : ORIGIN = 0x42000020, LENGTH = 2M

    /* On-chip DRAM for data, BSS, heap, and stacks.
       Total 400 KB from 0x3FC8_0000 to 0x3FCE_3FFF.
       Reserve top ~2 KB (0x3FCDF800 onwards) for ROM data symbols
       used by WiFi/BLE blobs. */
    DRAM  (rw) : ORIGIN = 0x3FC80000, LENGTH = 0x5F800
}

/* ══════════════════════════════════════════════════════════════════════════
 * ESP32-C3 ROM symbol addresses — from ESP-IDF esp32c3.rom.ld.
 * These map WiFi/PHY/PP/net80211 blob symbols to their fixed ROM locations.
 * ══════════════════════════════════════════════════════════════════════════ */

/* ── rom common functions ───────────────────────────────────────────── */
PROVIDE( ets_delay_us               = 0x40000050 );
PROVIDE( ets_printf                 = 0x40000040 );
PROVIDE( ets_backup_dma_copy        = 0x4000005c );
PROVIDE( roundup2                   = 0x40001894 );

/* ── rom phy functions ──────────────────────────────────────────────── */
PROVIDE( phy_get_romfuncs           = 0x40000dc4 );
PROVIDE( phy_param_addr             = 0x40000dc0 );

/* ── rom_net80211 functions ─────────────────────────────────────────── */
PROVIDE( chip_v7_set_chan_ana       = 0x40001b58 );
PROVIDE( esp_net80211_rom_version_get = 0x40001820 );

/* ── rom_pp functions ───────────────────────────────────────────────── */
PROVIDE( esp_pp_rom_version_get     = 0x40000430 );
PROVIDE( rom_read_hw_noisefloor     = 0x400004fc );

/* ── rom_net80211 data (fixed DRAM addresses) ───────────────────────── */
PROVIDE( g_ic_ptr                   = 0x3fcdf860 );
PROVIDE( net80211_funcs             = 0x3fcdf86c );
PROVIDE( g_scan                     = 0x3fcdf868 );
PROVIDE( g_chm                      = 0x3fcdf864 );
PROVIDE( g_hmac_cnt_ptr             = 0x3fcdf85c );
PROVIDE( g_tx_cacheq_ptr           = 0x3fcdf858 );
PROVIDE( s_netstack_free            = 0x3fcdf854 );
PROVIDE( mesh_rxcb                  = 0x3fcdf850 );
PROVIDE( sta_rxcb                   = 0x3fcdf84c );

/* ── rom_pp data (fixed DRAM addresses) ─────────────────────────────── */
PROVIDE( pTxRx                      = 0x3fcdf968 );
PROVIDE( lmacConfMib_ptr            = 0x3fcdf964 );
PROVIDE( our_wait_eb                = 0x3fcdf960 );
PROVIDE( our_tx_eb                  = 0x3fcdf95c );
PROVIDE( pp_wdev_funcs              = 0x3fcdf958 );
PROVIDE( g_osi_funcs_p              = 0x3fcdf954 );
PROVIDE( wDevCtrl_ptr               = 0x3fcdf950 );
PROVIDE( wDevMacSleep_ptr           = 0x3fcdf94c );
PROVIDE( g_lmac_cnt_ptr             = 0x3fcdf948 );
PROVIDE( pp_sig_cnt_ptr             = 0x3fcdf944 );
PROVIDE( g_eb_list_desc_ptr         = 0x3fcdf940 );
PROVIDE( s_fragment_ptr             = 0x3fcdf93c );
PROVIDE( if_ctrl_ptr                = 0x3fcdf938 );
PROVIDE( g_intr_lock_mux            = 0x3fcdf934 );
PROVIDE( g_wifi_global_lock         = 0x3fcdf930 );
PROVIDE( s_wifi_queue               = 0x3fcdf92c );
PROVIDE( pp_task_hdl                = 0x3fcdf928 );
PROVIDE( s_pp_task_create_sem       = 0x3fcdf924 );
PROVIDE( s_pp_task_del_sem          = 0x3fcdf920 );
PROVIDE( g_wifi_menuconfig_ptr      = 0x3fcdf91c );
PROVIDE( xphyQueue                  = 0x3fcdf918 );
PROVIDE( ap_no_lr_ptr               = 0x3fcdf914 );
PROVIDE( rc11BSchedTbl_ptr          = 0x3fcdf910 );
PROVIDE( rc11NSchedTbl_ptr          = 0x3fcdf90c );
PROVIDE( rcLoRaSchedTbl_ptr         = 0x3fcdf908 );
PROVIDE( BasicOFDMSched_ptr         = 0x3fcdf904 );
PROVIDE( trc_ctl_ptr                = 0x3fcdf900 );
PROVIDE( g_pm_cnt_ptr               = 0x3fcdf8fc );
PROVIDE( g_pm_ptr                   = 0x3fcdf8f8 );
PROVIDE( g_pm_cfg_ptr               = 0x3fcdf8f4 );
PROVIDE( g_esp_mesh_quick_funcs_ptr = 0x3fcdf8f0 );
PROVIDE( g_txop_queue_status_ptr    = 0x3fcdf8ec );
PROVIDE( g_mac_sleep_en_ptr         = 0x3fcdf8e8 );
PROVIDE( g_mesh_is_root_ptr         = 0x3fcdf8e4 );
PROVIDE( g_mesh_topology_ptr        = 0x3fcdf8e0 );
PROVIDE( g_mesh_init_ps_type_ptr    = 0x3fcdf8dc );
PROVIDE( g_mesh_is_started_ptr      = 0x3fcdf8d8 );
PROVIDE( g_config_func              = 0x3fcdf8d4 );
PROVIDE( g_net80211_tx_func         = 0x3fcdf8d0 );
PROVIDE( g_timer_func               = 0x3fcdf8cc );
PROVIDE( s_michael_mic_failure_cb   = 0x3fcdf8c8 );
PROVIDE( wifi_sta_rx_probe_req      = 0x3fcdf8c4 );
PROVIDE( g_tx_done_cb_func          = 0x3fcdf8c0 );
PROVIDE( g_per_conn_trc             = 0x3fcdf874 );
PROVIDE( s_encap_amsdu_func         = 0x3fcdf870 );

/* ── rom_pp data (read-only ROM constant region) ────────────────────── */
PROVIDE( our_instances_ptr          = 0x3ff1ee44 );
PROVIDE( g_wdev_last_desc_reset_ptr = 0x3ff1ee40 );
PROVIDE( our_controls_ptr           = 0x3ff1ee3c );

SECTIONS
{
    /* ── DROM segment (read-only data, flash-mapped) ────────────
       Must come FIRST so it becomes segment 0 in the image —
       the bootloader checks segment 0 for the app descriptor magic. */
    .rodata : ALIGN(4)
    {
        _rodata_start = ABSOLUTE(.);
        KEEP(*(.veeros.appdesc));
        *(.rodata .rodata.*);
        *(.srodata .srodata.*);
        /* WiFi blob log-string sections (rodata_wlog_*) */
        *(.rodata_wlog_debug .rodata_wlog_debug.*);
        *(.rodata_wlog_error .rodata_wlog_error.*);
        *(.rodata_wlog_info .rodata_wlog_info.*);
        *(.rodata_wlog_verbose .rodata_wlog_verbose.*);
        *(.rodata_wlog_warning .rodata_wlog_warning.*);
        _rodata_end = ABSOLUTE(.);
    } > DROM

    /* ── IROM segment (code, flash-mapped) ──────────────────── */
    .text : ALIGN(4)
    {
        _stext = ABSOLUTE(.);
        KEEP(*(.text._start));
        KEEP(*(.text._veer_trap_entry));
        KEEP(*(.text._veer_start_first_task));
        *(.text .text.*);
        /* WiFi blob IRAM sections — keep in flash for now. */
        *(.iram1 .iram1.*);
        *(.wifi0iram .wifi0iram.*);
        *(.wifiextrairam .wifiextrairam.*);
        *(.wifirxiram .wifirxiram.*);
        *(.wifislprxiram .wifislprxiram.*);
        *(.wifislpiram .wifislpiram.*);
        _etext = ABSOLUTE(.);
    } > IROM

    /* ── DRAM segment (mutable data, loaded to SRAM) ────────── */
    .data : ALIGN(4)
    {
        _data_start = ABSOLUTE(.);
        *(.data .data.*);
        *(.sdata .sdata.*);
        /* WiFi/PHY blob initialized data sections */
        *(.dram1 .dram1.*);
        _data_end = ABSOLUTE(.);
    } > DRAM

    .bss (NOLOAD) : ALIGN(4)
    {
        __bss_start = .;
        *(.bss .bss.*);
        *(.sbss .sbss.*);
        *(COMMON);
        __bss_end = .;
    } > DRAM

    /* Kernel boot stack — 8 KiB, placed after BSS. */
    .stack (NOLOAD) : ALIGN(16)
    {
        __stack_bottom = .;
        . += 8K;
        __stack_top = .;
    } > DRAM

    /DISCARD/ :
    {
        *(.eh_frame)
        *(.comment)
    }
}
