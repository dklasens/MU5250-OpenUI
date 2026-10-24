"""The mock agent must validate like the agent and reproduce cross-resource read-back.

PLAN2 A02.7: a blanket `{ok:true,data:{}}` is not evidence. Each test performs a
mutation over HTTP against a real mock server on an ephemeral port, then reads
the *other* resource the real firmware would change.
"""
import http.client
import importlib.util
import json
from pathlib import Path
import re
import threading
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('mock_agent', ROOT / 'web-app/tools/mock_agent.py')
mock = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mock)

DEFAULT_NR = '1,2,3,5,7,8,18,20,26,28,29,38,40,41,48,66,71,75,77,78,79'


class MockCase(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        mock.CONFIG['jitter'] = False
        mock.CONFIG['usb_switch_settle_s'] = 0.4
        cls.server = mock.make_server('127.0.0.1', 0)
        cls.port = cls.server.server_address[1]
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join(timeout=5)

    def setUp(self):
        self.call('POST', '/__mock/reset', {'scenario': 'SA'})

    # -- helpers -------------------------------------------------------------

    def raw(self, method, path, body=None, headers=None):
        conn = http.client.HTTPConnection('127.0.0.1', self.port, timeout=10)
        try:
            payload = None if body is None else json.dumps(body)
            hdrs = {'Content-Type': 'application/json', **(headers or {})}
            conn.request(method, path, payload, hdrs)
            resp = conn.getresponse()
            return resp.status, resp.getheader('Content-Type'), resp.read()
        finally:
            conn.close()

    def call(self, method, path, body=None, headers=None):
        status, _, data = self.raw(method, path, body, headers)
        return status, json.loads(data)

    def data(self, method, path, body=None, expect=200, headers=None):
        status, envelope = self.call(method, path, body, headers)
        self.assertEqual(status, expect, envelope)
        self.assertTrue(envelope['ok'], envelope)
        return envelope.get('data')

    def error(self, method, path, body=None, expect=400):
        status, envelope = self.call(method, path, body)
        self.assertEqual(status, expect, envelope)
        self.assertIs(envelope['ok'], False)
        self.assertIsInstance(envelope['error'], str)
        return envelope['error']

    def signal(self):
        return self.data('GET', '/api/dashboard')['signal']

    def recorded(self):
        return self.data('GET', '/__mock/requests')['requests']


class FixtureShapes(MockCase):
    def test_sa_netinfo_matches_the_observed_unit(self):
        s = self.signal()
        for key, value in {
            'network_type': 'SA', 'net_select': 'Only_5G', 'signalbar': '5', 'nr5g_pci': 745,
            'nr5g_action_channel': 643392, 'nr5g_action_band': 'n78', 'nr5g_rsrp': -53,
            'nr5g_rsrq': -11, 'nr5g_snr': '31.0', 'lte_rsrp': -48, 'lock_lte_cell': '',
            'lock_nr_cell': '', 'lte_band_lock': '0x87e29a0e00df',
            'nr5g_sa_band_lock': DEFAULT_NR, 'nr5g_nsa_band_lock': DEFAULT_NR,
        }.items():
            self.assertEqual(s[key], value, key)

    def test_data_usage_uses_agent_encodings(self):
        usage = self.data('GET', '/api/dashboard')['data_usage']
        self.assertEqual(usage['reset_enabled'], 1)
        self.assertIsInstance(usage['reset_enabled'], int)
        self.assertNotIsInstance(usage['reset_enabled'], bool)
        self.assertEqual(usage['reset_day'], 16)
        self.assertRegex(usage['clear_date_record'], r'^\d{4}/\d{2}/\d{2}$')
        self.assertRegex(usage['next_clear_date'], r'^\d{8}$')
        self.assertEqual(set(usage['day']), {'rx_bytes', 'tx_bytes', 'time_secs', 'rx_packets', 'tx_packets'})

    def test_dashboard_carries_source_freshness(self):
        batch = self.data('GET', '/api/dashboard')
        self.assertEqual(set(batch['sources']), {'signal', 'wan', 'wan6', 'thermal', 'data_usage', 'speed'})
        self.assertFalse(batch['sources']['signal']['stale'])
        self.assertIsNone(batch['charge_control_error'])

    def test_wifi_status_reports_wifi7_widths_and_power(self):
        wifi = self.data('GET', '/api/wifi/status')
        self.assertEqual((wifi['htmode_2g'], wifi['htmode_5g']), ('EHT40', 'EHT80'))
        self.assertEqual((wifi['txpower_2g'], wifi['txpower_5g']), ('80', '80'))

    def test_scenarios(self):
        expected = {
            'NSA': ('ENDC', 'LTE_AND_5G'),
            'LTE': ('LTE', 'WCDMA_AND_LTE'),
            'disconnected': ('No Service', 'WL_AND_5G'),
        }
        for scenario, (network_type, net_select) in expected.items():
            self.call('POST', '/__mock/reset', {'scenario': scenario})
            batch = self.data('GET', '/api/dashboard')
            self.assertEqual(batch['signal']['network_type'], network_type)
            self.assertEqual(batch['signal']['net_select'], net_select)
            self.assertEqual(batch['wan']['up'], scenario != 'disconnected')
        status, envelope = self.call('POST', '/__mock/reset', {'scenario': 'bogus'})
        self.assertEqual(status, 400)
        self.assertFalse(envelope['ok'])

    def test_unknown_routes_are_404_like_the_agent(self):
        self.assertEqual(self.error('GET', '/api/nope', expect=404), 'not found')

    def test_every_agent_route_is_served(self):
        server_rs = (ROOT / 'agent/src/server.rs').read_text()
        routes = re.findall(r'\(&Method::(\w+),\s*"(/api/[^"]+)"\)', server_rs)
        self.assertGreater(len(routes), 40)
        for method, path in routes:
            status, _, _ = self.raw(method.upper(), path, {} if method != 'Get' else None)
            self.assertNotEqual(status, 404, f'{method} {path}')
            self.call('POST', '/__mock/reset')  # keep one route from affecting the next


class ApnReadBack(MockCase):
    def profiles(self):
        return {str(p['profileId']): p for p in self.data('GET', '/api/router/apn/profiles')['apnListArray']}

    def test_activate_switches_to_manual_and_enables_the_profile(self):
        self.data('PUT', '/api/router/apn/mode', {'apn_mode': 0})
        self.assertEqual(self.data('GET', '/api/router/apn/mode'), {'apn_mode': 0})
        self.data('POST', '/api/router/apn/profiles/activate', {'profileId': '2'})
        self.assertEqual(self.data('GET', '/api/router/apn/mode'), {'apn_mode': 1})
        profiles = self.profiles()
        self.assertEqual(str(profiles['2']['isEnable']), '1')
        self.assertEqual(str(profiles['1']['isEnable']), '0')
        last = self.recorded()[-1]
        self.assertEqual((last['method'], last['path'], last['body'], last['status']),
                         ('POST', '/api/router/apn/profiles/activate', {'profileId': '2'}, 200))

    def test_failed_activation_restores_automatic_mode(self):
        self.data('PUT', '/api/router/apn/mode', {'apn_mode': 0})
        self.error('POST', '/api/router/apn/profiles/activate', {'profileId': '99'}, expect=503)
        self.assertEqual(self.data('GET', '/api/router/apn/mode'), {'apn_mode': 0})

    def test_mode_validation(self):
        self.assertIn('apn_mode must be', self.error('PUT', '/api/router/apn/mode', {'apn_mode': 2}))
        self.error('PUT', '/api/router/apn/mode', {'apn_mode': 1, 'extra': True})
        self.assertEqual(self.data('GET', '/api/router/apn/mode'), {'apn_mode': 1})

    def test_add_and_delete_change_the_list(self):
        self.data('POST', '/api/router/apn/profiles', {
            'profilename': 'Lab', 'wanapn': 'lab.example', 'pdpType': 3, 'pppAuthMode': 0})
        profiles = self.profiles()
        self.assertEqual(len(profiles), 3)
        new_id = next(i for i, p in profiles.items() if p['profilename'] == 'Lab')
        self.assertEqual(str(profiles[new_id]['isEnable']), '0')
        self.data('POST', '/api/router/apn/profiles/delete', {'profileId': new_id})
        self.assertNotIn(new_id, self.profiles())

    def test_the_active_profile_cannot_be_deleted(self):
        self.assertEqual(self.error('POST', '/api/router/apn/profiles/delete', {'profileId': '1'}, expect=409),
                         'cannot delete the active APN profile')
        self.assertIn('1', self.profiles())

    def test_add_validation(self):
        base = {'profilename': 'x', 'wanapn': 'y', 'pdpType': 3, 'pppAuthMode': 0}
        self.error('POST', '/api/router/apn/profiles', {**base, 'pdpType': 4})
        self.error('POST', '/api/router/apn/profiles', {**base, 'username': 'u'})  # creds need an auth mode
        self.error('POST', '/api/router/apn/profiles', {**base, 'profilename': ' '})
        self.assertEqual(len(self.profiles()), 2)


class WifiReadBack(MockCase):
    def test_tx_power_and_width_appear_on_the_next_read(self):
        result = self.data('PUT', '/api/wifi/settings', {'txpower_2g': 40, 'htmode_5g': 'EHT160'})
        self.assertEqual(result, {'status': 'ok', 'changed': True})
        wifi = self.data('GET', '/api/wifi/status')
        self.assertEqual(wifi['txpower_2g'], '40')
        self.assertEqual(wifi['txpower_5g'], '80')
        self.assertEqual((wifi['htmode_5g'], wifi['actual_bw_5g']), ('EHT160', '160 MHz'))
        self.assertEqual(self.data('PUT', '/api/wifi/settings', {'txpower_2g': '40'})['changed'], False)

    def test_invalid_values_are_rejected_and_change_nothing(self):
        before = self.data('GET', '/api/wifi/status')
        for bad in (0, 101, 'abc', -1, 3.5, None):
            self.error('PUT', '/api/wifi/settings', {'txpower_2g': bad})
        # A request is all-or-nothing: the valid half must not leak through.
        self.error('PUT', '/api/wifi/settings', {'ssid_2g': 'Renamed', 'txpower_5g': 101})
        self.error('PUT', '/api/wifi/settings', {'htmode_2g': 'HT40'})
        self.error('PUT', '/api/wifi/settings', {'channel_2g': 36})
        self.error('PUT', '/api/wifi/settings', {'key_5g': 'short'})
        self.error('PUT', '/api/wifi/settings', {'bogus': 1})
        self.error('PUT', '/api/wifi/settings', {})
        self.assertEqual(self.data('GET', '/api/wifi/status'), before)

    def test_ssid_key_and_channel(self):
        self.data('PUT', '/api/wifi/settings', {'ssid_5g': 'Lab5', 'key_5g': 'new-passphrase-1', 'channel_5g': '149'})
        wifi = self.data('GET', '/api/wifi/status')
        self.assertEqual((wifi['ssid_5g'], wifi['key_5g'], wifi['actual_channel_5g']), ('Lab5', 'new-passphrase-1', 149))
        # Echoing the dashboard's mask must not overwrite the stored key.
        self.data('PUT', '/api/wifi/settings', {'key_5g': '••••••••', 'hidden_5g': True})
        wifi = self.data('GET', '/api/wifi/status')
        self.assertEqual((wifi['key_5g'], wifi['hidden_5g']), ('new-passphrase-1', '1'))

    def test_invalid_json_envelope(self):
        status, _, raw = self.raw('PUT', '/api/wifi/settings')
        self.assertEqual(status, 400)
        self.assertFalse(json.loads(raw)['ok'])


class RadioReadBack(MockCase):
    def test_nr_band_lock_reads_back_in_the_sa_field_only(self):
        self.data('POST', '/api/cell/band/nr', {'nr5g_type': 'SA', 'nr5g_band': '41,78'})
        s = self.signal()
        self.assertEqual(s['nr5g_sa_band_lock'], '41,78')
        self.assertEqual(s['nr5g_nsa_band_lock'], DEFAULT_NR)
        self.assertEqual(s['lte_band_lock'], '0x87e29a0e00df')

    def test_nr_band_validation(self):
        for body in ({'nr5g_type': 'NSA', 'nr5g_band': '78'}, {'nr5g_type': 'SA', 'nr5g_band': '12'},
                     {'nr5g_type': 'SA', 'nr5g_band': '78,78'}, {'nr5g_type': 'SA', 'nr5g_band': ''},
                     {'nr5g_type': 'SA', 'nr5g_band': 'x'}, {'nr5g_type': 'SA'}):
            self.error('POST', '/api/cell/band/nr', body)
        self.assertEqual(self.signal()['nr5g_sa_band_lock'], DEFAULT_NR)

    def test_lte_band_lock_reads_back_as_hex(self):
        mask = (1 << 2) | (1 << 27)  # B3 + B28
        body = {'is_lte_band': '1', 'lte_band_mask': str(mask), 'is_gw_band': '0', 'gw_band_mask': '0'}
        self.data('POST', '/api/cell/band/lte', body)
        self.assertEqual(self.signal()['lte_band_lock'], '0x8000004')
        self.error('POST', '/api/cell/band/lte', {**body, 'lte_band_mask': str(1 << 11)})  # B12: unsupported
        self.error('POST', '/api/cell/band/lte', {**body, 'lte_band_mask': '0'})
        self.error('POST', '/api/cell/band/lte', {**body, 'is_gw_band': '1'})
        self.assertEqual(self.signal()['lte_band_lock'], '0x8000004')

    def test_band_reset_restores_defaults(self):
        self.data('POST', '/api/cell/band/nr', {'nr5g_type': 'SA', 'nr5g_band': '78'})
        self.data('POST', '/api/cell/band/lte', {
            'is_lte_band': '1', 'lte_band_mask': '4', 'is_gw_band': '0', 'gw_band_mask': '0'})
        self.data('POST', '/api/cell/band/reset')
        s = self.signal()
        self.assertEqual((s['lte_band_lock'], s['nr5g_sa_band_lock'], s['nr5g_nsa_band_lock']),
                         ('0x87e29a0e00df', DEFAULT_NR, DEFAULT_NR))

    def test_cell_lock_is_recorded_but_does_not_move_the_serving_cell(self):
        before = self.signal()
        self.data('POST', '/api/cell/lock/nr',
                  {'lock_nr_pci': '123', 'lock_nr_earfcn': '630912', 'lock_nr_cell_band': '78'})
        self.data('POST', '/api/cell/lock/lte', {'lock_lte_pci': '312', 'lock_lte_earfcn': '3650'})
        after = self.signal()
        self.assertEqual(after['lock_nr_cell'], '123,630912,78')
        self.assertEqual(after['lock_lte_cell'], '312,3650')
        for key in ('nr5g_pci', 'nr5g_action_channel', 'nr5g_action_band', 'lte_pci', 'network_type'):
            self.assertEqual(after[key], before[key], key)
        self.assertEqual(self.signal()['nr5g_pci'], 745)
        self.data('POST', '/api/cell/lock/reset')
        after = self.signal()
        self.assertEqual((after['lock_nr_cell'], after['lock_lte_cell']), ('', ''))
        self.assertEqual(after['nr5g_pci'], 745)

    def test_lock_requests_are_recorded_with_their_bodies(self):
        body = {'lock_nr_pci': '123', 'lock_nr_earfcn': '630912', 'lock_nr_cell_band': '78'}
        self.data('POST', '/api/cell/lock/nr', body)
        self.error('POST', '/api/cell/lock/nr', {'lock_nr_pci': '1'}, expect=503)
        calls = [r for r in self.recorded() if r['path'] == '/api/cell/lock/nr']
        self.assertEqual([(c['body'], c['status']) for c in calls],
                         [(body, 200), ({'lock_nr_pci': '1'}, 503)])

    def test_network_mode(self):
        self.data('PUT', '/api/modem/network-mode', {'net_select': 'LTE_AND_5G'})
        s = self.signal()
        self.assertEqual(s['net_select'], 'LTE_AND_5G')
        self.assertEqual(s['network_type'], 'SA')  # re-registration is not modelled
        self.error('PUT', '/api/modem/network-mode', {'net_select': 'Only_6G'})
        self.assertEqual(self.signal()['net_select'], 'LTE_AND_5G')


class UsageReadBack(MockCase):
    def test_reset_day_enables_the_cycle_and_returns_the_payload(self):
        mock.STATE['usage']['reset_enabled'] = 0  # a unit whose reset was off
        self.assertEqual(self.data('GET', '/api/dashboard')['data_usage']['reset_enabled'], 0)
        usage = self.data('PUT', '/api/data-usage/reset-day', {'reset_day': 5})
        self.assertEqual((usage['reset_day'], usage['reset_enabled']), (5, 1))
        self.assertRegex(usage['next_clear_date'], r'^\d{6}05$')
        self.assertIn('rx_bytes', usage['day'])
        dashboard = self.data('GET', '/api/dashboard')['data_usage']
        self.assertEqual((dashboard['reset_day'], dashboard['reset_enabled']), (5, 1))
        self.assertEqual(dashboard['next_clear_date'], usage['next_clear_date'])

    def test_reset_day_validation(self):
        for body in ({'reset_day': 0}, {'reset_day': 32}, {'reset_day': '5'}, {'reset_day': 5.5}, {}):
            self.assertEqual(self.error('PUT', '/api/data-usage/reset-day', body),
                             'reset_day must be between 1 and 31')
        self.assertEqual(self.data('GET', '/api/dashboard')['data_usage']['reset_day'], 16)

    def test_cycle_dates_clamp_to_short_months(self):
        import datetime
        record, nxt = mock.cycle_dates(31, datetime.date(2026, 2, 10))
        self.assertEqual((record, nxt), ('2026/01/31', '20260228'))
        record, nxt = mock.cycle_dates(16, datetime.date(2026, 10, 3))
        self.assertEqual((record, nxt), ('2026/09/16', '20261016'))


class UsbReadBack(MockCase):
    def status(self):
        return self.data('GET', '/api/usb/status')

    def wait_for_mode(self, mode, timeout=5.0):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if self.status()['active_mode'] == mode:
                return
            time.sleep(0.05)
        self.fail(f'USB never became {mode}')

    def test_ncm_is_scheduled_then_becomes_active(self):
        result = self.data('PUT', '/api/usb/mode', {'mode': 'ncm', 'confirm_experimental': True}, expect=202)
        self.assertEqual(result['status'], 'scheduled')
        self.assertEqual((result['mode'], result['experimental'], result['delay_ms']), ('ncm', True, 1000))
        self.assertIn('rollback', result)
        before = self.status()
        self.assertEqual(before['active_mode'], 'ecm')  # not switched yet
        self.assertTrue(before['interfaces']['ecm0'])
        self.wait_for_mode('ncm')
        after = self.status()
        self.assertEqual(after['interfaces']['ncm_ifname'], 'ncm0')
        self.assertIn('ncm.0', after['composition_functions'])
        self.assertIn('ncm0', after['bridge']['members'])
        self.assertEqual(after['default_mode'], 'ecm')  # a live switch is not persisted

    def test_ncm_needs_confirmation_and_blocks_concurrent_switches(self):
        self.error('PUT', '/api/usb/mode', {'mode': 'ncm'})
        self.assertEqual(self.status()['active_mode'], 'ecm')
        self.data('PUT', '/api/usb/mode', {'mode': 'ncm', 'confirm_experimental': True}, expect=202)
        self.assertEqual(self.error('PUT', '/api/usb/mode', {'mode': 'rndis'}, expect=409),
                         'another USB change is in progress')

    def test_leaving_ncm_for_ecm_is_scheduled_too(self):
        self.data('PUT', '/api/usb/mode', {'mode': 'ncm', 'confirm_experimental': True}, expect=202)
        self.wait_for_mode('ncm')
        back = self.data('PUT', '/api/usb/mode', {'mode': 'ecm'}, expect=202)
        self.assertEqual(back, {'status': 'scheduled', 'mode': 'ecm', 'delay_ms': 1000})
        self.assertEqual(self.status()['active_mode'], 'ncm')
        self.wait_for_mode('ecm')

    def test_debug_and_unknown_modes_are_rejected(self):
        for mode in ('debug', 'diag', 'mass_storage'):
            self.assertEqual(self.error('PUT', '/api/usb/mode', {'mode': mode}), 'unsupported USB mode')
        self.assertEqual(self.error('PUT', '/api/usb/mode', {}), 'mode is required')
        self.assertEqual(self.status()['active_mode'], 'ecm')

    def test_rndis_applies_immediately(self):
        self.data('PUT', '/api/usb/mode', {'mode': 'rndis'})
        status = self.status()
        self.assertEqual(status['active_mode'], 'rndis')
        self.assertTrue(status['interfaces']['rndis0'])
        self.assertFalse(status['interfaces']['ecm0'])

    def test_default_mode(self):
        self.error('PUT', '/api/usb/default', {'mode': 'ncm'})
        self.error('PUT', '/api/usb/default', {'mode': 'rndis'})
        result = self.data('PUT', '/api/usb/default', {'mode': 'ncm', 'confirm_experimental': True})
        self.assertEqual(result, {'default_mode': 'ncm', 'ncm_persist_on_boot': True})
        self.assertTrue(self.status()['ncm_persist_on_boot'])


class SmsReadBack(MockCase):
    def messages(self):
        rows = self.data('POST', '/api/sms/list', {'page': 0, 'per_page': 500})['messages']
        return {m['id']: m for m in rows}

    @staticmethod
    def decode(hex_text):
        return ''.join(chr(int(hex_text[i:i + 4], 16)) for i in range(0, len(hex_text), 4))

    def test_list_is_newest_first_with_ucs2_content(self):
        rows = self.data('POST', '/api/sms/list', {'page': 0, 'per_page': 500})['messages']
        self.assertEqual([m['id'] for m in rows], sorted((m['id'] for m in rows), reverse=True))
        self.assertEqual(self.decode(rows[-1]['content']), 'On my way, should be there in 20.')
        page = self.data('POST', '/api/sms/list', {'page': 1, 'per_page': 3})['messages']
        self.assertEqual(len(page), 1)
        self.error('POST', '/api/sms/list', {'per_page': 501})

    def test_read_flips_unread_to_read(self):
        self.assertEqual(self.messages()[3720]['tag'], 1)
        self.data('POST', '/api/sms/read', {'ids': [3720, 3719]})
        messages = self.messages()
        self.assertEqual((messages[3720]['tag'], messages[3719]['tag']), (0, 0))
        self.assertEqual(messages[3718]['tag'], 2)  # sent stays sent

    def test_delete_removes_messages(self):
        result = self.data('POST', '/api/sms/delete', {'ids': [3721, 3718]})
        self.assertEqual(result, {'result': 'success'})
        self.assertEqual(sorted(self.messages()), [3719, 3720])

    def test_send_appends_a_sent_message(self):
        before = set(self.messages())
        self.data('POST', '/api/sms/send', {'number': '+61491570158', 'message': 'hello from the mock'})
        new = [m for i, m in self.messages().items() if i not in before]
        self.assertEqual(len(new), 1)
        self.assertEqual((new[0]['number'], new[0]['tag']), ('+61491570158', 2))
        self.assertEqual(self.decode(new[0]['content']), 'hello from the mock')
        self.assertGreater(new[0]['id'], max(before))

    def test_validation(self):
        self.error('POST', '/api/sms/send', {'number': '123; reboot', 'message': 'x'})
        self.error('POST', '/api/sms/send', {'number': '123', 'message': ''})
        self.error('POST', '/api/sms/send', {'number': '123', 'message': 'x' * 161})
        self.error('POST', '/api/sms/send', {'number': '123'})
        self.error('POST', '/api/sms/delete', {'ids': []})
        self.error('POST', '/api/sms/delete', {'ids': [0]})
        self.error('POST', '/api/sms/read', {'ids': 'all'})
        self.assertEqual(len(self.messages()), 4)


class ChargeControl(MockCase):
    def test_manual_stop_is_inverted_in_the_charger_and_reported_in_full(self):
        state = self.data('PUT', '/api/device/charge-control', {'charging_stopped': True})
        for key in ('available', 'battery_available', 'charger_available', 'charging_stopped', 'battery_status',
                    'capacity', 'charge_limit_enabled', 'charge_limit', 'hysteresis', 'manual_override', 'last_error'):
            self.assertIn(key, state)
        self.assertEqual((state['charging_stopped'], state['manual_override'], state['battery_status']),
                         (True, True, 'Not charging'))
        self.assertEqual(self.data('GET', '/api/device/charger')['direct_power_supply_mode'], 'enable')
        self.assertEqual(self.data('GET', '/api/dashboard')['battery']['status'], 'Not charging')
        self.assertEqual(self.data('GET', '/api/device/charge-control'), state)
        resumed = self.data('PUT', '/api/device/charge-control', {'charging_stopped': False})
        self.assertEqual((resumed['charging_stopped'], resumed['manual_override']), (False, False))
        self.assertEqual(self.data('GET', '/api/device/charger')['direct_power_supply_mode'], 'disable')

    def test_limit_below_capacity_stops_charging_and_clears_the_override(self):
        self.data('PUT', '/api/device/charge-control', {'charging_stopped': True})
        state = self.data('PUT', '/api/device/charge-control',
                          {'charge_limit_enabled': True, 'charge_limit': 60, 'hysteresis': 5})
        self.assertEqual((state['charge_limit'], state['charge_limit_enabled'], state['manual_override']),
                         (60, True, False))
        self.assertTrue(state['charging_stopped'])  # capacity 78 >= limit 60

    def test_limit_validation_mirrors_the_enforcer(self):
        before = self.data('GET', '/api/device/charge-control')
        for body in ({'charge_limit': 49}, {'charge_limit': 101}, {'hysteresis': 0}, {'hysteresis': 21}):
            self.assertIn('limit must be 50-100', self.error('PUT', '/api/device/charge-control', body, expect=503))
        self.assertIn('at least one', self.error('PUT', '/api/device/charge-control', {}, expect=503))
        for body in ({'charge_limit': 336}, {'charging_stopped': 'true'}, {'unknown': 1}):
            self.error('PUT', '/api/device/charge-control', body)
        after = self.data('GET', '/api/device/charge-control')
        self.assertEqual({k: v for k, v in after.items() if k != 'last_error'},
                         {k: v for k, v in before.items() if k != 'last_error'})


class RecordingAndErrors(MockCase):
    def test_mutations_are_recorded_and_reads_are_not(self):
        self.data('GET', '/api/wifi/status')
        self.data('POST', '/api/sms/list', {})
        self.data('PUT', '/api/wifi/settings', {'txpower_2g': 50})
        self.error('PUT', '/api/wifi/settings', {'txpower_2g': 0})
        self.assertEqual([(r['method'], r['path'], r['body'], r['status']) for r in self.recorded()],
                         [('PUT', '/api/wifi/settings', {'txpower_2g': 50}, 200),
                          ('PUT', '/api/wifi/settings', {'txpower_2g': 0}, 400)])

    def test_reset_restores_state_and_clears_the_log(self):
        self.data('PUT', '/api/wifi/settings', {'txpower_2g': 50})
        self.call('POST', '/__mock/reset')
        self.assertEqual(self.data('GET', '/api/wifi/status')['txpower_2g'], '80')
        self.assertEqual(self.recorded(), [])

    def test_destructive_routes_need_confirmation(self):
        for path in ('/api/device/reboot', '/api/device/shutdown'):
            self.assertIn('X-Confirm', self.error('POST', path, {}))
            self.data('POST', path, {}, headers={'X-Confirm': 'true'})
        self.error('POST', '/api/system/kill-bloat', {'all': True})
        killed = self.data('POST', '/api/system/kill-bloat', {'all': True}, headers={'X-Confirm': 'true'})
        self.assertEqual(killed['bloat_count'] if 'bloat_count' in killed else len(killed['killed']), 2)

    def test_ttl_round_trip(self):
        self.error('PUT', '/api/ttl/set', {'ttl': 0})
        self.error('PUT', '/api/ttl/set', {'ttl': 256})
        self.assertEqual(self.data('PUT', '/api/ttl/set', {'ttl': 65}), {'ttl': 65, 'ipv4': True, 'ipv6': True})
        self.assertEqual(self.data('GET', '/api/ttl/status'), {'active': True, 'ipv6_active': True, 'ttl_value': 65})
        status, envelope = self.call('DELETE', '/api/ttl/clear')
        self.assertEqual((status, envelope), (200, {'ok': True}))
        self.assertFalse(self.data('GET', '/api/ttl/status')['active'])

    def test_at_console_allowlist(self):
        self.assertIn('response', self.data('POST', '/api/at/send', {'command': 'AT+CSQ'}))
        for command in ('AT+CFUN=1', 'AT^RESET', 'AT+FOO'):
            self.assertEqual(self.error('POST', '/api/at/send', {'command': command}, expect=403)[:7], 'command')

    def test_logger_lifecycle(self):
        self.assertFalse(self.data('GET', '/api/logger/signal/status')['running'])
        self.data('POST', '/api/logger/signal/start', {'duration_secs': 60, 'interval_secs': 3})
        self.assertTrue(self.data('GET', '/api/logger/signal/status')['running'])
        self.error('POST', '/api/logger/signal/start', {'duration_secs': 60, 'interval_secs': 3}, expect=409)
        self.call('POST', '/api/logger/signal/stop', {})
        self.assertFalse(self.data('GET', '/api/logger/signal/status')['running'])
        self.error('POST', '/api/logger/connection/start', {'duration_secs': 0})
        status, content_type, body = self.raw('GET', '/api/logger/signal/download')
        self.assertEqual(status, 200)
        self.assertIn('text/csv', content_type)
        self.assertTrue(body.startswith(b'timestamp,datetime,network_type'))


    # -- eSIM (agent/src/esim.rs shapes) ---------------------------------------

    CONFIRM = {'X-Confirm': 'true'}

    def esim(self, mode='card'):
        mock.CONFIG['esim_settle_s'] = 0.3
        self.call('POST', '/__mock/reset', {'scenario': 'SA', 'esim': mode})
        return self.data('GET', '/api/esim/profiles')

    def handle(self, view, masked):
        return next(p['handle'] for p in view['profiles'] if p['iccid_masked'] == masked)

    def switch(self, view, path, masked, op_id, expect=202):
        body = {'handle': self.handle(view, masked), 'revision': view['revision'], 'operation_id': op_id}
        return self.data('POST', path, body, expect=expect, headers=self.CONFIRM)

    def settle(self):
        for _ in range(60):
            view = self.data('GET', '/api/esim/profiles')
            if view['operation']['resolved']:
                return view
            time.sleep(0.05)
        self.fail('operation never resolved')

    def test_esim_view_masks_identifiers_and_orders_the_active_profile_first(self):
        view = self.esim()
        self.assertEqual(view['status'], 'ready')
        self.assertEqual(view['profiles'][0]['state'], 'enabled')
        fiji = view['profiles'][1]
        self.assertEqual((fiji['display_name'], fiji['name']), ('Vodafone Fiji', None))
        self.assertEqual(fiji['warnings'], ['manual_apn', 'roaming_off'])
        self.assertNotRegex(json.dumps(view), r'\d{18,}')
        self.assertEqual(self.data('GET', '/api/dashboard')['esim'], {'status': 'ready', 'available': True})

    def test_esim_activation_reloads_then_succeeds_and_dedupes(self):
        view = self.esim()
        started = self.switch(view, '/api/esim/enable', '…0002', 'op-mock-0001')
        self.assertFalse(started['actions']['enabled'])
        again = self.switch(view, '/api/esim/enable', '…0002', 'op-mock-0001', expect=200)
        self.assertEqual(again['operation']['id'], 'op-mock-0001')
        done = self.settle()
        self.assertEqual(done['operation']['state'], 'succeeded')
        self.assertEqual(done['profiles'][0]['iccid_masked'], '…0002')
        self.assertEqual(done['revision'], view['revision'] + 1)
        enables = [r for r in self.data('GET', '/__mock/requests')['requests'] if r['path'] == '/api/esim/enable']
        self.assertEqual(len(enables), 2, 'both requests recorded; one change applied')

    def test_esim_refusals_match_the_agent(self):
        view = self.esim()
        body = {'handle': self.handle(view, '…0002'), 'revision': view['revision'], 'operation_id': 'op-mock-0002'}
        status, _ = self.call('POST', '/api/esim/enable', body)
        self.assertEqual(status, 400, 'X-Confirm is required')
        stale = dict(body, revision=view['revision'] + 1)
        self.assertEqual(self.call('POST', '/api/esim/enable', stale, self.CONFIRM)[0], 409)
        test_class = dict(body, handle=self.handle(view, '…0003'), operation_id='op-mock-0003')
        self.assertEqual(self.call('POST', '/api/esim/enable', test_class, self.CONFIRM)[0], 409)
        self.data('POST', '/api/esim/lock', {'handle': body['handle'], 'locked': True})
        self.assertEqual(self.call('POST', '/api/esim/enable', body, self.CONFIRM)[0], 423)
        pin = self.data('POST', '/api/auth/login', {'pin': '123456'})['token']
        status, _ = self.call('POST', '/api/esim/lock', {'handle': body['handle'], 'locked': False},
                              {'Authorization': f'Bearer {pin}'})
        self.assertEqual(status, 403)

    def test_esim_busy_and_unknown_outcomes(self):
        for mode, state in (('busy', 'busy'), ('unknown', 'unknown')):
            view = self.esim(mode)
            self.switch(view, '/api/esim/disable', '…0001', f'op-mock-{mode}')
            done = self.settle()
            self.assertEqual(done['operation']['state'], state)
            self.assertEqual(done['profiles'][0]['state'], 'enabled', 'nothing changed in the mock')

    def test_esim_rename_validates_bytes_and_hidden_characters(self):
        view = self.esim()
        h = self.handle(view, '…0001')
        base = {'handle': h, 'revision': view['revision']}
        self.error('POST', '/api/esim/nickname', dict(base, operation_id='op-name-001', nickname='é' * 33))
        self.error('POST', '/api/esim/nickname', dict(base, operation_id='op-name-002', nickname='a\u202eb'))
        renamed = self.data('POST', '/api/esim/nickname', dict(base, operation_id='op-name-003', nickname='Home'))
        self.assertEqual(renamed['profiles'][0]['display_name'], 'Home')

    def test_esim_whole_card_states(self):
        for mode, status in (('unsupported', 'unsupported'), ('helper-missing', 'helper_missing')):
            view = self.esim(mode)
            self.assertEqual((view['status'], view['available'], view['profiles']), (status, False, []))
            self.assertFalse(view['actions']['enabled'])
        self.assertFalse(any(p['state'] == 'enabled' for p in self.esim('no-active')['profiles']))
        stale = self.esim('stale')
        self.assertTrue(stale['freshness']['stale'])
        self.assertEqual(stale['freshness']['error']['kind'], 'management_unavailable')


    # -- eSIM phase 2: add and delete profiles --------------------------------

    CODE = 'LPA:1$smdp.example.com$TEST-MATCHING-ID'

    def download_until(self, state):
        for _ in range(80):
            view = self.data('GET', '/api/esim/profiles')
            if view['operation']['state'] == state:
                return view
            time.sleep(0.05)
        self.fail(f'download never reached {state}: {view["operation"]}')

    def start_download(self, op_id='op-dl-0001', mode='card'):
        self.esim(mode)
        mock.CONFIG['esim_step_s'] = 0.05
        return self.data('POST', '/api/esim/download', {'activation_code': self.CODE, 'operation_id': op_id},
                         expect=202)

    def test_esim_download_previews_installs_and_names_the_new_profile(self):
        started = self.start_download()
        self.assertEqual(started['operation']['kind'], 'download')
        self.assertIsNone(started['operation']['target'])
        self.assertTrue(started['operation']['can_cancel'])
        preview = self.download_until('awaiting_confirmation')
        self.assertEqual(preview['operation']['preview']['provider'], 'Test Mobile')
        self.assertGreater(preview['operation']['preview_expires_in_ms'], 0)
        self.assertNotIn('TEST-MATCHING-ID', json.dumps(preview), 'codes are never echoed')
        self.data('POST', '/api/esim/download/confirm',
                  {'operation_id': 'op-dl-0001', 'install': True, 'nickname': 'Holiday'})
        self.assertEqual(self.error('POST', '/api/esim/download/cancel', {'operation_id': 'op-dl-0001'},
                                    expect=409), "the profile is already being installed; it can't be cancelled")
        done = self.download_until('succeeded')
        new = next(p for p in done['profiles'] if p['iccid_masked'] == '…0004')
        self.assertEqual((new['display_name'], new['state']), ('Holiday', 'disabled'))
        self.assertEqual(done['operation']['target'], new['handle'])
        recorded = [r for r in self.data('GET', '/__mock/requests')['requests'] if r['path'] == '/api/esim/download']
        self.assertEqual(recorded[0]['body']['activation_code'], '<redacted>')

    def test_esim_download_cancel_decline_and_refusals(self):
        self.start_download('op-dl-0002')
        cancelled = self.data('POST', '/api/esim/download/cancel', {'operation_id': 'op-dl-0002'})
        self.assertEqual((cancelled['operation']['state'], cancelled['operation']['message']), ('cancelled', 'cancelled'))
        self.start_download('op-dl-0003')
        self.download_until('awaiting_confirmation')
        declined = self.data('POST', '/api/esim/download/confirm', {'operation_id': 'op-dl-0003', 'install': False})
        self.assertEqual(declined['operation']['message'], 'cancelled: declined')
        self.start_download('op-dl-0004', mode='download-refused')
        self.assertIn('matchingID', self.download_until('failed')['operation']['message'])
        self.esim('no-data')
        self.assertIn('mobile data', self.error('POST', '/api/esim/download',
                                                {'activation_code': self.CODE, 'operation_id': 'op-dl-0005'}, expect=409))
        self.esim()
        for code in ('', 'LPA:2$smdp.example.com$X', 'LPA:1$10.0.0.1$X', 'LPA:1$smdp.example.com$X$$1'):
            message = self.error('POST', '/api/esim/download', {'activation_code': code, 'operation_id': 'op-dl-0006'})
            self.assertNotIn('smdp.example.com$', message)
        pin = self.data('POST', '/api/auth/login', {'pin': '123456'})['token']
        for path in ('/api/esim/download', '/api/esim/download/confirm', '/api/esim/download/cancel',
                     '/api/esim/delete', '/api/esim/receipts/send'):
            status, _ = self.call('POST', path, {}, {'Authorization': f'Bearer {pin}', **self.CONFIRM})
            self.assertEqual(status, 403, path)

    def test_esim_delete_only_inactive_unlocked_profiles(self):
        view = self.esim()
        active = {'handle': self.handle(view, '…0001'), 'revision': view['revision'], 'operation_id': 'op-del-0001'}
        self.assertEqual(self.call('POST', '/api/esim/delete', active)[0], 400, 'X-Confirm is required')
        self.assertEqual(self.call('POST', '/api/esim/delete', active, self.CONFIRM)[0], 409)
        fiji = dict(active, handle=self.handle(view, '…0002'), operation_id='op-del-0002')
        self.data('POST', '/api/esim/lock', {'handle': fiji['handle'], 'locked': True})
        self.assertEqual(self.call('POST', '/api/esim/delete', fiji, self.CONFIRM)[0], 423)
        test = dict(active, handle=self.handle(view, '…0003'), operation_id='op-del-0003')
        self.data('POST', '/api/esim/delete', test, expect=202, headers=self.CONFIRM)
        done = self.settle()
        self.assertEqual(done['operation']['state'], 'succeeded')
        self.assertNotIn('…0003', [p['iccid_masked'] for p in done['profiles']])

    def test_esim_send_receipts_runs_and_empties_the_card(self):
        view = self.esim()
        self.assertEqual(view['card']['pending_receipts'], 5)
        started = self.data('POST', '/api/esim/receipts/send', {}, expect=202)
        self.assertTrue(started['receipts']['sending'])
        for _ in range(60):
            view = self.data('GET', '/api/esim/profiles')
            if not view['receipts']['sending']:
                break
            time.sleep(0.05)
        self.assertEqual((view['card']['pending_receipts'], view['receipts']['pending']), (0, 0))
        self.data('POST', '/api/esim/receipts/send', {}, expect=200)


if __name__ == '__main__':
    unittest.main()
