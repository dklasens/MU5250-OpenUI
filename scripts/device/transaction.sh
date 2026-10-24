#!/bin/sh
# Durable deployment snapshots. This is invoked on demand, never at boot.
# U60_TEST_ROOT is used only by host-side failure-injection tests.
set -eu
umask 077
root=${U60_TEST_ROOT:-}
base="$root/data/local/tmp/open-u60-transactions"
action=${1:?action required}
id=${2:?transaction id required}
identity=${3:?identity fingerprint required}
case "$id" in ''|*[!a-zA-Z0-9-]*) exit 64 ;; esac
case "$identity" in ''|*[!a-f0-9]*) exit 64 ;; esac
transaction="$base/$id"
targets='data/zte-agent
data/zte-esim
data/open-u60-manifest.json
data/www
data/www.current
data/local/tmp/start_zte_agent.sh
data/local/tmp/start_dashboard.sh
data/local/tmp/start_dropbear.sh
data/bin/dropbear
data/bin/dbclient
data/bin/dropbearkey
data/bin/dashboard-uhttpd
etc/rc.local
etc/dropbear
data/dropbear
etc/config/uhttpd'

case "$action" in
begin)
    mkdir -p "$base"
    if ! mkdir "$base/lock" 2>/dev/null; then
        echo 'An unfinished deployment exists. Recover it before starting another installation.' >&2
        exit 73
    fi
    printf '%s\n' "$id" > "$base/lock/owner"
    # A failed snapshot must never be advertised as a usable recovery point.
    if ! mkdir "$transaction"; then rm -f "$base/lock/owner"; rmdir "$base/lock"; exit 73; fi
    trap 'rm -rf "$transaction"; rm -f "$base/lock/owner"; rmdir "$base/lock" 2>/dev/null || true' EXIT HUP INT TERM
    mkdir "$transaction/before" "$transaction/present"
    printf '%s\n' "$identity" > "$transaction/identity"
    for target in $targets; do
        name=$(printf '%s' "$target" | tr / _)
        if [ -e "$root/$target" ] || [ -L "$root/$target" ]; then
            cp -a "$root/$target" "$transaction/before/$name"
            : > "$transaction/present/$name"
        fi
    done
    sync
    printf '%s\n' pending > "$transaction/state"
    sync
    printf '%s\n' "$id" > "$base/active.new"
    mv "$base/active.new" "$base/active"
    sync
    trap - EXIT HUP INT TERM
    ;;
discard-incomplete)
    # Preparation never touches live files. An incomplete snapshot must never
    # be used for restoration, because missing entries are not known absences.
    test ! -e "$base/active"
    test ! -e "$transaction/state"
    test "$(cat "$base/lock/owner")" = "$id"
    test "$(cat "$transaction/identity")" = "$identity"
    rm -rf "$transaction"
    rm -f "$base/lock/owner"
    rmdir "$base/lock"
    sync
    ;;
restore|complete)
    test "$(cat "$transaction/identity")" = "$identity"
    case "$(cat "$transaction/state")" in pending|committed|restored) ;; *) exit 65 ;; esac
    if [ "$action" = restore ] && [ ! -e "$base/active" ]; then
        if [ -d "$base/lock" ]; then test "$(cat "$base/lock/owner")" = "$id"
        else mkdir "$base/lock"; printf '%s\n' "$id" > "$base/lock/owner"; fi
        printf '%s\n' "$id" > "$base/active"
    fi
    test "$(cat "$base/active")" = "$id"
    if [ "$action" = complete ]; then test "$(cat "$transaction/state")" = pending; fi
    if [ "$action" = restore ]; then
        reload_stock_http=false
        if [ -z "$root" ] && [ -f "$transaction/present/etc_config_uhttpd" ]; then
            previous=$(sha256sum "$transaction/before/etc_config_uhttpd" | awk '{print $1}')
            current=$(sha256sum /etc/config/uhttpd 2>/dev/null | awk '{print $1}')
            [ "$previous" = "$current" ] || reload_stock_http=true
        fi
        if [ -z "$root" ]; then
            killall zte-agent 2>/dev/null || true
            sh /data/local/tmp/stop_open_u60_listener.sh dropbear 08AE
            sh /data/local/tmp/stop_open_u60_listener.sh dashboard-uhttpd 1F90
        fi
        # Never destroy a snapshot while restoring. Interrupted recovery can be
        # retried using the same transaction and verified device identity.
        for target in $targets; do
            name=$(printf '%s' "$target" | tr / _)
            destination="$root/$target"
            staged="$destination.restore-$id"
            if [ -f "$transaction/present/$name" ]; then
                rm -rf "$staged"
                cp -a "$transaction/before/$name" "$staged"
                case "$target" in etc/rc.local|data/local/tmp/start_*.sh) sh -n "$staged" ;; esac
                if [ -d "$destination" ] && [ ! -L "$destination" ]; then
                    # Keep the failed directory until its replacement is ready.
                    failed="$transaction/failed-$name"
                    if [ ! -e "$failed" ]; then mv "$destination" "$failed"; else rm -rf "$destination"; fi
                fi
                mv -Tf "$staged" "$destination"
            else
                rm -rf "$destination"
            fi
        done
        if [ -z "$root" ]; then
            # A legacy dashboard UCI instance may have been removed during
            # hardening. Restore the stock listener's runtime configuration too.
            if [ "$reload_stock_http" = true ]; then
                /etc/init.d/uhttpd restart
            fi
            for service in dropbear zte_agent dashboard; do
                startup="/data/local/tmp/start_${service}.sh"
                if [ -f "$startup" ]; then sh "$startup"; fi
            done
        fi
        printf '%s\n' restored > "$transaction/state"
    else
        printf '%s\n' committed > "$transaction/state"
    fi
    sync
    rm -f "$base/active"
    rm -f "$base/lock/owner"
    rmdir "$base/lock"
    sync
    ;;
*) exit 64 ;;
esac
