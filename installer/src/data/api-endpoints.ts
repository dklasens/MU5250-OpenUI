export interface APICategory {
  name: string;
  count: number;
  capabilities: string;
  endpoints?: APIEndpoint[];
}

export interface APIEndpoint {
  method: string;
  path: string;
  description: string;
}

export const apiCategories: APICategory[] = [
  {
    name: "Auth",
    count: 1,
    capabilities: "Token-based authentication",
    endpoints: [
      { method: "POST", path: "/api/auth/login", description: "Authenticate and obtain session token" },
    ],
  },
  {
    name: "Device",
    count: 15,
    capabilities:
      "System info, battery, CPU, memory, thermal, charger, reboot, shutdown, factory reset, power save, fast boot",
    endpoints: [
      { method: "GET", path: "/api/device", description: "System information (sysfs)" },
      { method: "GET", path: "/api/battery", description: "Battery status and percentage" },
      { method: "GET", path: "/api/cpu", description: "CPU usage and frequency" },
      { method: "GET", path: "/api/memory", description: "Memory usage statistics" },
      { method: "GET", path: "/api/device/battery-info", description: "Battery details (ubus)" },
      { method: "GET", path: "/api/device/thermal", description: "Temperature sensors" },
      { method: "GET", path: "/api/device/charger", description: "Charger status" },
      { method: "GET", path: "/api/device/system", description: "System details (ubus)" },
      { method: "POST", path: "/api/device/reboot", description: "Reboot device" },
      { method: "POST", path: "/api/device/shutdown", description: "Shut down device" },
      { method: "POST", path: "/api/device/factory-reset", description: "Factory reset" },
      { method: "POST", path: "/api/device/power-save", description: "Power save mode status" },
      { method: "PUT", path: "/api/device/power-save", description: "Toggle power save" },
      { method: "GET", path: "/api/device/fast-boot", description: "Fast boot status" },
      { method: "PUT", path: "/api/device/fast-boot", description: "Toggle fast boot" },
    ],
  },
  {
    name: "Network",
    count: 9,
    capabilities:
      "Signal strength (RSRP/SINR/RSRQ), speed, traffic stats, WAN/LAN status, rmnet, connected clients",
    endpoints: [
      { method: "GET", path: "/api/network/signal", description: "Signal strength metrics" },
      { method: "GET", path: "/api/network/traffic", description: "Traffic statistics" },
      { method: "GET", path: "/api/network/speed", description: "Current speed" },
      { method: "GET", path: "/api/network/wan", description: "WAN IPv4 status" },
      { method: "GET", path: "/api/network/wan6", description: "WAN IPv6 status" },
      { method: "GET", path: "/api/network/lan-status", description: "LAN status" },
      { method: "GET", path: "/api/network/clients", description: "Connected clients" },
      { method: "GET", path: "/api/network/speeds", description: "Speed history" },
      { method: "GET", path: "/api/network/rmnet", description: "rmnet interface info" },
    ],
  },
  {
    name: "Modem",
    count: 13,
    capabilities:
      "Airplane mode, mobile data toggle, data usage reset day, network mode (2G–5G), operator scan, manual registration",
    endpoints: [
      { method: "GET", path: "/api/data-usage", description: "Data usage statistics" },
      { method: "PUT", path: "/api/data-usage/reset-day", description: "Set data usage reset day" },
      { method: "GET", path: "/api/modem/status", description: "Modem status" },
      { method: "POST", path: "/api/modem/online", description: "Go online/offline" },
      { method: "GET", path: "/api/modem/data", description: "Mobile data status" },
      { method: "PUT", path: "/api/modem/data", description: "Toggle mobile data" },
      { method: "POST", path: "/api/modem/airplane", description: "Toggle airplane mode" },
      { method: "PUT", path: "/api/modem/network-mode", description: "Set network mode (2G–5G)" },
      { method: "POST", path: "/api/modem/scan", description: "Start operator scan" },
      { method: "GET", path: "/api/modem/scan/status", description: "Operator scan status" },
      { method: "GET", path: "/api/modem/scan/results", description: "Operator scan results" },
      { method: "POST", path: "/api/modem/register", description: "Manual network registration" },
      { method: "GET", path: "/api/modem/register/result", description: "Registration result" },
    ],
  },
  {
    name: "SMS",
    count: 14,
    capabilities:
      "List, send, delete, mark read, storage capacity, SMS forwarding config/rules/toggle/test/log",
    endpoints: [
      { method: "POST", path: "/api/sms/list", description: "List SMS messages" },
      { method: "GET", path: "/api/sms/capacity", description: "SMS storage capacity" },
      { method: "POST", path: "/api/sms/send", description: "Send SMS" },
      { method: "POST", path: "/api/sms/delete", description: "Delete SMS" },
      { method: "POST", path: "/api/sms/read", description: "Mark SMS as read" },
      { method: "GET", path: "/api/sms/forward/config", description: "Forwarding configuration" },
      { method: "PUT", path: "/api/sms/forward/config", description: "Update forwarding config" },
      { method: "POST", path: "/api/sms/forward/rules", description: "Create forwarding rule" },
      { method: "PUT", path: "/api/sms/forward/rules", description: "Update forwarding rule" },
      { method: "DELETE", path: "/api/sms/forward/rules", description: "Delete forwarding rule" },
      { method: "PUT", path: "/api/sms/forward/rules/toggle", description: "Toggle forwarding rule" },
      { method: "POST", path: "/api/sms/forward/test", description: "Test forwarding" },
      { method: "GET", path: "/api/sms/forward/log", description: "Forwarding log" },
      { method: "POST", path: "/api/sms/forward/log/clear", description: "Clear forwarding log" },
    ],
  },
  {
    name: "SIM",
    count: 7,
    capabilities: "SIM info, IMEI, PIN management, PUK unlock",
    endpoints: [
      { method: "GET", path: "/api/sim/info", description: "SIM card information" },
      { method: "GET", path: "/api/sim/imei", description: "Device IMEI" },
      { method: "POST", path: "/api/sim/pin/verify", description: "Verify SIM PIN" },
      { method: "POST", path: "/api/sim/pin/change", description: "Change SIM PIN" },
      { method: "POST", path: "/api/sim/pin/mode", description: "Toggle PIN lock mode" },
      { method: "POST", path: "/api/sim/unlock", description: "PUK unlock" },
      { method: "GET", path: "/api/sim/lock-trials", description: "Remaining PIN/PUK attempts" },
    ],
  },
  {
    name: "Cell/Band",
    count: 19,
    capabilities:
      "NR/LTE band locking, cell locking, neighbor scan, STC, signal quality detection",
    endpoints: [
      { method: "POST", path: "/api/cell/lock/nr", description: "Lock NR cell" },
      { method: "POST", path: "/api/cell/lock/lte", description: "Lock LTE cell" },
      { method: "POST", path: "/api/cell/lock/reset", description: "Reset cell locks" },
      { method: "POST", path: "/api/cell/neighbors/scan", description: "Scan neighbor cells" },
      { method: "GET", path: "/api/cell/neighbors/nr", description: "NR neighbor cells" },
      { method: "GET", path: "/api/cell/neighbors/lte", description: "LTE neighbor cells" },
      { method: "POST", path: "/api/cell/band/nr", description: "Lock NR bands" },
      { method: "POST", path: "/api/cell/band/lte", description: "Lock LTE bands" },
      { method: "POST", path: "/api/cell/band/reset", description: "Reset band locks" },
      { method: "GET", path: "/api/cell/stc/params", description: "STC parameters" },
      { method: "PUT", path: "/api/cell/stc/params", description: "Set STC parameters" },
      { method: "GET", path: "/api/cell/stc/status", description: "STC status" },
      { method: "POST", path: "/api/cell/stc/enable", description: "Enable STC" },
      { method: "POST", path: "/api/cell/stc/disable", description: "Disable STC" },
      { method: "POST", path: "/api/cell/stc/reset", description: "Reset STC" },
      { method: "POST", path: "/api/cell/signal-detect/start", description: "Start signal detection" },
      { method: "POST", path: "/api/cell/signal-detect/stop", description: "Stop signal detection" },
      { method: "GET", path: "/api/cell/signal-detect/results", description: "Signal detection results" },
      { method: "GET", path: "/api/cell/signal-detect/progress", description: "Signal detection progress" },
    ],
  },
  {
    name: "Router",
    count: 29,
    capabilities:
      "DNS, LAN/DHCP, firewall, NAT, DMZ, UPnP, port forwarding, QoS, domain filter, APN profiles, VPN/ALG",
    endpoints: [
      { method: "GET", path: "/api/router/dns", description: "DNS settings" },
      { method: "PUT", path: "/api/router/dns", description: "Update DNS settings" },
      { method: "GET", path: "/api/router/lan", description: "LAN/DHCP settings" },
      { method: "PUT", path: "/api/router/lan", description: "Update LAN settings" },
      { method: "GET", path: "/api/router/firewall", description: "Firewall status" },
      { method: "PUT", path: "/api/router/firewall/switch", description: "Toggle firewall" },
      { method: "PUT", path: "/api/router/firewall/level", description: "Set firewall level" },
      { method: "PUT", path: "/api/router/firewall/nat", description: "Configure NAT" },
      { method: "PUT", path: "/api/router/firewall/dmz", description: "Configure DMZ" },
      { method: "GET", path: "/api/router/firewall/upnp", description: "UPnP status" },
      { method: "PUT", path: "/api/router/firewall/upnp", description: "Configure UPnP" },
      { method: "GET", path: "/api/router/firewall/port-forward", description: "Port forwarding rules" },
      { method: "POST", path: "/api/router/firewall/port-forward", description: "Add port forwarding rule" },
      { method: "PUT", path: "/api/router/firewall/port-forward/switch", description: "Toggle port forwarding" },
      { method: "GET", path: "/api/router/firewall/filter-rules", description: "Firewall filter rules" },
      { method: "GET", path: "/api/router/vpn", description: "VPN passthrough settings" },
      { method: "PUT", path: "/api/router/vpn", description: "Update VPN passthrough" },
      { method: "GET", path: "/api/router/qos", description: "QoS settings" },
      { method: "PUT", path: "/api/router/qos", description: "Update QoS settings" },
      { method: "GET", path: "/api/router/domain-filter", description: "Domain filter settings" },
      { method: "PUT", path: "/api/router/domain-filter", description: "Update domain filter" },
      { method: "GET", path: "/api/router/apn/mode", description: "APN mode" },
      { method: "PUT", path: "/api/router/apn/mode", description: "Set APN mode" },
      { method: "GET", path: "/api/router/apn/profiles", description: "APN profiles" },
      { method: "POST", path: "/api/router/apn/profiles", description: "Add APN profile" },
      { method: "PUT", path: "/api/router/apn/profiles", description: "Modify APN profile" },
      { method: "GET", path: "/api/router/apn/auto-profiles", description: "Auto-detected APN profiles" },
      { method: "POST", path: "/api/router/apn/profiles/delete", description: "Delete APN profile" },
      { method: "POST", path: "/api/router/apn/profiles/activate", description: "Activate APN profile" },
    ],
  },
  {
    name: "WiFi",
    count: 4,
    capabilities: "Status, SSID/password/channel/TX power, guest WiFi",
    endpoints: [
      { method: "GET", path: "/api/wifi/status", description: "WiFi status and info" },
      { method: "PUT", path: "/api/wifi/settings", description: "Update WiFi settings" },
      { method: "GET", path: "/api/wifi/guest", description: "Guest WiFi status" },
      { method: "PUT", path: "/api/wifi/guest", description: "Update guest WiFi" },
    ],
  },
  {
    name: "USB",
    count: 2,
    capabilities: "USB mode switching",
    endpoints: [
      { method: "GET", path: "/api/usb/status", description: "USB port status" },
      { method: "PUT", path: "/api/usb/mode", description: "Set USB mode" },
    ],
  },
  {
    name: "Telephony",
    count: 11,
    capabilities:
      "Voice calls (dial/hangup/answer/DTMF/mute), USSD codes, SIM Toolkit menus",
    endpoints: [
      { method: "POST", path: "/api/call/dial", description: "Dial a number" },
      { method: "POST", path: "/api/call/hangup", description: "Hang up call" },
      { method: "POST", path: "/api/call/answer", description: "Answer incoming call" },
      { method: "GET", path: "/api/call/status", description: "Call status" },
      { method: "POST", path: "/api/call/dtmf", description: "Send DTMF tone" },
      { method: "POST", path: "/api/call/mute", description: "Toggle mute" },
      { method: "POST", path: "/api/ussd/send", description: "Send USSD code" },
      { method: "POST", path: "/api/ussd/respond", description: "Respond to USSD prompt" },
      { method: "POST", path: "/api/ussd/cancel", description: "Cancel USSD session" },
      { method: "GET", path: "/api/stk/menu", description: "SIM Toolkit menu" },
      { method: "POST", path: "/api/stk/select", description: "Select STK menu item" },
    ],
  },
  {
    name: "Speed Test",
    count: 4,
    capabilities: "Server list, run test, progress tracking",
    endpoints: [
      { method: "GET", path: "/api/speedtest/servers", description: "Available speed test servers" },
      { method: "POST", path: "/api/speedtest/start", description: "Start speed test" },
      { method: "GET", path: "/api/speedtest/progress", description: "Speed test progress" },
      { method: "POST", path: "/api/speedtest/stop", description: "Stop speed test" },
    ],
  },
  {
    name: "DoH Proxy",
    count: 6,
    capabilities: "DNS-over-HTTPS proxy, cache management",
    endpoints: [
      { method: "GET", path: "/api/doh/status", description: "DoH proxy status" },
      { method: "PUT", path: "/api/doh/config", description: "Update DoH configuration" },
      { method: "POST", path: "/api/doh/enable", description: "Enable DoH proxy" },
      { method: "POST", path: "/api/doh/disable", description: "Disable DoH proxy" },
      { method: "GET", path: "/api/doh/cache", description: "DNS cache entries" },
      { method: "POST", path: "/api/doh/cache/clear", description: "Clear DNS cache" },
    ],
  },
  {
    name: "LAN Test",
    count: 3,
    capabilities: "WiFi ping, download/upload throughput measurement",
    endpoints: [
      { method: "GET", path: "/api/lan/ping", description: "LAN ping test" },
      { method: "GET", path: "/api/lan/download", description: "Download throughput test" },
      { method: "POST", path: "/api/lan/upload", description: "Upload throughput test" },
    ],
  },
  {
    name: "Scheduler",
    count: 5,
    capabilities: "Scheduled/recurring jobs for any API action",
    endpoints: [
      { method: "GET", path: "/api/scheduler/jobs", description: "List scheduled jobs" },
      { method: "POST", path: "/api/scheduler/jobs", description: "Create scheduled job" },
      { method: "PUT", path: "/api/scheduler/jobs", description: "Update scheduled job" },
      { method: "DELETE", path: "/api/scheduler/jobs", description: "Delete scheduled job" },
      { method: "PUT", path: "/api/scheduler/jobs/toggle", description: "Toggle job enabled" },
    ],
  },
];

export const totalEndpoints = apiCategories.reduce(
  (sum, cat) => sum + cat.count,
  0
);
export const totalCategories = apiCategories.length;
