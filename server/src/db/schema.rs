// @generated automatically by Diesel CLI.

pub mod sql_types {
    #[derive(diesel::query_builder::QueryId, Clone, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "alarm_state"))]
    pub struct AlarmState;

    #[derive(diesel::query_builder::QueryId, Clone, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "metric"))]
    pub struct Metric;

    #[derive(diesel::query_builder::QueryId, Clone, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "modulation"))]
    pub struct Modulation;

    #[derive(diesel::query_builder::QueryId, Clone, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "rollup_resolution"))]
    pub struct RollupResolution;

    #[derive(diesel::query_builder::QueryId, Clone, diesel::sql_types::SqlType)]
    #[diesel(postgres_type(name = "work_order_status"))]
    pub struct WorkOrderStatus;
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::AlarmState;

    alarm_events (id) {
        id -> Int8,
        alarm_id -> Int8,
        from_state -> Nullable<AlarmState>,
        to_state -> AlarmState,
        changed_by_user_id -> Nullable<Int8>,
        reason -> Text,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::Metric;
    use super::sql_types::AlarmState;

    alarms (id) {
        id -> Int8,
        node_id -> Int8,
        channel_id -> Nullable<Int8>,
        metric -> Nullable<Metric>,
        state -> AlarmState,
        explanation -> Jsonb,
        raised_at -> Timestamp,
        updated_at -> Timestamp,
        closed_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::Modulation;

    channels (id) {
        id -> Int8,
        #[max_length = 100]
        name -> Varchar,
        frequency_hz -> Int8,
        modulation -> Modulation,
        created_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::Metric;

    measurements (id, window_start) {
        id -> Int8,
        node_id -> Int8,
        channel_id -> Int8,
        metric -> Metric,
        window_start -> Timestamp,
        window_end -> Timestamp,
        min -> Float8,
        max -> Float8,
        mean -> Float8,
        median -> Float8,
        stddev -> Float8,
        p95 -> Float8,
        sample_count -> Int8,
        created_at -> Timestamp,
    }
}

diesel::table! {
    node_channels (id) {
        id -> Int8,
        node_id -> Int8,
        channel_id -> Int8,
        bandwidth_hz -> Nullable<Int4>,
        created_at -> Timestamp,
    }
}

diesel::table! {
    node_credentials (id) {
        id -> Int8,
        node_id -> Int8,
        #[max_length = 255]
        token -> Varchar,
        created_at -> Timestamp,
        expires_at -> Nullable<Timestamp>,
        revoked_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    node_health (id) {
        id -> Int8,
        node_id -> Int8,
        measured_at -> Timestamp,
        uptime_seconds -> Float8,
        load_1m -> Float8,
        load_5m -> Float8,
        load_15m -> Float8,
        cpu_temperature_celsius -> Float8,
        clock_offset_seconds -> Float8,
        created_at -> Timestamp,
    }
}

diesel::table! {
    nodes (id) {
        id -> Int8,
        #[max_length = 64]
        external_identity -> Nullable<Varchar>,
        #[max_length = 100]
        name -> Varchar,
        latitude -> Nullable<Float8>,
        longitude -> Nullable<Float8>,
        hardware -> Jsonb,
        capabilities -> Jsonb,
        suspended -> Bool,
        channel_plan_version -> Int8,
        owner_id -> Nullable<Int8>,
        created_at -> Timestamp,
        last_seen_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    roles (id) {
        id -> Int8,
        #[max_length = 32]
        name -> Varchar,
        description -> Text,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::Metric;
    use super::sql_types::RollupResolution;

    rollups (id) {
        id -> Int8,
        node_id -> Int8,
        channel_id -> Int8,
        metric -> Metric,
        resolution -> RollupResolution,
        bucket_start -> Timestamp,
        bucket_end -> Timestamp,
        min -> Float8,
        max -> Float8,
        mean -> Float8,
        median -> Float8,
        stddev -> Float8,
        sample_count -> Int8,
        created_at -> Timestamp,
    }
}

diesel::table! {
    sessions (id) {
        id -> Int8,
        #[max_length = 255]
        token -> Varchar,
        user_id -> Int8,
        created_at -> Timestamp,
        expires_at -> Timestamp,
        last_used_at -> Nullable<Timestamp>,
    }
}

diesel::table! {
    users (id) {
        id -> Int8,
        #[max_length = 320]
        email -> Varchar,
        password_hash -> Text,
        #[max_length = 100]
        full_name -> Varchar,
        role_id -> Int8,
        deactivated -> Bool,
        created_at -> Timestamp,
        updated_at -> Timestamp,
    }
}

diesel::table! {
    use diesel::sql_types::*;
    use super::sql_types::WorkOrderStatus;

    work_orders (id) {
        id -> Int8,
        alarm_id -> Int8,
        technician_user_id -> Int8,
        station_name -> Text,
        status -> WorkOrderStatus,
        fault_present -> Nullable<Bool>,
        cause -> Nullable<Text>,
        action_taken -> Nullable<Text>,
        completed_at -> Nullable<Timestamp>,
        created_at -> Timestamp,
    }
}

diesel::joinable!(alarm_events -> alarms (alarm_id));
diesel::joinable!(alarm_events -> users (changed_by_user_id));
diesel::joinable!(alarms -> channels (channel_id));
diesel::joinable!(alarms -> nodes (node_id));
diesel::joinable!(measurements -> channels (channel_id));
diesel::joinable!(measurements -> nodes (node_id));
diesel::joinable!(node_channels -> channels (channel_id));
diesel::joinable!(node_channels -> nodes (node_id));
diesel::joinable!(node_credentials -> nodes (node_id));
diesel::joinable!(node_health -> nodes (node_id));
diesel::joinable!(nodes -> users (owner_id));
diesel::joinable!(rollups -> channels (channel_id));
diesel::joinable!(rollups -> nodes (node_id));
diesel::joinable!(sessions -> users (user_id));
diesel::joinable!(users -> roles (role_id));
diesel::joinable!(work_orders -> alarms (alarm_id));
diesel::joinable!(work_orders -> users (technician_user_id));

diesel::allow_tables_to_appear_in_same_query!(
    alarm_events,
    alarms,
    channels,
    measurements,
    node_channels,
    node_credentials,
    node_health,
    nodes,
    roles,
    rollups,
    sessions,
    users,
    work_orders,
);
