/* Linker script for ESP32-C6 — Seeed XIAO ESP32-C6
 *
 * The ESP-IDF 2nd-stage bootloader expects the app image to contain:
 *   - Exactly 2 flash-mapped segments (DROM + IROM)
 *   - 1+ SRAM-loaded segments (data)
 *
 * Code (.text) and read-only data (.rodata) run from flash via the
 * instruction / data cache at 0x4200_0000.  Only mutable data (.data,
 * .bss, stacks, heap) is placed in the 512 KiB HP SRAM at 0x4080_0000.
 *
 * The 0x20 offset on FLASH accounts for the ESP image header (24 B)
 * plus the segment header (8 B) that espflash prepends to each segment.
 *
 * Memory map (HP core):
 *   0x4000_0000 .. 0x4004_FFFF   320 KB  Boot ROM (read-only)
 *   0x4080_0000 .. 0x4087_FFFF   512 KB  HP SRAM (data + bss + stack)
 *   0x4200_0000 .. 0x427F_FFFF    8 MB   Flash cache (IROM / DROM)
 */

ENTRY(_start);

MEMORY
{
    /* Flash-mapped data cache (read-only data, app descriptor).
       Placed first in the image at 0x4200_0020. */
    DROM  (r)  : ORIGIN = 0x42000020, LENGTH = 2M

    /* Flash-mapped instruction cache (code).
       Starts at a separate address (0x4220_0020) so espflash
       generates two distinct ROM segments for the bootloader. */
    IROM  (rx) : ORIGIN = 0x42200020, LENGTH = 2M

    /* On-chip SRAM for data, BSS, heap, and stacks.
       Last ~1 KiB (0x4087_FC00–0x4087_FFFF) is reserved for ROM data
       pointers used by the WiFi/PHY/PP blobs. */
    DRAM  (rw) : ORIGIN = 0x40800000, LENGTH = 0x7FC00
}

/* ══════════════════════════════════════════════════════════════════════════
 * ESP32-C6 ROM symbol addresses — from esp-rom-sys linker scripts.
 * These map WiFi/PHY/PP blob symbols to their fixed ROM locations.
 * ══════════════════════════════════════════════════════════════════════════ */

/* ── rom common ─────────────────────────────────────────────────────── */
PROVIDE( ets_delay_us               = 0x40000040 );
PROVIDE( roundup2                   = 0x40000088 );
PROVIDE( ets_printf                 = 0x40000028 );

/* ── rom phy functions ──────────────────────────────────────────────── */
PROVIDE( abs_temp                   = 0x40001130 );
PROVIDE( get_data_sat               = 0x40001134 );
PROVIDE( phy_byte_to_word           = 0x40001138 );
PROVIDE( freq_module_resetn         = 0x40001148 );
PROVIDE( get_tone_sar_dout          = 0x400011a8 );
PROVIDE( linear_to_db               = 0x400011b4 );
PROVIDE( get_power_db               = 0x400011b8 );
PROVIDE( chip_v7_set_chan_ana       = 0x40001250 );
PROVIDE( txiq_get_mis_pwr           = 0x40001270 );
PROVIDE( get_power_atten            = 0x4000127c );
PROVIDE( pwdet_code_cal             = 0x40001284 );
PROVIDE( i2c_sar2_init_code         = 0x400012ec );
PROVIDE( pbus_set_dco               = 0x40001328 );
PROVIDE( txcal_work_mode            = 0x4000132c );
PROVIDE( txiq_set_reg               = 0x40001388 );
PROVIDE( rxiq_set_reg               = 0x4000138c );
PROVIDE( dc_iq_est                  = 0x400013f4 );
PROVIDE( pbus_rx_dco_cal            = 0x40001410 );
PROVIDE( index_to_txbbgain          = 0x40001460 );
PROVIDE( get_rc_dout                = 0x40001114 );
PROVIDE( set_rfpll_freq             = 0x40001238 );
PROVIDE( write_pll_cap              = 0x40001248 );
PROVIDE( read_pll_cap               = 0x4000124c );
PROVIDE( freq_chan_en_sw            = 0x4000114c );
PROVIDE( get_freq_mem_addr          = 0x40001158 );
PROVIDE( pwdet_ref_code             = 0x40001280 );
PROVIDE( bb_bss_cbw40_dig           = 0x4000134c );
PROVIDE( cbw2040_cfg                = 0x40001350 );
PROVIDE( bt_gain_offset             = 0x40001364 );
PROVIDE( wifi_fbw_sel               = 0x400013b8 );
PROVIDE( phy_freq_correct           = 0x400013cc );
PROVIDE( code_to_temp               = 0x4000143c );
PROVIDE( tsens_code_read            = 0x4000144c );

/* ── rom_net80211 functions ─────────────────────────────────────────── */
PROVIDE( esp_net80211_rom_version_get = 0x40000b4c );
PROVIDE( wifi_get_macaddr           = 0x40000ba0 );

/* ── rom_net80211 data ──────────────────────────────────────────────── */
PROVIDE( net80211_funcs             = 0x4087ffac );
PROVIDE( g_scan                     = 0x4087ffa8 );
PROVIDE( g_chm                      = 0x4087ffa4 );
PROVIDE( g_ic_ptr                   = 0x4087ffa0 );
PROVIDE( g_hmac_cnt_ptr             = 0x4087ff9c );
PROVIDE( g_tx_cacheq_ptr            = 0x4087ff98 );
PROVIDE( s_netstack_free            = 0x4087ff94 );
PROVIDE( mesh_rxcb                  = 0x4087ff90 );
PROVIDE( sta_rxcb                   = 0x4087ff8c );
PROVIDE( g_itwt_fid                 = 0x4087ff88 );

/* ── rom_pp functions ───────────────────────────────────────────────── */
PROVIDE( esp_pp_rom_version_get     = 0x40000bd8 );

/* ── rom_pp data ────────────────────────────────────────────────────── */
PROVIDE( our_instances_ptr          = 0x4004ffe0 );
PROVIDE( g_wdev_last_desc_reset_ptr = 0x4004ffdc );
PROVIDE( our_controls_ptr           = 0x4004ffd8 );
PROVIDE( sigb_ru_allocation_user_num = 0x4004ffc8 );
PROVIDE( sigb_common_ru_allocation  = 0x4004ff38 );
PROVIDE( mu_mimo_special_cfg_user_num_2 = 0x4004fee8 );
PROVIDE( mu_mimo_special_cfg_user_num_3 = 0x4004fe80 );
PROVIDE( mu_mimo_special_cfg_user_num_4 = 0x4004fe28 );
PROVIDE( mu_mimo_special_cfg_user_num_5 = 0x4004fdf0 );
PROVIDE( mu_mimo_special_cfg_user_num_6 = 0x4004fdd0 );
PROVIDE( mu_mimo_special_cfg_user_num_7 = 0x4004fdc0 );
PROVIDE( mu_mimo_special_cfg_user_num_8 = 0x4004fdb8 );
PROVIDE( he_max_apep_length         = 0x4004fd40 );
PROVIDE( pTxRx                      = 0x4087ff80 );
PROVIDE( lmacConfMib_ptr            = 0x4087ff7c );
PROVIDE( our_wait_eb                = 0x4087ff78 );
PROVIDE( our_tx_eb                  = 0x4087ff74 );
PROVIDE( pp_wdev_funcs              = 0x4087ff70 );
PROVIDE( g_osi_funcs_p              = 0x4087ff6c );
PROVIDE( wDevCtrl_ptr               = 0x4087ff68 );
PROVIDE( wDevMacSleep_ptr           = 0x4087ff64 );
PROVIDE( g_lmac_cnt_ptr             = 0x4087ff60 );
PROVIDE( pp_sig_cnt_ptr             = 0x4087ff5c );
PROVIDE( g_eb_list_desc_ptr         = 0x4087ff58 );
PROVIDE( s_fragment_ptr             = 0x4087ff54 );
PROVIDE( if_ctrl_ptr                = 0x4087ff50 );
PROVIDE( g_intr_lock_mux            = 0x4087ff4c );
PROVIDE( g_wifi_global_lock         = 0x4087ff48 );
PROVIDE( s_wifi_queue               = 0x4087ff44 );
PROVIDE( pp_task_hdl                = 0x4087ff40 );
PROVIDE( s_pp_task_create_sem       = 0x4087ff3c );
PROVIDE( s_pp_task_del_sem          = 0x4087ff38 );
PROVIDE( g_wifi_menuconfig_ptr      = 0x4087ff34 );
PROVIDE( xphyQueue                  = 0x4087ff30 );
PROVIDE( ap_no_lr_ptr               = 0x4087ff2c );
PROVIDE( rc11BSchedTbl_ptr          = 0x4087ff28 );
PROVIDE( rc11NSchedTbl_ptr          = 0x4087ff24 );
PROVIDE( rcLoRaSchedTbl_ptr         = 0x4087ff20 );
PROVIDE( BasicOFDMSched_ptr         = 0x4087ff1c );
PROVIDE( trc_ctl_ptr                = 0x4087ff18 );
PROVIDE( g_pm_cnt_ptr               = 0x4087ff14 );
PROVIDE( g_pm_ptr                   = 0x4087ff10 );
PROVIDE( g_pm_cfg_ptr               = 0x4087ff0c );
PROVIDE( g_esp_mesh_quick_funcs_ptr = 0x4087ff08 );
PROVIDE( g_txop_queue_status_ptr    = 0x4087ff04 );
PROVIDE( g_mac_sleep_en_ptr         = 0x4087ff00 );
PROVIDE( g_mesh_is_root_ptr         = 0x4087fefc );
PROVIDE( g_mesh_topology_ptr        = 0x4087fef8 );
PROVIDE( g_mesh_init_ps_type_ptr    = 0x4087fef4 );
PROVIDE( g_mesh_is_started_ptr      = 0x4087fef0 );
PROVIDE( g_config_func              = 0x4087feec );
PROVIDE( g_net80211_tx_func         = 0x4087fee8 );
PROVIDE( g_timer_func               = 0x4087fee4 );
PROVIDE( s_michael_mic_failure_cb   = 0x4087fee0 );
PROVIDE( wifi_sta_rx_probe_req      = 0x4087fedc );
PROVIDE( g_tx_done_cb_func          = 0x4087fed8 );
PROVIDE( g_per_conn_trc             = 0x4087fe8c );
PROVIDE( s_encap_amsdu_func         = 0x4087fe88 );
PROVIDE( rx_beacon_count            = 0x4087fe84 );
PROVIDE( s_itwt_state               = 0x4087fe40 );
PROVIDE( g_dbg_interp_tsf           = 0x4087fe3c );
PROVIDE( g_dbg_interp_tsf_end       = 0x4087fe38 );
PROVIDE( s_he_min_len_bytes         = 0x4087fdf0 );
PROVIDE( s_he_dcm_min_len_bytes     = 0x4087fdd0 );
PROVIDE( s_mplen_low_bitmap         = 0x4087fdc0 );
PROVIDE( s_mplen_high_bitmap        = 0x4087fdb0 );
PROVIDE( esp_test_tx_statistics_aci_bitmap = 0x4087fda4 );
PROVIDE( esp_test_tx_statistics     = 0x4087fd94 );
PROVIDE( esp_test_tx_tb_statistics  = 0x4087fd84 );
PROVIDE( esp_test_tx_fail_statistics = 0x4087fd24 );
PROVIDE( esp_test_rx_statistics     = 0x4087fd1c );
PROVIDE( esp_test_rx_mu_statistics  = 0x4087fd18 );
PROVIDE( esp_test_mu_print_ru_allocation = 0x4087fd14 );
PROVIDE( esp_test_rx_error_occurs   = 0x4087fd10 );
PROVIDE( g_pp_tx_pkt_num            = 0x4087fd0c );
PROVIDE( amdpu_delay_time_ms        = 0x4087fd08 );
PROVIDE( ampdu_delay_packet         = 0x4087fd04 );
PROVIDE( s_ht_ampdu_density_us      = 0x4087fd02 );
PROVIDE( s_ht_ampdu_density         = 0x4087fd01 );
PROVIDE( s_running_phy_type         = 0x4087fd00 );
PROVIDE( esp_wifi_cert_tx_mcs       = 0x4087fcfc );
PROVIDE( esp_wifi_cert_tx_bcc       = 0x4087fcf8 );
PROVIDE( esp_wifi_cert_tx_nss       = 0x4087fcec );
PROVIDE( complete_ena_tb_seqno      = 0x4087fe4c );
PROVIDE( complete_ena_tb_final      = 0x4087fe48 );
PROVIDE( complete_ena_tb_count      = 0x4087fe44 );

/* ── rom_phy functions ──────────────────────────────────────────────── */
PROVIDE( phy_param_addr             = 0x40001104 );
PROVIDE( phy_get_romfuncs           = 0x40001108 );
PROVIDE( set_chan_reg               = 0x4000113c );
PROVIDE( i2c_master_reset           = 0x40001140 );
PROVIDE( write_chan_freq             = 0x40001150 );
PROVIDE( get_freq_mem_param         = 0x40001154 );
PROVIDE( freq_i2c_num_addr          = 0x40001170 );
PROVIDE( phy_en_hw_set_freq         = 0x40001188 );
PROVIDE( tx_pwctrl_bg_init          = 0x400011d0 );
PROVIDE( phy_enable_low_rate        = 0x400011f8 );
PROVIDE( phy_disable_low_rate       = 0x400011fc );
PROVIDE( phy_is_low_rate_enabled    = 0x40001200 );
PROVIDE( set_rx_sense_rom           = 0x40001214 );
PROVIDE( mhz2ieee                   = 0x40001220 );
PROVIDE( chan_to_freq                = 0x40001224 );
PROVIDE( set_channel_rfpll_freq     = 0x4000123c );
PROVIDE( i2c_paral_write_num        = 0x400012e4 );
PROVIDE( pbus_debugmode              = 0x40001320 );
PROVIDE( pbus_workmode               = 0x40001324 );
PROVIDE( disable_agc                 = 0x40001338 );
PROVIDE( enable_agc                  = 0x4000133c );
PROVIDE( i2cmst_reg_init             = 0x40001360 );
PROVIDE( fe_reg_init                 = 0x40001368 );
PROVIDE( mac_enable_bb               = 0x4000136c );
PROVIDE( bb_wdg_cfg                  = 0x40001370 );
PROVIDE( fe_txrx_reset               = 0x40001374 );
PROVIDE( set_txclk_en                = 0x40001390 );
PROVIDE( set_rxclk_en                = 0x40001394 );
PROVIDE( read_hw_noisefloor          = 0x400013a0 );
PROVIDE( iq_corr_enable              = 0x400013a4 );
PROVIDE( wifi_agc_sat_gain           = 0x400013a8 );
PROVIDE( phy_bbpll_cal               = 0x400013ac );
PROVIDE( phy_ant_init                = 0x400013b0 );
PROVIDE( set_pbus_reg                = 0x400013d0 );
PROVIDE( txbbgain_to_index           = 0x4000145c );
PROVIDE( bt_bb_to_index              = 0x40001468 );
PROVIDE( get_rate_fcc_index          = 0x40001480 );

/* ── rom_phy data ───────────────────────────────────────────────────── */
PROVIDE( phy_param_rom               = 0x4087fce8 );

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
        /* WiFi blob IRAM sections — must execute from SRAM when flash
           cache is disabled.  For now keep them in flash (IROM); move
           to a DRAM-loaded section if WiFi calibration faults. */
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
