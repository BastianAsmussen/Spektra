#!/usr/bin/env bash

col() {
  local row name type key
  for row in "$@"; do
    IFS='|' read -r name type key <<< "$row"
    case "$key" in
      PK)  printf '    "%s %s": "" {constraint: primary_key}\n' "$name" "$type" ;;
      FK)  printf '    "%s %s": "" {constraint: foreign_key}\n' "$name" "$type" ;;
      UNQ) printf '    "%s %s": "" {constraint: unique}\n'      "$name" "$type" ;;
      *)   printf '    "%s %s": ""\n'                           "$name" "$type" ;;
    esac
  done
}

table() {
  local id="$1"; shift
  printf '  %s: {\n    shape: sql_table\n' "$id"
  col "$@"
  printf '  }\n'
}

cat <<EOF
#
# The uniform audit columns (created_at, updated_at) are left out to keep the

vars: {
  d2-config: {
    layout-engine: elk
    theme-id: 0
  }
}

direction: right

adgang: "Adgang" {
$(table roles "id|bigserial|PK" "name|varchar(32)|UNQ" "description|text|")
$(table users "id|bigserial|PK" "email|varchar(320)|UNQ" "password_hash|text|" "full_name|varchar(100)|" "role_id|bigint|FK" "deactivated|boolean|")
$(table sessions "id|bigserial|PK" "token|varchar(255)|UNQ" "user_id|bigint|FK" "expires_at|timestamp|" "last_used_at|timestamp|")

  users."role_id bigint" -> roles."id bigserial"
  sessions."user_id bigint" -> users."id bigserial"
}

flaade: "Flåde" {
$(table nodes "id|bigserial|PK" "external_identity|varchar(64)|UNQ" "name|varchar(100)|" "latitude|float8|" "longitude|float8|" "hardware|jsonb|" "capabilities|jsonb|" "suspended|boolean|" "report_interval_seconds|int|" "channel_plan_version|bigint|" "owner_id|bigint|FK" "last_seen_at|timestamp|")
$(table node_credentials "id|bigserial|PK" "node_id|bigint|FK" "token|varchar(255)|UNQ" "expires_at|timestamp|" "revoked_at|timestamp|")
$(table channels "id|bigserial|PK" "name|varchar(100)|" "frequency_hz|bigint|UNQ" "modulation|modulation|UNQ")
$(table node_channels "id|bigserial|PK" "node_id|bigint|FK" "channel_id|bigint|FK" "bandwidth_hz|integer|")

  node_credentials."node_id bigint" -> nodes."id bigserial"
  node_channels."node_id bigint" -> nodes."id bigserial"
  node_channels."channel_id bigint" -> channels."id bigserial"
}

maaledata: "Måledata" {
$(table measurements "id|bigserial|PK" "window_start|timestamp|PK" "node_id|bigint|FK" "channel_id|bigint|FK" "metric|metric|" "window_end|timestamp|" "min|float8|" "max|float8|" "mean|float8|" "median|float8|" "stddev|float8|" "p95|float8|" "sample_count|bigint|")
$(table rollups "id|bigserial|PK" "node_id|bigint|FK" "channel_id|bigint|FK" "metric|metric|" "resolution|rollup_resolution|" "bucket_start|timestamp|" "bucket_end|timestamp|" "min|float8|" "max|float8|" "mean|float8|" "median|float8|" "stddev|float8|" "sample_count|bigint|")
$(table node_health "id|bigserial|PK" "node_id|bigint|FK" "measured_at|timestamp|UNQ" "uptime_seconds|float8|" "load_1m|float8|" "load_5m|float8|" "load_15m|float8|" "cpu_temperature_celsius|float8|" "clock_offset_seconds|float8|")
}

haendelser: "Hændelser" {
$(table alarms "id|bigserial|PK" "node_id|bigint|FK" "channel_id|bigint|FK" "metric|metric|" "state|alarm_state|" "explanation|jsonb|" "raised_at|timestamp|" "closed_at|timestamp|")
$(table alarm_events "id|bigserial|PK" "alarm_id|bigint|FK" "from_state|alarm_state|" "to_state|alarm_state|" "changed_by_user_id|bigint|FK" "reason|text|")
$(table work_orders "id|bigserial|PK" "alarm_id|bigint|FK" "technician_user_id|bigint|FK" "station_name|text|" "status|work_order_status|" "fault_present|boolean|" "cause|text|" "action_taken|text|" "completed_at|timestamp|")

  alarm_events."alarm_id bigint" -> alarms."id bigserial"
  work_orders."alarm_id bigint" -> alarms."id bigserial"
}

flaade.nodes."owner_id bigint" -> adgang.users."id bigserial"

maaledata.measurements."node_id bigint" -> flaade.nodes."id bigserial"
maaledata.measurements."channel_id bigint" -> flaade.channels."id bigserial"
maaledata.rollups."node_id bigint" -> flaade.nodes."id bigserial"
maaledata.rollups."channel_id bigint" -> flaade.channels."id bigserial"
maaledata.node_health."node_id bigint" -> flaade.nodes."id bigserial"

haendelser.alarms."node_id bigint" -> flaade.nodes."id bigserial"
haendelser.alarms."channel_id bigint" -> flaade.channels."id bigserial"
haendelser.alarm_events."changed_by_user_id bigint" -> adgang.users."id bigserial"
haendelser.work_orders."technician_user_id bigint" -> adgang.users."id bigserial"
EOF
