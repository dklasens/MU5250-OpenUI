{
  "unauthenticated": {
    "description": "",
    "read": {
      "ubus": {
        "zwrt_web": [
          "web_login",
          "web_login_info",
          "web_language_get",
          "web_language_set",
          "web_quick_settings_init_flag_get",
          "web_privacy_read_flag_get"
        ],
        "zwrt_router.api": [
          "router_get_status_no_auth"
        ],
        "zwrt_mc.device.manager": [
          "fac_query_key",
          "fac_open",
          "adb_switch"
        ],
        "zwrt_wlan": [
          "report"
        ],
        "zwrt_zte_mdm.api": [
          "get_sim_info_before"
        ],
        "zte_nwinfo_api": [
          "nwinfo_get_netinfo"
        ],
        "uci": [
          "get"
        ]
      },
      "uci": [
        "zwrt_common_info"
      ]
    },
    "write": {
      "ubus": {
        "zwrt_router.api": [
          "router_mbblog_open"
        ],
        "zwrt_mc.device.manager": [
          "fac_open",
          "fac_close",
          "fac_reset",
          "fac_reboot"
        ]
      }
    }
  },
  "web": {
    "description": "",
    "read": {
      "ubus": {
        "zwrt_router.api": [
          "router_get_status",
          "router_get_user_list_num",
          "router_get_status_no_auth",
          "router_wireless_access_list",
          "router_lan_access_list",
          "router_set_lan_para",
          "router_set_wan_mtu",
          "router_set_public_ip",
          "router_set_remote_acl",
          "router_set_portforward",
          "router_set_portforward_switch",
          "router_set_macipport_filter_switch",
          "router_set_macipport_filter",
          "router_set_upnp_switch",
          "router_set_dmz",
          "router_set_portmapping",
          "router_set_portmapping_switch",
          "router_get_wan_mode_para",
          "router_set_cable_param_bak",
          "router_set_wan_mode",
          "router_set_pppoe_mode",
          "router_set_domain_filter",
          "router_get_pctrl_by_mac",
          "router_set_pctrl",
          "router_delete_pctrl_by_mac",
          "router_get_modified_lan_hostname",
          "router_modify_lan_hostname",
          "router_set_mac_ip_bind",
          "router_set_mac_ip_bond_switch",
          "router_after_sales_save_log",
          "router_set_ping_diagnose",
          "router_set_traceroute_diagnose",
          "router_set_watchdog",
          "router_set_nat_switch",
          "router_get_ddns",
          "router_set_ddns",
          "router_get_macs_setted_pctrl",
          "router_offline_list",
          "router_delete_offline_by_mac",
          "router_set_alg_switch",
          "router_set_dhcp_mode",
          "router_set_static_mode",
          "router_get_wifi_isolate",
          "router_set_wifi_isolate",
          "router_get_syslog",
          "router_webui_catchlog_check_key",
          "router_set_syslog",
          "router_webui_catch_tcpdump",
          "router_webui_catch_modemlog",
          "router_set_ipa_switch",
          "router_set_lan_vlan_mpdn_mapping"
        ],
        "zwrt_data": [
          "get_wwandst_monthlimit",
          "get_wwandst",
          "get_wwaniface",
          "set_wwaniface",
          "get_wwandst_clearday",
          "set_wwandst_monthlimit",
          "set_wwandst_clearday",
          "set_wwandst_calibmonth"
        ],
        "zwrt_zte_mdm.api": [
          "get_sim_info",
          "get_sim_info_before",
          "sim_verify_pin_puk",
          "sim_change_pin_mode",
          "sim_change_pin",
          "set_simlock_nck",
          "get_simlock_available_trials"
        ],
        "zte_nwinfo_api": [
          "nwinfo_get_netinfo",
          "nwinfo_set_netselect",
          "nwinfo_manual_scan",
          "nwinfo_m_netselect_status",
          "nwinfo_m_netselect_contents",
          "nwinfo_manual_register",
          "nwinfo_m_netselect_result",
          "nwinfo_start_detect_signal_quality",
          "nwinfo_end_detect_signal_quality",
          "nwinfo_get_progress_and_quality",
          "nwinfo_add_item_signal_quality",
          "nwinfo_modify_item_signal_quality",
          "nwinfo_delete_item_signal_quality",
          "nwinfo_set_lte_ext_band",
          "nwinfo_lock_lte_cell",
          "nwinfo_set_nrbandlock",
          "nwinfo_lock_nr_cell",
          "nwinfo_set_odu_as_mode",
          "nwinfo_reset_band_cell_setting",
          "nwinfo_set_nr5g_sa"
        ],
        "zwrt_wms": [
          "zwrt_get_wms_nvitems",
          "zwrt_wms_get_wms_capacity",
          "zte_libwms_get_sms_data",
          "zte_libwms_send_sms",
          "zwrt_wms_write_sms",
          "zwrt_wms_delete_sms",
          "zwrt_wms_get_cmd_status",
          "zwrt_wms_modify_tag",
          "zwrt_wms_get_status_rpt_data",
          "zte_wms_get_parameter",
          "zte_wms_set_parameter"
        ],
        "zwrt_led": [
          "get_ODU_switch_state",
          "set_ODU_switch_state"
        ],
        "zwrt_zte_dm": [
          "get_update_info",
          "confirm_download",
          "cancel_download",
          "check_new_version",
          "set_update_mode"
        ],
        "zwrt_web": [
          "web_privacy_read_flag_get",
          "web_quick_settings_init_flag_set",
          "web_language_get",
          "web_language_set",
          "web_login_info",
          "web_crt_get",
          "web_http_enstr_set",
          "web_login",
          "web_logout",
          "web_change_password",
          "web_privacy_read_flag_set",
          "web_info",
          "web_developer_option_login",
          "web_quick_settings_init_flag_get"
        ],
        "zwrt_apn_object": [
          "getAutoApnList",
          "getManuApnList",
          "get_apn_mode",
          "getApnAtCid",
          "delete_manu_apn",
          "set_apn_mode",
          "enable_manu_apn_id",
          "addManuApn",
          "modifyManuApn",
          "get_apn_at_cid",
          "set_apn_at_cid"
        ],
        "zwrt_mc.device.manager": [
          "device_reset",
          "set_device_info",
          "device_reboot",
          "device_poweroff",
          "device_backup_proc",
          "device_restore_proc",
          "get_device_info"
        ],
        "zwrt_sntp": [
          "get_systime",
          "set_manual_time",
          "set_auto_time",
          "get_systime_mode",
          "get_sync_state"
        ],
        "zwrt_tr069.api": [
          "get_acsinfo",
          "acsinfo_change"
        ],
        "zwrt_fota_res.api": [
          "get_update_result",
          "start_update"
        ],
        "zwrt_upd.api": [
          "clear_update_result"
        ],
        "zwrt_bsp.thermal": [
          "get_policy",
          "set_policy"
        ],
        "zwrt_tunnel.config": [
          "get"
        ],
        "zwrt_tunnel.pptp.config": [
          "set"
        ],
        "zwrt_tunnel.l2tp.config": [
          "set"
        ],
        "zwrt_tunnel.pptp": [
          "handle"
        ],
        "zwrt_tunnel.l2tp": [
          "handle"
        ]
      },
      "uci": [
        "zte_*",
        "zwrt_*",
        "firewall",
        "dhcp",
        "network",
        "wireless",
        "upnpd"
      ]
    },
    "write": {
      "ubus": {
        "zwrt_router.api": [
          "router_get_status",
          "router_get_user_list_num",
          "router_get_status_no_auth",
          "router_wireless_access_list",
          "router_lan_access_list",
          "router_set_lan_para",
          "router_set_wan_mtu",
          "router_set_public_ip",
          "router_set_remote_acl",
          "router_set_portforward",
          "router_set_portforward_switch",
          "router_set_macipport_filter_switch",
          "router_set_macipport_filter",
          "router_set_upnp_switch",
          "router_set_dmz",
          "router_set_portmapping",
          "router_set_portmapping_switch",
          "router_get_wan_mode_para",
          "router_set_cable_param_bak",
          "router_set_wan_mode",
          "router_set_pppoe_mode",
          "router_set_domain_filter",
          "router_get_pctrl_by_mac",
          "router_set_pctrl",
          "router_delete_pctrl_by_mac",
          "router_get_modified_lan_hostname",
          "router_modify_lan_hostname",
          "router_set_mac_ip_bind",
          "router_set_mac_ip_bond_switch",
          "router_after_sales_save_log",
          "router_set_ping_diagnose",
          "router_set_traceroute_diagnose",
          "router_set_watchdog",
          "router_set_nat_switch",
          "router_get_ddns",
          "router_set_ddns",
          "router_get_macs_setted_pctrl",
          "router_offline_list",
          "router_delete_offline_by_mac",
          "router_set_alg_switch",
          "router_set_dhcp_mode",
          "router_set_static_mode",
          "router_get_wifi_isolate",
          "router_set_wifi_isolate",
          "router_get_syslog",
          "router_webui_catchlog_check_key",
          "router_set_syslog",
          "router_webui_catch_tcpdump",
          "router_webui_catch_modemlog",
          "router_set_ipa_switch",
          "router_set_lan_vlan_mpdn_mapping"
        ],
        "zwrt_data": [
          "get_wwandst_monthlimit",
          "get_wwandst",
          "get_wwaniface",
          "set_wwaniface",
          "get_wwandst_clearday",
          "set_wwandst_monthlimit",
          "set_wwandst_clearday",
          "set_wwandst_calibmonth"
        ],
        "zwrt_zte_mdm.api": [
          "get_sim_info",
          "get_sim_info_before",
          "sim_verify_pin_puk",
          "sim_change_pin_mode",
          "sim_change_pin",
          "set_simlock_nck",
          "get_simlock_available_trials"
        ],
        "zte_nwinfo_api": [
          "nwinfo_get_netinfo",
          "nwinfo_set_netselect",
          "nwinfo_manual_scan",
          "nwinfo_m_netselect_status",
          "nwinfo_m_netselect_contents",
          "nwinfo_manual_register",
          "nwinfo_m_netselect_result",
          "nwinfo_start_detect_signal_quality",
          "nwinfo_end_detect_signal_quality",
          "nwinfo_get_progress_and_quality",
          "nwinfo_add_item_signal_quality",
          "nwinfo_modify_item_signal_quality",
          "nwinfo_delete_item_signal_quality",
          "nwinfo_set_lte_ext_band",
          "nwinfo_lock_lte_cell",
          "nwinfo_set_nrbandlock",
          "nwinfo_lock_nr_cell",
          "nwinfo_set_odu_as_mode",
          "nwinfo_reset_band_cell_setting",
          "nwinfo_set_nr5g_sa"
        ],
        "zwrt_wms": [
          "zwrt_get_wms_nvitems",
          "zwrt_wms_get_wms_capacity",
          "zte_libwms_get_sms_data",
          "zte_libwms_send_sms",
          "zwrt_wms_write_sms",
          "zwrt_wms_delete_sms",
          "zwrt_wms_get_cmd_status",
          "zwrt_wms_modify_tag",
          "zwrt_wms_get_status_rpt_data",
          "zte_wms_get_parameter",
          "zte_wms_set_parameter"
        ],
        "zwrt_led": [
          "get_ODU_switch_state",
          "set_ODU_switch_state"
        ],
        "zwrt_zte_dm": [
          "get_update_info",
          "confirm_download",
          "cancel_download",
          "check_new_version",
          "set_update_mode"
        ],
        "zwrt_web": [
          "web_privacy_read_flag_get",
          "web_quick_settings_init_flag_set",
          "web_language_get",
          "web_language_set",
          "web_login_info",
          "web_crt_get",
          "web_http_enstr_set",
          "web_login",
          "web_logout",
          "web_change_password",
          "web_privacy_read_flag_set",
          "web_info",
          "web_developer_option_login",
          "web_quick_settings_init_flag_get"
        ],
        "zwrt_apn_object": [
          "getAutoApnList",
          "getManuApnList",
          "get_apn_mode",
          "getApnAtCid",
          "delete_manu_apn",
          "set_apn_mode",
          "enable_manu_apn_id",
          "addManuApn",
          "modifyManuApn",
          "get_apn_at_cid",
          "set_apn_at_cid"
        ],
        "zwrt_mc.device.manager": [
          "device_reset",
          "set_device_info",
          "device_reboot",
          "device_poweroff",
          "device_backup_proc",
          "device_restore_proc",
          "get_device_info"
        ],
        "zwrt_sntp": [
          "get_systime",
          "set_manual_time",
          "set_auto_time",
          "get_systime_mode",
          "get_sync_state"
        ],
        "zwrt_tr069.api": [
          "get_acsinfo",
          "acsinfo_change"
        ],
        "zwrt_fota_res.api": [
          "get_update_result",
          "start_update"
        ],
        "zwrt_upd.api": [
          "clear_update_result"
        ],

        "zwrt_bsp.thermal": [
          "get_policy",
          "set_policy"
        ],
        "zwrt_tunnel.config": [
          "get"
        ],
        "zwrt_tunnel.pptp.config": [
          "set"
        ],
        "zwrt_tunnel.l2tp.config": [
          "set"
        ],
        "zwrt_tunnel.pptp": [
          "handle"
        ],
        "zwrt_tunnel.l2tp": [
          "handle"
        ]
      }
    }
  }
}